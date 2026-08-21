use super::litellm::ModelPricing;
use super::{cache, describe_error, fetch};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

const CACHE_FILENAME: &str = "pricing-openrouter.json";
/// Root of the OpenRouter REST API. Both requests this module makes — the
/// model list and the per-model endpoint lookup — are built from this one
/// value so a test can point the whole fetch at a local fixture server. The
/// per-model URL used to be hardcoded, which left the author-pricing leg
/// unreachable offline: any test that got far enough to exercise it made a
/// real request to openrouter.ai.
///
/// Do not reintroduce a literal URL for either request.
/// `a_fetch_with_caching_disabled_writes_no_cache_file` reaches the
/// author-pricing leg on purpose — that is the only path that populates
/// `result`, and therefore the only one that can reach the cache write the
/// test guards. Hardcoding that URL again would not fail the test; it would
/// make `cargo test` call openrouter.ai for real on every run.
const API_BASE: &str = "https://openrouter.ai/api/v1";
const MAX_CONCURRENT_REQUESTS: usize = 10;

/// Structs for `/api/v1/models` endpoint (list all models).

#[derive(Deserialize)]
struct ModelListPricing {
    prompt: String,
    completion: String,
}

#[derive(Deserialize)]
struct ModelListItem {
    id: String,
    pricing: Option<ModelListPricing>,
}

#[derive(Deserialize)]
struct ModelsListResponse {
    data: Vec<ModelListItem>,
}

/// Structs for `/api/v1/models/{id}/endpoints` endpoint (author pricing).

#[derive(Deserialize)]
struct EndpointPricing {
    prompt: String,
    completion: String,
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
}

#[derive(Deserialize)]
struct Endpoint {
    provider_name: String,
    pricing: EndpointPricing,
}

#[derive(Deserialize)]
struct EndpointData {
    #[allow(dead_code)]
    id: String,
    endpoints: Vec<Endpoint>,
}

#[derive(Deserialize)]
struct EndpointsResponse {
    data: EndpointData,
}

/// Model ID prefix to provider name mapping.
///
/// Translates model ID prefixes like `z-ai` to their corresponding
/// provider names in the endpoints API, such as `Z.AI`.
fn get_author_provider_name(model_id: &str) -> Option<&'static str> {
    let prefix = model_id.split('/').next()?;

    match prefix.to_lowercase().as_str() {
        "z-ai" => Some("Z.AI"),
        "x-ai" => Some("xAI"),
        "anthropic" => Some("Anthropic"),
        "openai" => Some("OpenAI"),
        "google" => Some("Google"),
        "meta-llama" => Some("Meta"),
        "mistralai" => Some("Mistral"),
        "deepseek" => Some("DeepSeek"),
        "qwen" => Some("Alibaba"),
        "cohere" => Some("Cohere"),
        "perplexity" => Some("Perplexity"),
        "moonshotai" => Some("Moonshot AI"),
        _ => None,
    }
}

pub fn load_cached() -> Option<HashMap<String, ModelPricing>> {
    cache::load_cache(CACHE_FILENAME)
}

pub fn load_cached_any_age() -> Option<HashMap<String, ModelPricing>> {
    cache::load_cache_any_age(CACHE_FILENAME)
}

fn parse_price(s: &str) -> Option<f64> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

async fn fetch_author_pricing(
    client: Arc<reqwest::Client>,
    api_base: Arc<String>,
    model_id: String,
    semaphore: Arc<Semaphore>,
    fallback_pricing: Option<ModelPricing>,
) -> Option<(String, ModelPricing)> {
    let _permit = semaphore.acquire().await.ok()?;

    let author_name = match get_author_provider_name(&model_id) {
        Some(name) => name,
        None => return fallback_pricing.map(|p| (model_id, p)),
    };

    let url = format!("{}/models/{}/endpoints", api_base, model_id);

    let response = match client
        .get(&url)
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return fallback_pricing.map(|p| (model_id, p));
        }
    };

    if !response.status().is_success() {
        return fallback_pricing.map(|p| (model_id, p));
    }

    let data: EndpointsResponse = match response.json().await {
        Ok(d) => d,
        Err(_) => {
            return fallback_pricing.map(|p| (model_id, p));
        }
    };

    match select_endpoint_pricing(&data.data.endpoints, author_name, fallback_pricing.as_ref()) {
        Some(pricing) => Some((model_id, pricing)),
        None => fallback_pricing.map(|p| (model_id, p)),
    }
}

