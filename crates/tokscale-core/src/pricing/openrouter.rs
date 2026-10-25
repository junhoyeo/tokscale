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
struct OpenRouterPricing {
    prompt: String,
    completion: String,
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
    #[serde(default)]
    input_cache_write_1h: Option<String>,
    // Keep override entries raw until conversion. OpenRouter publishes several
    // unrelated override shapes, and one malformed entry must not reject the
    // model list or an endpoint response.
    #[serde(default, deserialize_with = "deserialize_overrides")]
    overrides: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct PricingOverride {
    /// Read as `f64` so an integral float (`100000.0`) is recognised as well
    /// as an integer; only the exact 100,000 threshold is used.
    min_prompt_tokens: f64,
    prompt: String,
    completion: String,
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
    #[serde(default)]
    input_cache_write_1h: Option<String>,
}

#[derive(Deserialize)]
struct ModelListItem {
    id: String,
    pricing: Option<OpenRouterPricing>,
}

#[derive(Deserialize)]
struct ModelsListResponse {
    data: Vec<ModelListItem>,
}

fn deserialize_overrides<'de, D>(deserializer: D) -> Result<Vec<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Array(entries) => entries,
        _ => Vec::new(),
    })
}

/// Structs for `/api/v1/models/{id}/endpoints` endpoint (author pricing).

