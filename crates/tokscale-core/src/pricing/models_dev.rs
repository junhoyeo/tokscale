use super::litellm::ModelPricing;
use super::{cache, fetch};
use serde::Deserialize;
use std::collections::HashMap;

const CACHE_FILENAME: &str = "pricing-models-dev.json";
const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const PER_MILLION: f64 = 1_000_000.0;

#[derive(Deserialize)]
struct Provider {
    #[serde(default)]
    models: HashMap<String, Model>,
}

#[derive(Deserialize)]
struct Model {
    id: Option<String>,
    cost: Option<ModelCost>,
}

#[derive(Deserialize)]
struct ModelCost {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    // Keep each raw entry isolated until mapping. Catalog providers can add a
    // new tier shape independently, and one such entry must not make serde
    // reject every model in the shared dataset.
    #[serde(default)]
    tiers: Vec<serde_json::Value>,
}

/// The rates of a tier entry. Which card the entry is comes from its `tier`
/// object, read separately by [`TierHeader`].
#[derive(Deserialize)]
struct ModelCostTier {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

#[derive(Deserialize)]
struct ContextTier {
    #[serde(rename = "type")]
    kind: String,
    size: u64,
}

/// Just the `tier` object of a tier entry, so an entry can be recognised as
/// the 100K context card even when its rates do not decode.
#[derive(Deserialize)]
struct TierHeader {
    tier: ContextTier,
}

/// Whether a raw tier entry advertises the `> 100,000` context card. models.dev
/// spells that boundary both as `size: 100000` and as `size: 100001`.
fn is_100k_context_tier(entry: &serde_json::Value) -> bool {
    serde_json::from_value::<TierHeader>(entry.clone()).is_ok_and(|header| {
        header.tier.kind == "context" && matches!(header.tier.size, 100_000 | 100_001)
    })
}

/// A 100K context card this crate can price: input and output are usable
/// rates, and any cache rate it states is usable too.
fn readable_100k_tier(entry: &serde_json::Value) -> Option<ModelCostTier> {
    let tier: ModelCostTier = serde_json::from_value(entry.clone()).ok()?;
    let stated_rate_is_usable =
        |rate: Option<f64>| rate.is_none_or(|rate| per_token(rate).is_some());
    (tier.input.and_then(per_token).is_some()
        && tier.output.and_then(per_token).is_some()
        && stated_rate_is_usable(tier.cache_read)
        && stated_rate_is_usable(tier.cache_write))
    .then_some(tier)
}

pub type PricingDataset = HashMap<String, ModelPricing>;

pub fn load_cached() -> Option<PricingDataset> {
    cache::load_cache(CACHE_FILENAME)
}

pub fn load_cached_any_age() -> Option<PricingDataset> {
    cache::load_cache_any_age(CACHE_FILENAME)
}

pub(crate) fn parse_dataset(content: &str) -> Result<PricingDataset, serde_json::Error> {
    let providers: HashMap<String, Provider> = serde_json::from_str(content)?;
    Ok(map_providers(providers))
}

pub async fn fetch() -> Result<PricingDataset, String> {
    fetch_inner(MODELS_DEV_URL, true).await
}

/// `use_disk_cache` governs both the read below and the write at the end. See
/// the same function in `litellm.rs` for why the write is gated on the caller's
/// flag rather than on tests remembering to redirect `TOKSCALE_CONFIG_DIR`.
async fn fetch_inner(url: &str, use_disk_cache: bool) -> Result<PricingDataset, String> {
    if use_disk_cache {
        if let Some(cached) = load_cached() {
            return Ok(cached);
        }
    }

    let client = fetch::pricing_client()?;
    let response = fetch::get_with_retry(&client, url, "models.dev").await?;
    let content = response.text().await.map_err(|error| error.to_string())?;
    let data = parse_dataset(&content)
        .map_err(|error| format!("models.dev JSON parse failed: {error}"))?;
    if data.is_empty() {
        return Err("models.dev returned no usable pricing rows".to_string());
    }
    if use_disk_cache {
        if let Err(e) = cache::save_cache(CACHE_FILENAME, &data) {
            eprintln!(
                "[tokscale] Warning: Failed to cache models.dev pricing at {}: {}",
                cache::get_cache_path(CACHE_FILENAME).display(),
                e
            );
        }
    }
    Ok(data)
}

fn map_providers(providers: HashMap<String, Provider>) -> PricingDataset {
    let mut result = HashMap::new();

    for (provider_id, provider) in providers {
        for (model_key, model) in provider.models {
            let model_id = model.id.as_deref().unwrap_or(&model_key);
            let Some(pricing) = model.cost.and_then(cost_to_pricing) else {
                continue;
            };
            result.insert(format!("{provider_id}/{model_id}").to_lowercase(), pricing);
        }
    }

    result
}

fn cost_to_pricing(cost: ModelCost) -> Option<ModelPricing> {
    let input = per_token(cost.input?)?;
    let output = per_token(cost.output?)?;
    // models.dev currently encodes the same `> 100,000` boundary both as
    // `size: 100000` and `size: 100001`, depending on the provider row. The
    // Anthropic-specific pricing path decides the actual exclusive boundary;
    // this mapping only preserves either spelling of the published rate card.
    //
    // Entries are decoded one at a time, so an unrelated or unknown tier is
    // ignored locally instead of rejecting the provider dataset.
    let advertised_100k: Vec<&serde_json::Value> = cost
        .tiers
        .iter()
        .filter(|entry| is_100k_context_tier(entry))
        .collect();
    let above_100k = advertised_100k
        .iter()
        .find_map(|entry| readable_100k_tier(entry));
    if !advertised_100k.is_empty() && above_100k.is_none() {
        // The row advertises a long-context card whose rates cannot be read.
        // Keeping its flat rates would price long requests at the base card,
        // the silent under-billing the 100K handling exists to prevent, so the
        // row is left out and other sources, or an incomplete result, apply.
        return None;
    }

    Some(ModelPricing {
        input_cost_per_token: Some(input),
        input_cost_per_token_above_100k_tokens: above_100k
            .as_ref()
            .and_then(|tier| tier.input)
            .and_then(per_token),
        output_cost_per_token: Some(output),
        output_cost_per_token_above_100k_tokens: above_100k
            .as_ref()
            .and_then(|tier| tier.output)
            .and_then(per_token),
        cache_read_input_token_cost: cost.cache_read.and_then(per_token),
        cache_read_input_token_cost_above_100k_tokens: above_100k
            .as_ref()
            .and_then(|tier| tier.cache_read)
            .and_then(per_token),
        cache_creation_input_token_cost: cost.cache_write.and_then(per_token),
        cache_creation_input_token_cost_above_100k_tokens: above_100k
            .as_ref()
            .and_then(|tier| tier.cache_write)
            .and_then(per_token),
        ..Default::default()
    })
}

fn per_token(value: f64) -> Option<f64> {
    value
        .is_finite()
        .then_some(value)
        .filter(|v| *v >= 0.0)
        .map(|v| v / PER_MILLION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use serial_test::serial;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::TempDir;

    fn retryable_status_server(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer);
                let response =
                    format!("{status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(response.as_bytes());
            }
        });