fn endpoint_pricing(endpoint: &Endpoint) -> Option<ModelPricing> {
    Some(ModelPricing {
        input_cost_per_token: Some(parse_price(&endpoint.pricing.prompt)?),
        output_cost_per_token: Some(parse_price(&endpoint.pricing.completion)?),
        cache_read_input_token_cost: endpoint
            .pricing
            .input_cache_read
            .as_deref()
            .and_then(parse_price),
        cache_creation_input_token_cost: endpoint
            .pricing
            .input_cache_write
            .as_deref()
            .and_then(parse_price),
        ..Default::default()
    })
}

fn quotes_same_base_price(candidate: &ModelPricing, listed: &ModelPricing) -> bool {
    let same = |candidate: Option<f64>, listed: Option<f64>| match (candidate, listed) {
        (Some(candidate), Some(listed)) => (candidate - listed).abs() <= listed.abs() * 1e-9,
        _ => false,
    };

    same(candidate.input_cost_per_token, listed.input_cost_per_token)
        && same(
            candidate.output_cost_per_token,
            listed.output_cost_per_token,
        )
}

/// Pick the pricing row for a model from its OpenRouter endpoints.
///
/// The model author's own endpoint still wins, so `glm-4.7` keeps Z.AI's
/// price rather than a reseller's markup. When the model has no endpoint from
/// its author, the listed price is used exactly as before — but it is taken
/// from an endpoint that quotes that same base price, so the cache rates
/// OpenRouter publishes alongside it survive.
///
/// Discarding them is what broke `tokscale submit`: OpenRouter serves
/// `openai/gpt-5.2-codex` only from an `Azure` endpoint, so the author lookup
/// missed and the row lost the `input_cache_read` price it publishes.
/// Submission validation treats a populated bucket with no rate as
/// unpriceable, so every Codex session — which always carries cached tokens —
/// aborted the whole submission (#1013).
fn select_endpoint_pricing(
    endpoints: &[Endpoint],
    author_name: &str,
    listed: Option<&ModelPricing>,
) -> Option<ModelPricing> {
    if let Some(author) = endpoints.iter().find(|e| e.provider_name == author_name) {
        return endpoint_pricing(author);
    }

    let listed = listed?;
    let matching: Vec<ModelPricing> = endpoints
        .iter()
        .filter_map(endpoint_pricing)
        .filter(|pricing| quotes_same_base_price(pricing, listed))
        .collect();

    // Cache read and cache write are independent fields, so the endpoint
    // publishing the most of them is the one that leaves the fewest buckets
    // unpriceable. On an equal count, retain cache-read pricing: it is the
    // bucket required by Codex usage and must not be lost to an earlier
    // write-only endpoint.
    matching.into_iter().reduce(|best, candidate| {
        if published_cache_rates(&candidate) > published_cache_rates(&best)
            || (published_cache_rates(&candidate) == published_cache_rates(&best)
                && candidate.cache_read_input_token_cost.is_some()
                && best.cache_read_input_token_cost.is_none())
        {
            candidate
        } else {
            best
        }
    })
}

fn published_cache_rates(pricing: &ModelPricing) -> usize {
    usize::from(pricing.cache_read_input_token_cost.is_some())
        + usize::from(pricing.cache_creation_input_token_cost.is_some())
}

/// Fetch all models and get author pricing for each
pub async fn fetch_all_models() -> Result<HashMap<String, ModelPricing>, String> {
    fetch_all_models_from_api_base(API_BASE, true).await
}