#[derive(Deserialize)]
struct Endpoint {
    provider_name: String,
    pricing: OpenRouterPricing,
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

struct Parsed100kOverride {
    input: f64,
    output: f64,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    cache_write_1h: Option<f64>,
}

fn parse_optional_price(value: Option<&str>) -> Option<Option<f64>> {
    match value {
        Some(value) => parse_price(value).map(Some),
        None => Some(None),
    }
}

fn parse_100k_override(value: &serde_json::Value) -> Option<Parsed100kOverride> {
    let value: PricingOverride = serde_json::from_value(value.clone()).ok()?;
    if value.min_prompt_tokens != 100_000.0 {
        return None;
    }

    Some(Parsed100kOverride {
        input: parse_price(&value.prompt)?,
        output: parse_price(&value.completion)?,
        cache_read: parse_optional_price(value.input_cache_read.as_deref())?,
        cache_write: parse_optional_price(value.input_cache_write.as_deref())?,
        cache_write_1h: parse_optional_price(value.input_cache_write_1h.as_deref())?,
    })
}

fn model_pricing(pricing: &OpenRouterPricing) -> Option<ModelPricing> {
    let above_100k = pricing.overrides.iter().find_map(parse_100k_override);

    Some(ModelPricing {
        input_cost_per_token: Some(parse_price(&pricing.prompt)?),
        input_cost_per_token_above_100k_tokens: above_100k.as_ref().map(|tier| tier.input),
        output_cost_per_token: Some(parse_price(&pricing.completion)?),
        output_cost_per_token_above_100k_tokens: above_100k.as_ref().map(|tier| tier.output),
        cache_read_input_token_cost: pricing.input_cache_read.as_deref().and_then(parse_price),
        cache_read_input_token_cost_above_100k_tokens: above_100k
            .as_ref()
            .and_then(|tier| tier.cache_read),
        cache_creation_input_token_cost: pricing.input_cache_write.as_deref().and_then(parse_price),
        cache_creation_input_token_cost_above_100k_tokens: above_100k
            .as_ref()
            .and_then(|tier| tier.cache_write),
        cache_creation_input_token_cost_above_1hr: pricing
            .input_cache_write_1h
            .as_deref()
            .and_then(parse_price),
        cache_creation_input_token_cost_above_1hr_above_100k_tokens: above_100k
            .as_ref()
            .and_then(|tier| tier.cache_write_1h),
        ..Default::default()
    })
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
    model_pricing(&endpoint.pricing)
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
///
/// One author can serve the same model from several endpoints that differ only
/// by service tier — `openai` against `openai/flex` and `openai/fast`,
/// `google-vertex/global` against `.../flex` and `.../priority` — and all of
/// them report the same `provider_name`. Taking the first match therefore made
/// the stored price depend on OpenRouter's response ordering, which currently
/// returns the discounted Flex tier first and so halved every rate for 26
/// models. Flex is a real price but not the one that ran: it has to be opted
/// into per request, and Codex records `"service_tier":"priority"` or
/// `"default"`, never `flex`. The listed price on `/models` is the standard
/// tier, so it breaks the tie between endpoints of one author. `tag` is
/// deliberately not used here: tier naming is not uniform across authors, so
/// it would need per-author rules where the listed price needs none.
fn select_endpoint_pricing(
    endpoints: &[Endpoint],
    author_name: &str,
    listed: Option<&ModelPricing>,
) -> Option<ModelPricing> {
    let author_endpoints: Vec<&Endpoint> = endpoints
        .iter()
        .filter(|e| e.provider_name == author_name)
        .collect();

    if let Some(first) = author_endpoints.first() {
        if let Some(listed) = listed {
            let standard_endpoints: Vec<ModelPricing> = author_endpoints
                .iter()
                .copied()
                .filter_map(endpoint_pricing)
                .filter(|pricing| quotes_same_base_price(pricing, listed))
                .collect();
            // The first match stands only when no author endpoint quotes the
            // listed price, so a model whose author price legitimately differs
            // from the listed one is left exactly as it was. When at least one
            // endpoint does quote it, see `select_same_base_card` for how the
            // listed card corroborates and completes the chosen endpoint.
            if !standard_endpoints.is_empty() {
                return select_same_base_card(listed, standard_endpoints);
            }
        }
        return endpoint_pricing(first);
    }

    let listed = listed?;
    let matching_endpoints: Vec<ModelPricing> = endpoints
        .iter()
        .filter_map(endpoint_pricing)
        .filter(|pricing| quotes_same_base_price(pricing, listed))
        .collect();
    if matching_endpoints.is_empty() {
        return None;
    }
    select_same_base_card(listed, matching_endpoints)
}

/// Pick the card for a model from the endpoints that quote its listed base
/// price (input and output).
///
/// The listed `/models` card is checked against each endpoint on the optional
/// rates both publish:
///
/// - An endpoint *corroborates* the listing when they share at least one
///   optional rate and agree on every shared one ([`optional_rates_agree`]).
///   When any endpoint corroborates it, only those compete. The choice then
///   does not depend on the order OpenRouter returns the endpoints in, and a
///   card from another deal (a regional or tiered price with the same base
///   rates) cannot win on its field count.
/// - When no endpoint corroborates the listing but one *contradicts* it,
///   every same-base endpoint competes as before, and the listing is not
///   used: an endpoint's own quote outranks it. The listing can show another
///   provider's rate. `z-ai/glm-4.6v` lists Novita's cache-read price while
///   Z.AI's own endpoint quotes a different one, and the author's price must
///   win. An endpoint that publishes none of the listed optional rates does
///   not count as corroboration, so it cannot bring the listing back in.
/// - The candidate leaving the fewest buckets unpriceable wins
///   ([`fewest_unpriceable_buckets`]). Unless an endpoint contradicts the
///   listing with no endpoint corroborating it, the listing then fills the
///   optional buckets the winner leaves empty, so a decoded endpoint cannot
///   erase rates `/models` publishes for the same deal. The listed card never
///   replaces a rate the endpoint quotes.
fn select_same_base_card(
    listed: &ModelPricing,
    same_base_endpoints: Vec<ModelPricing>,
) -> Option<ModelPricing> {
    let corroborates = |candidate: &ModelPricing| {
        shares_an_optional_rate(listed, candidate) && optional_rates_agree(listed, candidate)
    };
    if same_base_endpoints.iter().any(corroborates) {
        return fewest_unpriceable_buckets(same_base_endpoints.into_iter().filter(corroborates))
            .map(|card| with_missing_optional_rates_from(card, listed));
    }
    let contradicted = same_base_endpoints
        .iter()
        .any(|candidate| !optional_rates_agree(listed, candidate));
    let card = fewest_unpriceable_buckets(same_base_endpoints.into_iter())?;
    Some(if contradicted {
        card
    } else {
        with_missing_optional_rates_from(card, listed)
    })
}

/// Whether both cards publish at least one of the same optional rates.
fn shares_an_optional_rate(listed: &ModelPricing, candidate: &ModelPricing) -> bool {
    optional_rates(listed)
        .into_iter()
        .zip(optional_rates(candidate))
        .any(|pair| matches!(pair, (Some(_), Some(_))))
}

/// `card` with each optional bucket it leaves empty taken from `listed`. It
/// only fills empty buckets, so it adds rates and never changes one.
fn with_missing_optional_rates_from(mut card: ModelPricing, listed: &ModelPricing) -> ModelPricing {
    let fill = |slot: &mut Option<f64>, listed_rate: Option<f64>| {
        if slot.is_none() {
            *slot = listed_rate;
        }
    };
    fill(
        &mut card.cache_read_input_token_cost,
        listed.cache_read_input_token_cost,
    );
    fill(
        &mut card.cache_creation_input_token_cost,
        listed.cache_creation_input_token_cost,
    );
    fill(
        &mut card.cache_creation_input_token_cost_above_1hr,
        listed.cache_creation_input_token_cost_above_1hr,
    );
    fill(
        &mut card.input_cost_per_token_above_100k_tokens,
        listed.input_cost_per_token_above_100k_tokens,
    );
    fill(
        &mut card.output_cost_per_token_above_100k_tokens,
        listed.output_cost_per_token_above_100k_tokens,
    );
    fill(
        &mut card.cache_read_input_token_cost_above_100k_tokens,
        listed.cache_read_input_token_cost_above_100k_tokens,
    );
    fill(
        &mut card.cache_creation_input_token_cost_above_100k_tokens,
        listed.cache_creation_input_token_cost_above_100k_tokens,
    );
    fill(
        &mut card.cache_creation_input_token_cost_above_1hr_above_100k_tokens,
        listed.cache_creation_input_token_cost_above_1hr_above_100k_tokens,
    );
    card
}

/// Reduce equally-priced candidates to the one that leaves the fewest token
/// buckets unpriceable.
///
/// Cache buckets and long-context cards are independent fields, so the row
/// publishing the most of them wins. On an equal count, retain cache-read
/// pricing: it is the bucket required by Codex usage and must not be lost to
/// an earlier write-only endpoint. [`select_same_base_card`] decides which
/// candidates reach it.
fn fewest_unpriceable_buckets(
    candidates: impl Iterator<Item = ModelPricing>,
) -> Option<ModelPricing> {
    candidates.reduce(|best, candidate| {
        if published_optional_rates(&candidate) > published_optional_rates(&best)
            || (published_optional_rates(&candidate) == published_optional_rates(&best)
                && candidate.cache_read_input_token_cost.is_some()
                && best.cache_read_input_token_cost.is_none())
        {
            candidate
        } else {
            best
        }
    })
}

fn optional_rates(pricing: &ModelPricing) -> [Option<f64>; 8] {
    [
        pricing.cache_read_input_token_cost,
        pricing.cache_creation_input_token_cost,
        pricing.cache_creation_input_token_cost_above_1hr,
        pricing.input_cost_per_token_above_100k_tokens,
        pricing.output_cost_per_token_above_100k_tokens,
        pricing.cache_read_input_token_cost_above_100k_tokens,
        pricing.cache_creation_input_token_cost_above_100k_tokens,
        pricing.cache_creation_input_token_cost_above_1hr_above_100k_tokens,
    ]
}

fn published_optional_rates(pricing: &ModelPricing) -> usize {
    optional_rates(pricing)
        .into_iter()
        .filter(Option::is_some)
        .count()
}

/// Whether `candidate` quotes the same rate as `reference` for every optional
/// bucket both publish. A bucket only one of them publishes does not count
/// against agreement.
fn optional_rates_agree(reference: &ModelPricing, candidate: &ModelPricing) -> bool {
    optional_rates(reference)
        .into_iter()
        .zip(optional_rates(candidate))
        .all(|pair| match pair {
            (Some(reference), Some(candidate)) => {
                (reference - candidate).abs() <= reference.abs() * 1e-9
            }
            _ => true,
        })
}

/// Fetch all models and get author pricing for each
pub(super) async fn fetch_uncached() -> Result<HashMap<String, ModelPricing>, String> {
    fetch_all_models_from_api_base(API_BASE, false).await
}

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
            let fallback = m.pricing.as_ref().and_then(model_pricing);
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
    use crate::{pricing::PricingService, TokenBreakdown};
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
            pricing: OpenRouterPricing {
                prompt: prompt.to_string(),
                completion: completion.to_string(),
                input_cache_read: input_cache_read.map(str::to_string),
                input_cache_write: input_cache_write.map(str::to_string),
                input_cache_write_1h: None,
                overrides: Vec::new(),
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
    fn endpoints_the_listing_corroborates_win_in_any_order() {
        // Two same-base endpoints disagree on the cache-read rate, and the
        // listed `/models` card agrees with one of them. Whatever order
        // OpenRouter returns the endpoints in, the corroborated one wins, and
        // the listed card fills the cache-write rate it leaves empty.
        let disagreeing = || endpoint("OpenAI", "0.00001", "0.00005", Some("0.000002"));
        let agreeing = || endpoint("OpenAI", "0.00001", "0.00005", Some("0.000001"));
        let mut listed_card = listed(1e-5, 5e-5);
        listed_card.cache_read_input_token_cost = Some(1e-6);
        listed_card.cache_creation_input_token_cost = Some(1.25e-5);

        // "OpenAI" exercises the author branch, "Somebody Else" the branch
        // for a model with no endpoint from its author.
        for author in ["OpenAI", "Somebody Else"] {
            for endpoints in [
                vec![disagreeing(), agreeing()],
                vec![agreeing(), disagreeing()],
            ] {
                let pricing =
                    select_endpoint_pricing(&endpoints, author, Some(&listed_card)).unwrap();
                assert_eq!(pricing.cache_read_input_token_cost, Some(1e-6), "{author}");
                assert_eq!(
                    pricing.cache_creation_input_token_cost,
                    Some(1.25e-5),
                    "{author}"
                );
            }
        }
    }

    // Live shape of `z-ai/glm-4.6v` (2026-10-14): `/models` lists Novita's
    // cache-read price, while Z.AI's own endpoint quotes another one. The
    // author's own quote must win over the listing.
    #[test]
    fn an_endpoint_quote_outranks_a_listing_that_disagrees() {
        let endpoints = vec![
            endpoint("Novita", "0.0000003", "0.0000009", Some("0.000000055")),
            endpoint("Z.AI", "0.0000003", "0.0000009", Some("0.00000005")),
        ];
        let mut listed_card = listed(3e-7, 9e-7);
        listed_card.cache_read_input_token_cost = Some(5.5e-8);

        let pricing = select_endpoint_pricing(&endpoints, "Z.AI", Some(&listed_card)).unwrap();
        assert_eq!(pricing.cache_read_input_token_cost, Some(5e-8));
    }

    // `z-ai/glm-4.6v` with a second Z.AI endpoint that publishes no cache
    // rate. That endpoint shares no optional rate with the listing, so it is
    // not corroboration and cannot bring in the listed (Novita) rate over
    // the cache-read rate Z.AI quotes.
    #[test]
    fn an_endpoint_without_optional_rates_does_not_corroborate_the_listing() {
        let mut listed_card = listed(3e-7, 9e-7);
        listed_card.cache_read_input_token_cost = Some(5.5e-8);
        for endpoints in [
            vec![
                endpoint("Z.AI", "0.0000003", "0.0000009", None),
                endpoint("Z.AI", "0.0000003", "0.0000009", Some("0.00000005")),
            ],
            vec![
                endpoint("Z.AI", "0.0000003", "0.0000009", Some("0.00000005")),
                endpoint("Z.AI", "0.0000003", "0.0000009", None),
            ],
        ] {
            let pricing = select_endpoint_pricing(&endpoints, "Z.AI", Some(&listed_card)).unwrap();
            assert_eq!(pricing.cache_read_input_token_cost, Some(5e-8));
        }
    }

    // A decoded endpoint that publishes none of the listed optional rates
    // contradicts nothing, so the listed card still fills them.
    #[test]
    fn the_listing_fills_an_endpoint_that_publishes_no_optional_rate() {
        let mut listed_card = listed(1e-7, 5e-7);
        listed_card.cache_read_input_token_cost = Some(1e-8);
        listed_card.cache_creation_input_token_cost_above_1hr = Some(2e-7);
        listed_card.input_cost_per_token_above_100k_tokens = Some(5e-7);
        listed_card.output_cost_per_token_above_100k_tokens = Some(2.5e-6);
        let endpoints = vec![endpoint("Anthropic", "0.0000001", "0.0000005", None)];

        let pricing = select_endpoint_pricing(&endpoints, "Anthropic", Some(&listed_card)).unwrap();
        assert_eq!(pricing.cache_read_input_token_cost, Some(1e-8));
        assert_eq!(
            pricing.cache_creation_input_token_cost_above_1hr,
            Some(2e-7)
        );
        assert_eq!(pricing.input_cost_per_token_above_100k_tokens, Some(5e-7));
        assert_eq!(
            pricing.output_cost_per_token_above_100k_tokens,
            Some(2.5e-6)
        );
    }

    #[test]
    fn a_regional_100k_card_never_outranks_a_corroborated_endpoint() {
        let haiku_endpoint = |cache_write: Option<&str>,
                              input_above: &str,
                              output_above: &str,
                              cache_read_above: Option<&str>| {
            let mut above_100k = serde_json::json!({
                "min_prompt_tokens": 100000,
                "prompt": input_above,
                "completion": output_above,
            });
            if let Some(rate) = cache_read_above {
                above_100k["input_cache_read"] = serde_json::json!(rate);
            }
            Endpoint {
                provider_name: "Anthropic".to_string(),
                pricing: OpenRouterPricing {
                    prompt: "0.0000001".to_string(),
                    completion: "0.0000005".to_string(),
                    input_cache_read: Some("0.00000001".to_string()),
                    input_cache_write: cache_write.map(str::to_string),
                    input_cache_write_1h: None,
                    overrides: vec![above_100k],
                },
            }
        };
        let listed_card = ModelPricing {
            input_cost_per_token: Some(1e-7),
            output_cost_per_token: Some(5e-7),
            cache_read_input_token_cost: Some(1e-8),
            cache_creation_input_token_cost: Some(1.25e-7),
            input_cost_per_token_above_100k_tokens: Some(5e-7),
            output_cost_per_token_above_100k_tokens: Some(2.5e-6),
            ..Default::default()
        };
        // Same base rates, more published buckets, but another deal's 100K card.
        let regional = || haiku_endpoint(Some("0.000000125"), "0.00000055", "0.00000275", None);
        // The listed deal, publishing fewer buckets than the regional card.
        let sparser = || haiku_endpoint(None, "0.0000005", "0.0000025", None);
        // The listed deal, publishing more buckets than the listed card.
        let fuller = || {
            haiku_endpoint(
                Some("0.000000125"),
                "0.0000005",
                "0.0000025",
                Some("0.00000005"),
            )
        };

        for endpoints in [vec![regional(), sparser()], vec![sparser(), regional()]] {
            let pricing =
                select_endpoint_pricing(&endpoints, "Anthropic", Some(&listed_card)).unwrap();
            assert_eq!(pricing.input_cost_per_token_above_100k_tokens, Some(5e-7));
            assert_eq!(
                pricing.output_cost_per_token_above_100k_tokens,
                Some(2.5e-6)
            );
            // The endpoint omits the cache-write rate; the listed card fills it.
            assert_eq!(pricing.cache_creation_input_token_cost, Some(1.25e-7));
        }

        for endpoints in [vec![regional(), fuller()], vec![fuller(), regional()]] {
            let pricing =
                select_endpoint_pricing(&endpoints, "Anthropic", Some(&listed_card)).unwrap();
            assert_eq!(pricing.input_cost_per_token_above_100k_tokens, Some(5e-7));
            assert_eq!(
                pricing.output_cost_per_token_above_100k_tokens,
                Some(2.5e-6)
            );
            assert_eq!(
                pricing.cache_read_input_token_cost_above_100k_tokens,
                Some(5e-8)
            );
        }
    }

    #[test]
    fn a_100k_override_threshold_may_be_written_as_a_float() {
        let input_above_100k = |threshold: &str| {
            let raw = format!(
                r#"{{"min_prompt_tokens":{threshold},"prompt":"0.0000005","completion":"0.0000025"}}"#
            );
            parse_100k_override(&serde_json::from_str(&raw).unwrap()).map(|tier| tier.input)
        };
        assert_eq!(input_above_100k("100000"), Some(5e-7));
        assert_eq!(input_above_100k("100000.0"), Some(5e-7));
        assert_eq!(input_above_100k("100000.5"), None);
        assert_eq!(input_above_100k(r#""100000""#), None);
        assert_eq!(input_above_100k("128000"), None);
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

    // OpenRouter serves `openai/gpt-6-astra` from three `OpenAI` endpoints that
    // differ only by service tier, and returns the discounted `openai/flex` one
    // first. Selecting by `provider_name` alone therefore stored $5/$25 for a
    // model whose standard rate — and whose `/models` listed price — is
    // $10/$50, halving every reported cost for it.
    #[test]
    fn a_discounted_tier_does_not_outrank_the_standard_author_endpoint() {
        let endpoints = vec![
            endpoint("OpenAI", "0.000005", "0.000025", Some("0.0000005")),
            endpoint("Azure", "0.00001", "0.00005", Some("0.000001")),
            endpoint("OpenAI", "0.00001", "0.00005", Some("0.000001")),
        ];

        let pricing =
            select_endpoint_pricing(&endpoints, "OpenAI", Some(&listed(1e-5, 5e-5))).unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(1e-5));
        assert_eq!(pricing.output_cost_per_token, Some(5e-5));
        assert_eq!(pricing.cache_read_input_token_cost, Some(1e-6));
    }

    // The error is not one-directional: `openai/fast` is 2x the standard rate,
    // so the same ordering dependency overcharges if OpenRouter ever returns
    // that tier first.
    #[test]
    fn a_premium_tier_does_not_outrank_the_standard_author_endpoint() {
        let endpoints = vec![
            endpoint("OpenAI", "0.00002", "0.0001", Some("0.000002")),
            endpoint("OpenAI", "0.00001", "0.00005", Some("0.000001")),
        ];

        let pricing =
            select_endpoint_pricing(&endpoints, "OpenAI", Some(&listed(1e-5, 5e-5))).unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(1e-5));
        assert_eq!(pricing.output_cost_per_token, Some(5e-5));
    }

    // Tiering is not an OpenAI shape. Google serves `gemini-3.6-flash` from
    // `google-vertex/global/flex` before `google-vertex/global`, both as
    // `Google`, and no 272k LiteLLM preference shields Gemini the way it
    // currently shields some OpenAI models.
    #[test]
    fn the_tier_rule_is_not_specific_to_one_author() {
        let endpoints = vec![
            endpoint("Google", "0.000000375", "0.000001875", Some("0.0000000375")),
            endpoint("Google AI Studio", "0.00000075", "0.00000375", None),
            endpoint("Google", "0.00000075", "0.00000375", Some("0.000000075")),
        ];

        let pricing =
            select_endpoint_pricing(&endpoints, "Google", Some(&listed(7.5e-7, 3.75e-6))).unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(7.5e-7));
        assert_eq!(pricing.output_cost_per_token, Some(3.75e-6));
        assert_eq!(pricing.cache_read_input_token_cost, Some(7.5e-8));
    }

    // Z.AI quantization tiers (`z-ai/fp4`, `z-ai/fp8`) quote prices that match
    // no listed price, and the listed price there is a reseller's. Those models
    // must keep resolving exactly as before, so the tier rule stays additive.
    #[test]
    fn author_endpoint_still_wins_when_none_quotes_the_listed_price() {
        let endpoints = vec![
            endpoint("Z.AI", "0.0000006", "0.0000022", None),
            endpoint("Z.AI", "0.0000008", "0.0000029", None),
        ];

        let pricing =
            select_endpoint_pricing(&endpoints, "Z.AI", Some(&listed(4e-7, 2e-6))).unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(6e-7));
        assert_eq!(pricing.output_cost_per_token, Some(2.2e-6));
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
    fn openrouter_api_server(models_body: &str, endpoints_body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let models_body = models_body.to_string();
        let endpoints_body = endpoints_body.to_string();
        thread::spawn(move || {
            for _ in 0..2 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = if request.contains("/endpoints") {
                    &endpoints_body
                } else {
                    &models_body
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

    #[tokio::test]
    async fn haiku_55_overrides_price_a_long_request_and_ignore_bad_entries() {
        let pricing = r#"{
            "prompt":"0.0000001",
            "completion":"0.0000005",
            "input_cache_read":"0.00000001",
            "input_cache_write":"0.000000125",
            "input_cache_write_1h":"0.0000002",
            "overrides":[
                null,
                {"min_prompt_tokens":100000,"prompt":"not-a-rate","completion":"0.0000025"},
                {"min_prompt_tokens":128000,"prompt":"9","completion":"9"},
                {
                    "min_prompt_tokens":100000,
                    "prompt":"0.0000005",
                    "completion":"0.0000025",
                    "input_cache_read":"0.00000005",
                    "input_cache_write":"0.000000625",
                    "input_cache_write_1h":"0.000001"
                }
            ]
        }"#;
        let incomplete_pricing = r#"{
            "prompt":"0.0000001",
            "completion":"0.0000005",
            "input_cache_read":"0.00000001",
            "input_cache_write":"0.000000125",
            "overrides":[
                {"min_prompt_tokens":100000,"prompt":"not-a-rate","completion":"0.0000025"}
            ]
        }"#;
        let models =
            format!(r#"{{"data":[{{"id":"anthropic/claude-haiku-5.5","pricing":{pricing}}}]}}"#);
        let endpoints = format!(
            r#"{{"data":{{"id":"anthropic/claude-haiku-5.5","endpoints":[
                {{"provider_name":"Anthropic","pricing":{incomplete_pricing}}},
                {{"provider_name":"Anthropic","pricing":{pricing}}}
            ]}}}}"#
        );
        let url = openrouter_api_server(&models, &endpoints);

        let data = fetch_all_models_from_api_base(&url, false)
            .await
            .expect("bad and unrelated overrides must not drop the OpenRouter model");
        let row = data
            .get("anthropic/claude-haiku-5.5")
            .expect("the Haiku row must survive");
        assert_eq!(row.cache_creation_input_token_cost_above_1hr, Some(2e-7));
        assert_eq!(row.input_cost_per_token_above_100k_tokens, Some(5e-7));
        assert_eq!(row.output_cost_per_token_above_100k_tokens, Some(2.5e-6));
        assert_eq!(
            row.cache_read_input_token_cost_above_100k_tokens,
            Some(5e-8)
        );
        assert_eq!(
            row.cache_creation_input_token_cost_above_100k_tokens,
            Some(6.25e-7)
        );
        assert_eq!(
            row.cache_creation_input_token_cost_above_1hr_above_100k_tokens,
            Some(1e-6)
        );

        // The model-list card is also the fallback when the author endpoint
        // cannot be decoded. Both ingestion legs must preserve the same tier.
        let fallback_url = openrouter_api_server(&models, "not json");
        let fallback = fetch_all_models_from_api_base(&fallback_url, false)
            .await
            .expect("an invalid endpoint body must retain the listed card");
        let fallback = fallback
            .get("anthropic/claude-haiku-5.5")
            .expect("the listed Haiku card must remain available");
        assert_eq!(
            fallback.cache_creation_input_token_cost_above_1hr,
            Some(2e-7)
        );
        assert_eq!(fallback.input_cost_per_token_above_100k_tokens, Some(5e-7));
        assert_eq!(
            fallback.cache_creation_input_token_cost_above_1hr_above_100k_tokens,
            Some(1e-6)
        );

        // A successfully decoded but incomplete same-base endpoint must not
        // replace the complete card from the model list.
        let incomplete_endpoints = format!(
            r#"{{"data":{{"id":"anthropic/claude-haiku-5.5","endpoints":[{{"provider_name":"Anthropic","pricing":{incomplete_pricing}}}]}}}}"#
        );
        let fallback_url = openrouter_api_server(&models, &incomplete_endpoints);
        let fallback = fetch_all_models_from_api_base(&fallback_url, false)
            .await
            .expect("the complete listed card must outrank an incomplete endpoint card");
        let fallback = fallback
            .get("anthropic/claude-haiku-5.5")
            .expect("the listed Haiku card must remain available");
        assert_eq!(
            fallback.cache_creation_input_token_cost_above_1hr,
            Some(2e-7)
        );
        assert_eq!(fallback.input_cost_per_token_above_100k_tokens, Some(5e-7));
        assert_eq!(
            fallback.cache_creation_input_token_cost_above_1hr_above_100k_tokens,
            Some(1e-6)
        );

        let service = PricingService::new(HashMap::new(), data);
        let usage = TokenBreakdown {
            input: 40_000,
            output: 100,
            cache_read: 30_000,
            cache_write: 30_001,
            cache_write_1h: 10_000,
            reasoning: 20,
        };
        assert!(service.covers_usage_with_provider("claude-haiku-5.5", Some("anthropic"), &usage,));
        let cost =
            service.calculate_cost_with_provider("claude-haiku-5.5", Some("anthropic"), &usage);
        let expected = 40_000.0 * 5e-7
            + 120.0 * 2.5e-6
            + 30_000.0 * 5e-8
            + 20_001.0 * 6.25e-7
            + 10_000.0 * 1e-6;
        assert!((cost - expected).abs() < 1e-12, "cost was {cost}");
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