        url
    }

    fn response_server(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });

        url
    }

    #[test]
    fn parses_above_100k_context_tier_rates() {
        let data = parse_dataset(
            r#"{
                "anthropic": {
                    "models": {
                        "claude-haiku-5-5": {
                            "cost": {
                                "input": 0.1,
                                "output": 0.5,
                                "cache_read": 0.01,
                                "cache_write": 0.125,
                                "tiers": [{
                                    "input": 0.5,
                                    "output": 2.5,
                                    "cache_read": 0.05,
                                    "cache_write": 0.625,
                                    "tier": {"type": "context", "size": 100000}
                                }]
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        let pricing = data.get("anthropic/claude-haiku-5-5").unwrap();

        let assert_rate = |actual: Option<f64>, expected: f64| {
            let actual = actual.expect("the tier rate must be preserved");
            assert!((actual - expected).abs() < 1e-18, "rate was {actual}");
        };
        assert_rate(pricing.input_cost_per_token_above_100k_tokens, 5e-7);
        assert_rate(pricing.output_cost_per_token_above_100k_tokens, 2.5e-6);
        assert_rate(pricing.cache_read_input_token_cost_above_100k_tokens, 5e-8);
        assert_rate(
            pricing.cache_creation_input_token_cost_above_100k_tokens,
            6.25e-7,
        );
        assert_eq!(
            pricing.cache_creation_input_token_cost_above_1hr_above_100k_tokens, None,
            "models.dev does not publish a separate 1-hour cache-write rate"
        );
    }

    #[test]
    fn malformed_tier_entries_do_not_drop_flat_rates_or_other_models() {
        let data = parse_dataset(
            r#"{
                "mixed-provider": {
                    "models": {
                        "malformed-tiers": {
                            "cost": {
                                "input": 1,
                                "output": 2,
                                "tiers": [
                                    null,
                                    {"input": 5, "output": 6},
                                    {"input": 5, "output": 6, "tier": "context"},
                                    {"input": 5, "output": 6, "tier": {"type": 1, "size": 100000}},
                                    {"input": 5, "output": 6, "tier": {"type": "context", "size": "100000"}},
                                    {"input": 5, "output": 6, "tier": {"type": "latency", "size": 100000}},
                                    {"input": 5, "output": 6, "tier": {"type": "context", "size": 128000}}
                                ]
                            }
                        },
                        "unreadable-100k-rate": {
                            "cost": {
                                "input": 1,
                                "output": 2,
                                "tiers": [
                                    {"input": "not-a-rate", "output": 6, "tier": {"type": "context", "size": 100000}}
                                ]
                            }
                        },
                        "100k-card-without-output": {
                            "cost": {
                                "input": 1,
                                "output": 2,
                                "tiers": [
                                    {"input": 5, "tier": {"type": "context", "size": 100001}}
                                ]
                            }
                        },
                        "one-readable-100k-card": {
                            "cost": {
                                "input": 1,
                                "output": 2,
                                "tiers": [
                                    {"input": "not-a-rate", "output": 6, "tier": {"type": "context", "size": 100000}},
                                    {"input": 5, "output": 6, "tier": {"type": "context", "size": 100000}}
                                ]
                            }
                        },
                        "claude-haiku-5-5": {
                            "cost": {
                                "input": 0.1,
                                "output": 0.5,
                                "cache_read": 0.01,
                                "cache_write": 0.125,
                                "tiers": [{
                                    "input": 0.5,
                                    "output": 2.5,
                                    "cache_read": 0.05,
                                    "cache_write": 0.625,
                                    "tier": {"type": "context", "size": 100001}
                                }]
                            }
                        },
                        "unrelated-model": {
                            "cost": {"input": 3, "output": 15}
                        }
                    }
                }
            }"#,
        )
        .expect("one malformed tier entry must not reject the provider dataset");

        let assert_rate = |actual: Option<f64>, expected: f64| {
            let actual = actual.expect("the expected rate must be preserved");
            assert!((actual - expected).abs() < 1e-18, "rate was {actual}");
        };

        let malformed = data
            .get("mixed-provider/malformed-tiers")
            .expect("the malformed tier row must keep its flat pricing");
        assert_rate(malformed.input_cost_per_token, 1e-6);
        assert_rate(malformed.output_cost_per_token, 2e-6);
        assert_eq!(malformed.input_cost_per_token_above_100k_tokens, None);
        assert_eq!(malformed.output_cost_per_token_above_100k_tokens, None);

        let haiku = data
            .get("mixed-provider/claude-haiku-5-5")
            .expect("a valid tiered model beside the malformed row must survive");
        assert_rate(haiku.input_cost_per_token_above_100k_tokens, 5e-7);
        assert_rate(haiku.output_cost_per_token_above_100k_tokens, 2.5e-6);
        assert_rate(haiku.cache_read_input_token_cost_above_100k_tokens, 5e-8);
        assert_rate(
            haiku.cache_creation_input_token_cost_above_100k_tokens,
            6.25e-7,
        );

        let unrelated = data
            .get("mixed-provider/unrelated-model")
            .expect("an unrelated flat model must survive the malformed tier");
        assert_rate(unrelated.input_cost_per_token, 3e-6);
        assert_rate(unrelated.output_cost_per_token, 15e-6);

        // A row that advertises the 100K card but whose card cannot be read
        // fails closed: it is left out rather than priced at its flat rates
        // for long requests.
        assert!(!data.contains_key("mixed-provider/unreadable-100k-rate"));
        assert!(!data.contains_key("mixed-provider/100k-card-without-output"));

        // One readable 100K card among unreadable ones is enough.
        let readable = data
            .get("mixed-provider/one-readable-100k-card")
            .expect("a readable 100K card keeps the row");
        assert_rate(readable.input_cost_per_token_above_100k_tokens, 5e-6);
        assert_rate(readable.output_cost_per_token_above_100k_tokens, 6e-6);
    }

    #[tokio::test]
    async fn fetch_returns_error_after_retryable_http_statuses() {
        let url = retryable_status_server("HTTP/1.1 503 Service Unavailable");

        let result = fetch_inner(&url, false).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn malformed_and_empty_datasets_are_fetch_errors() {
        let malformed = fetch_inner(&response_server("not json"), false).await;
        assert!(malformed
            .unwrap_err()
            .contains("models.dev JSON parse failed"));

        let empty = fetch_inner(&response_server("{}"), false).await;
        assert!(empty.unwrap_err().contains("no usable pricing rows"));
    }

    /// models.dev carried the same defect as LiteLLM: `use_disk_cache` gated
    /// only the read, so `malformed_and_empty_datasets_are_fetch_errors` above
    /// was one successful-parse fixture away from overwriting the developer's
    /// real `pricing-models-dev.json`. See the sibling test in `litellm.rs` for
    /// why the assertion redirects `TOKSCALE_CONFIG_DIR` rather than probing
    /// the developer's home directory.
    #[tokio::test]
    #[serial]
    async fn a_fetch_with_caching_disabled_writes_no_cache_file() {
        let temp_config = TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&["TOKSCALE_CONFIG_DIR"]);
        env.set("TOKSCALE_CONFIG_DIR", temp_config.path());

        let cache_path = cache::get_cache_path(CACHE_FILENAME);
        assert!(
            cache_path.starts_with(temp_config.path()),
            "the config-dir redirect must be in effect or this test proves nothing: {}",
            cache_path.display()
        );

        let url = response_server(
            r#"{"anthropic":{"models":{"claude":{"cost":{"input":3,"output":15}}}}}"#,
        );
        let data = fetch_inner(&url, false)
            .await
            .expect("the fixture serves one priced model");
        assert!(
            data.contains_key("anthropic/claude"),
            "the fetch itself must succeed"
        );
        assert_eq!(
            cache::get_cache_path(CACHE_FILENAME),
            cache_path,
            "the redirect moved while the fetch ran, so the assertion below would check a path the fetch never targeted"
        );

        assert!(
            !cache_path.exists(),
            "a fetch that opted out of the cache must not write it, but {} was created",
            cache_path.display()
        );
    }
}