async fn fetch_all_models_from_api_base(
    api_base: &str,
    use_disk_cache: bool,
) -> Result<HashMap<String, ModelPricing>, String> {
    if use_disk_cache {
        if let Some(cached) = load_cached() {
            return Ok(cached);
        }
    }

    let api_base = Arc::new(api_base.to_string());
    let models_url = format!("{api_base}/models");
    let client = Arc::new(fetch::pricing_client()?);
    let response = fetch::get_with_retry(&client, &models_url, "OpenRouter").await?;
    let data: ModelsListResponse = response.json().await.map_err(|error| {
        format!(
            "OpenRouter models JSON parse failed: {}",
            describe_error(&error)
        )
    })?;
    let models_with_fallback: Vec<(String, Option<ModelPricing>)> = data
        .data
        .into_iter()
        .map(|m| {
            let fallback = m.pricing.and_then(|p| {
                let input = parse_price(&p.prompt)?;
                let output = parse_price(&p.completion)?;
                Some(ModelPricing {
                    input_cost_per_token: Some(input),
                    output_cost_per_token: Some(output),
                    cache_read_input_token_cost: None,
                    cache_creation_input_token_cost: None,
                    ..Default::default()
                })
            });
            (m.id, fallback)
        })
        .collect();

    if models_with_fallback.is_empty() {
        return Err("OpenRouter returned no models".to_string());
    }

    let models_with_authors: Vec<(String, Option<ModelPricing>)> = models_with_fallback
        .into_iter()
        .filter(|(id, _)| get_author_provider_name(id).is_some())
        .collect();

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    let mut handles = Vec::with_capacity(models_with_authors.len());

    for (model_id, fallback) in models_with_authors {
        let client = Arc::clone(&client);
        let api_base = Arc::clone(&api_base);
        let sem = Arc::clone(&semaphore);

        let handle = tokio::spawn(async move {
            fetch_author_pricing(client, api_base, model_id, sem, fallback).await
        });

        handles.push(handle);
    }

    // Collect results
    let mut result = HashMap::new();

    for handle in handles {
        if let Ok(Some((model_id, pricing))) = handle.await {
            result.insert(model_id, pricing);
        }
    }

    // `use_disk_cache` gates the write as well as the read above. See
    // `litellm::fetch_inner` for why the caller's opt-out, not a
    // `TOKSCALE_CONFIG_DIR` redirect in each test, is what keeps a fixture
    // fetch out of the user's real cache.
    if use_disk_cache && !result.is_empty() {
        if let Err(e) = cache::save_cache(CACHE_FILENAME, &result) {
            eprintln!(
                "[tokscale] Warning: Failed to cache OpenRouter pricing at {}: {}",
                cache::get_cache_path(CACHE_FILENAME).display(),
                e
            );
        }
    }

    if result.is_empty() {
        return Err("OpenRouter returned no usable pricing rows".to_string());
    }

    Ok(result)
}

pub async fn fetch_all_mapped() -> Result<HashMap<String, ModelPricing>, String> {
    fetch_all_models().await
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

    fn response_server(status: &'static str, body: &'static str, requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for _ in 0..requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "{status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        url
    }

    fn endpoint(
        provider_name: &str,
        prompt: &str,
        completion: &str,
        input_cache_read: Option<&str>,
    ) -> Endpoint {
        endpoint_with_cache(provider_name, prompt, completion, input_cache_read, None)
    }

    fn endpoint_with_cache(
        provider_name: &str,
        prompt: &str,
        completion: &str,
        input_cache_read: Option<&str>,
        input_cache_write: Option<&str>,
    ) -> Endpoint {
        Endpoint {
            provider_name: provider_name.to_string(),
            pricing: EndpointPricing {
                prompt: prompt.to_string(),
                completion: completion.to_string(),
                input_cache_read: input_cache_read.map(str::to_string),
                input_cache_write: input_cache_write.map(str::to_string),
            },
        }
    }

    fn listed(input: f64, output: f64) -> ModelPricing {
        ModelPricing {
            input_cost_per_token: Some(input),
            output_cost_per_token: Some(output),
            ..Default::default()
        }
    }

    // Regression: #1013. OpenRouter serves `openai/gpt-5.2-codex` only from an
    // `Azure` endpoint, so the `OpenAI` author lookup missed and the row fell
    // back to the listed price with its cache rates dropped. Submission
    // validation then rejected every Codex session as unpriced.
    #[test]
    fn cache_rates_survive_when_the_model_has_no_author_endpoint() {
        let endpoints = vec![endpoint(
            "Azure",
            "0.00000175",
            "0.000014",
            Some("0.000000175"),
        )];

        let pricing =
            select_endpoint_pricing(&endpoints, "OpenAI", Some(&listed(1.75e-6, 1.4e-5))).unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(1.75e-6));
        assert_eq!(pricing.output_cost_per_token, Some(1.4e-5));
        assert_eq!(pricing.cache_read_input_token_cost, Some(1.75e-7));
    }

    // The author's own price stays authoritative, so a reseller endpoint can
    // never override it just because it publishes extra cache rates.
    #[test]
    fn author_endpoint_still_wins_over_other_providers() {
        let endpoints = vec![
            endpoint("Azure", "0.0000035", "0.0000175", Some("0.00000035")),
            endpoint("OpenAI", "0.0000002", "0.0000015", None),
        ];

        let pricing =
            select_endpoint_pricing(&endpoints, "OpenAI", Some(&listed(3.5e-6, 1.75e-5))).unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(2e-7));
        assert_eq!(pricing.output_cost_per_token, Some(1.5e-6));
        assert_eq!(pricing.cache_read_input_token_cost, None);
    }

    // An endpoint quoting a different base price is a different deal, so its
    // cache rate must not be grafted onto the listed price.
    #[test]
    fn endpoints_quoting_another_base_price_are_not_adopted() {
        let endpoints = vec![endpoint(
            "Azure",
            "0.0000035",
            "0.0000175",
            Some("0.00000035"),
        )];

        assert!(
            select_endpoint_pricing(&endpoints, "OpenAI", Some(&listed(1.75e-6, 1.4e-5))).is_none()
        );
    }

    // Cache read and cache write are independent fields, so preferring the
    // first endpoint that publishes a read rate can hide another endpoint
    // that publishes both. Usage with cache-write tokens would then stay
    // unpriceable for no reason.
    #[test]
    fn the_endpoint_publishing_the_most_cache_rates_wins() {
        let endpoints = vec![
            endpoint_with_cache("Azure", "0.00000175", "0.000014", Some("0.000000175"), None),
            endpoint_with_cache(
                "Foundry",
                "0.00000175",
                "0.000014",
                Some("0.000000175"),
                Some("0.0000022"),
            ),
        ];

        let pricing =
            select_endpoint_pricing(&endpoints, "OpenAI", Some(&listed(1.75e-6, 1.4e-5))).unwrap();

        assert_eq!(pricing.cache_read_input_token_cost, Some(1.75e-7));
        assert_eq!(pricing.cache_creation_input_token_cost, Some(2.2e-6));
    }

    #[test]
    fn cache_read_wins_a_cache_rate_count_tie() {
        let endpoints = vec![
            endpoint_with_cache("Azure", "0.00000175", "0.000014", None, Some("0.0000022")),
            endpoint_with_cache(
                "Foundry",
                "0.00000175",
                "0.000014",
                Some("0.000000175"),
                None,
            ),
        ];

        let pricing =
            select_endpoint_pricing(&endpoints, "OpenAI", Some(&listed(1.75e-6, 1.4e-5))).unwrap();

        assert_eq!(pricing.cache_read_input_token_cost, Some(1.75e-7));
        assert_eq!(pricing.cache_creation_input_token_cost, None);
    }

    #[tokio::test]
    async fn list_status_and_decode_failures_remain_explicit() {
        let status = response_server("HTTP/1.1 503 Service Unavailable", "", 3);
        assert!(fetch_all_models_from_api_base(&status, false)
            .await
            .unwrap_err()
            .contains("HTTP 503"));

        let malformed = response_server("HTTP/1.1 200 OK", "not json", 1);
        assert!(fetch_all_models_from_api_base(&malformed, false)
            .await
            .unwrap_err()
            .contains("JSON parse failed"));
    }

    /// Serve the two request shapes a full OpenRouter fetch makes, dispatching
    /// on the path so the author-pricing leg is answered locally instead of
    /// reaching openrouter.ai. `response_server` above cannot do this: it
    /// replays one fixed body for every connection.
    ///
    /// Bounded to the two requests a single fetch makes so the thread and its
    /// listening socket are released when the test ends, rather than parking
    /// on `accept` for the life of the test process.
    fn openrouter_api_server(models_body: &'static str, endpoints_body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for _ in 0..2 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = if request.contains("/endpoints") {
                    endpoints_body
                } else {
                    models_body
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        url
    }

    /// OpenRouter carried the same defect as LiteLLM and models.dev: the write
    /// was gated only on `!result.is_empty()`, never on the caller's opt-out,
    /// so a successful fixture fetch overwrote the developer's real
    /// `pricing-openrouter.json`. No existing test reached the write — the two
    /// cases above both fail before it — which is precisely why the module
    /// needs its own proof rather than inheriting confidence from its siblings.
    ///
    /// See the sibling test in `litellm.rs` for why the assertion redirects
    /// `TOKSCALE_CONFIG_DIR` rather than probing the developer's home.
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

        // `anthropic/` maps to a known author, so this model survives the
        // author filter and drives the endpoints request the fixture answers.
        //
        // The endpoint quotes a different prompt price than the model list on
        // purpose. `fetch_author_pricing` falls back to the listed price on any
        // endpoints failure, so a fixture that quoted the same number on both
        // legs would pass even if the endpoints request had gone to
        // openrouter.ai and failed — which is precisely the regression
        // `API_BASE` exists to prevent. Asserting the endpoint's distinct rate
        // won makes that leg observable.
        let url = openrouter_api_server(
            r#"{"data":[{"id":"anthropic/claude","pricing":{"prompt":"0.000003","completion":"0.000015"}}]}"#,
            r#"{"data":{"id":"anthropic/claude","endpoints":[{"provider_name":"Anthropic","pricing":{"prompt":"0.000009","completion":"0.000015"}}]}}"#,
        );
        let data = fetch_all_models_from_api_base(&url, false)
            .await
            .expect("the fixture serves one priced model");
        assert_eq!(
            data.get("anthropic/claude")
                .and_then(|pricing| pricing.input_cost_per_token),
            Some(9e-6),
            "the local endpoints fixture must have served the author-pricing leg; the listed 3e-6 means it fell back, so this test would no longer catch a hardcoded URL"
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
