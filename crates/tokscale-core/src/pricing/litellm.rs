use super::{cache, describe_error, fetch};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CACHE_FILENAME: &str = "pricing-litellm.json";
const PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricing {
    pub input_cost_per_token: Option<f64>,
    pub input_cost_per_token_above_128k_tokens: Option<f64>,
    pub input_cost_per_token_above_200k_tokens: Option<f64>,
    pub input_cost_per_token_above_256k_tokens: Option<f64>,
    pub input_cost_per_token_above_272k_tokens: Option<f64>,
    pub output_cost_per_token: Option<f64>,
    pub output_cost_per_token_above_128k_tokens: Option<f64>,
    pub output_cost_per_token_above_200k_tokens: Option<f64>,
    pub output_cost_per_token_above_256k_tokens: Option<f64>,
    pub output_cost_per_token_above_272k_tokens: Option<f64>,
    pub cache_creation_input_token_cost: Option<f64>,
    pub cache_creation_input_token_cost_above_200k_tokens: Option<f64>,
    pub cache_read_input_token_cost: Option<f64>,
    pub cache_read_input_token_cost_above_200k_tokens: Option<f64>,
    pub cache_read_input_token_cost_above_272k_tokens: Option<f64>,
}

impl ModelPricing {
    /// Whether this row can price every populated token bucket under
    /// `compute_cost`'s current base-rate fallback semantics. Explicit zeroes
    /// are valid prices; a missing base rate is not covered by a later tier.
    pub(crate) fn covers_usage(&self, usage: &crate::TokenBreakdown) -> bool {
        let valid_rate =
            |rate: Option<f64>| rate.is_some_and(|rate| rate.is_finite() && rate >= 0.0);
        (usage.input <= 0 || valid_rate(self.input_cost_per_token))
            && (usage.output <= 0 && usage.reasoning <= 0 || valid_rate(self.output_cost_per_token))
            && (usage.cache_read <= 0 || valid_rate(self.cache_read_input_token_cost))
            && (usage.cache_write <= 0 || valid_rate(self.cache_creation_input_token_cost))
    }

    /// A copy of this row with rates taken from `fallback` for the buckets
    /// `usage` populates but this row cannot price.
    ///
    /// No rate already present here is ever overwritten, including the
    /// long-context tiers: a row that publishes an above-threshold rate for a
    /// bucket whose base rate it omits keeps that tier, and only the rates it
    /// genuinely lacks are taken from `fallback`. Callers are responsible for
    /// establishing that the two rows price the same deal before borrowing.
    pub(crate) fn with_missing_rates_from(
        &self,
        fallback: &Self,
        usage: &crate::TokenBreakdown,
    ) -> Self {
        let valid_rate =
            |rate: Option<f64>| rate.is_some_and(|rate| rate.is_finite() && rate >= 0.0);
        let valid_or_fallback = |rate: Option<f64>, fallback_rate: Option<f64>| {
            rate.filter(|rate| rate.is_finite() && *rate >= 0.0)
                .or_else(|| fallback_rate.filter(|rate| rate.is_finite() && *rate >= 0.0))
        };
        let mut filled = self.clone();

        if usage.input > 0
            && !valid_rate(filled.input_cost_per_token)
            && valid_rate(fallback.input_cost_per_token)
        {
            filled.input_cost_per_token = fallback.input_cost_per_token;
            filled.input_cost_per_token_above_128k_tokens = valid_or_fallback(
                filled.input_cost_per_token_above_128k_tokens,
                fallback.input_cost_per_token_above_128k_tokens,
            );
            filled.input_cost_per_token_above_200k_tokens = valid_or_fallback(
                filled.input_cost_per_token_above_200k_tokens,
                fallback.input_cost_per_token_above_200k_tokens,
            );
            filled.input_cost_per_token_above_256k_tokens = valid_or_fallback(
                filled.input_cost_per_token_above_256k_tokens,
                fallback.input_cost_per_token_above_256k_tokens,
            );
            filled.input_cost_per_token_above_272k_tokens = valid_or_fallback(
                filled.input_cost_per_token_above_272k_tokens,
                fallback.input_cost_per_token_above_272k_tokens,
            );
        }

        if (usage.output > 0 || usage.reasoning > 0)
            && !valid_rate(filled.output_cost_per_token)
            && valid_rate(fallback.output_cost_per_token)
        {
            filled.output_cost_per_token = fallback.output_cost_per_token;
            filled.output_cost_per_token_above_128k_tokens = valid_or_fallback(
                filled.output_cost_per_token_above_128k_tokens,
                fallback.output_cost_per_token_above_128k_tokens,
            );
            filled.output_cost_per_token_above_200k_tokens = valid_or_fallback(
                filled.output_cost_per_token_above_200k_tokens,
                fallback.output_cost_per_token_above_200k_tokens,
            );
            filled.output_cost_per_token_above_256k_tokens = valid_or_fallback(
                filled.output_cost_per_token_above_256k_tokens,
                fallback.output_cost_per_token_above_256k_tokens,
            );
            filled.output_cost_per_token_above_272k_tokens = valid_or_fallback(
                filled.output_cost_per_token_above_272k_tokens,
                fallback.output_cost_per_token_above_272k_tokens,
            );
        }

        if usage.cache_read > 0
            && !valid_rate(filled.cache_read_input_token_cost)
            && valid_rate(fallback.cache_read_input_token_cost)
        {
            filled.cache_read_input_token_cost = fallback.cache_read_input_token_cost;
            filled.cache_read_input_token_cost_above_200k_tokens = valid_or_fallback(
                filled.cache_read_input_token_cost_above_200k_tokens,
                fallback.cache_read_input_token_cost_above_200k_tokens,
            );
            filled.cache_read_input_token_cost_above_272k_tokens = valid_or_fallback(
                filled.cache_read_input_token_cost_above_272k_tokens,
                fallback.cache_read_input_token_cost_above_272k_tokens,
            );
        }

        if usage.cache_write > 0
            && !valid_rate(filled.cache_creation_input_token_cost)
            && valid_rate(fallback.cache_creation_input_token_cost)
        {
            filled.cache_creation_input_token_cost = fallback.cache_creation_input_token_cost;
            filled.cache_creation_input_token_cost_above_200k_tokens = valid_or_fallback(
                filled.cache_creation_input_token_cost_above_200k_tokens,
                fallback.cache_creation_input_token_cost_above_200k_tokens,
            );
        }

        filled
    }

    pub(crate) fn has_any_usable_base_rate(&self) -> bool {
        [
            self.input_cost_per_token,
            self.output_cost_per_token,
            self.cache_creation_input_token_cost,
            self.cache_read_input_token_cost,
        ]
        .into_iter()
        .any(|rate| rate.is_some_and(|rate| rate.is_finite() && rate >= 0.0))
    }
}

pub type PricingDataset = HashMap<String, ModelPricing>;

#[cfg(test)]
mod pricing_row_tests {
    use super::ModelPricing;
    use crate::TokenBreakdown;

    fn cache_read_usage() -> TokenBreakdown {
        TokenBreakdown {
            input: 10,
            output: 0,
            cache_read: 10,
            cache_write: 0,
            reasoning: 0,
        }
    }

    // A hinted row can publish a long-context tier for a bucket whose base
    // rate it omits. Filling the base must not drag the fallback's tier in
    // with it, or long-context usage silently reprices onto another row.
    #[test]
    fn existing_long_context_tiers_survive_a_filled_base_rate() {
        let hinted = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            output_cost_per_token: Some(1.4e-5),
            cache_read_input_token_cost_above_200k_tokens: Some(5e-7),
            ..Default::default()
        };
        let fallback = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            output_cost_per_token: Some(1.4e-5),
            cache_read_input_token_cost: Some(1.75e-7),
            cache_read_input_token_cost_above_200k_tokens: Some(9.9e-7),
            ..Default::default()
        };

        let filled = hinted.with_missing_rates_from(&fallback, &cache_read_usage());

        assert_eq!(filled.cache_read_input_token_cost, Some(1.75e-7));
        assert_eq!(
            filled.cache_read_input_token_cost_above_200k_tokens,
            Some(5e-7),
            "the hinted row's own long-context tier must be preserved"
        );
    }

    // Absent tiers are still worth filling, otherwise a borrowed base rate
    // walks off a cliff once usage crosses the threshold.
    #[test]
    fn absent_long_context_tiers_are_filled_alongside_the_base_rate() {
        let hinted = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            output_cost_per_token: Some(1.4e-5),
            ..Default::default()
        };
        let fallback = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            output_cost_per_token: Some(1.4e-5),
            cache_read_input_token_cost: Some(1.75e-7),
            cache_read_input_token_cost_above_200k_tokens: Some(9.9e-7),
            ..Default::default()
        };

        let filled = hinted.with_missing_rates_from(&fallback, &cache_read_usage());

        assert_eq!(filled.cache_read_input_token_cost, Some(1.75e-7));
        assert_eq!(
            filled.cache_read_input_token_cost_above_200k_tokens,
            Some(9.9e-7)
        );
    }

    #[test]
    fn invalid_long_context_tiers_fall_back_to_valid_tiers() {
        let hinted = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            output_cost_per_token: Some(1.4e-5),
            cache_read_input_token_cost_above_200k_tokens: Some(f64::NAN),
            ..Default::default()
        };
        let fallback = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            output_cost_per_token: Some(1.4e-5),
            cache_read_input_token_cost: Some(1.75e-7),
            cache_read_input_token_cost_above_200k_tokens: Some(9.9e-7),
            ..Default::default()
        };

        let filled = hinted.with_missing_rates_from(&fallback, &cache_read_usage());

        assert_eq!(
            filled.cache_read_input_token_cost_above_200k_tokens,
            Some(9.9e-7)
        );
    }

    // A bucket the usage does not touch is never filled.
    #[test]
    fn untouched_buckets_are_left_alone() {
        let hinted = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            ..Default::default()
        };
        let fallback = ModelPricing {
            input_cost_per_token: Some(1.75e-6),
            cache_creation_input_token_cost: Some(2e-6),
            ..Default::default()
        };

        let filled = hinted.with_missing_rates_from(&fallback, &cache_read_usage());

        assert_eq!(filled.cache_creation_input_token_cost, None);
    }
}

pub fn load_cached() -> Option<PricingDataset> {
    cache::load_cache(CACHE_FILENAME)
}

pub fn load_cached_any_age() -> Option<PricingDataset> {
    cache::load_cache_any_age(CACHE_FILENAME)
}

pub async fn fetch() -> Result<PricingDataset, String> {
    fetch_inner(PRICING_URL, true).await
}

async fn fetch_inner(url: &str, use_cache: bool) -> Result<PricingDataset, String> {
    if use_cache {
        if let Some(cached) = load_cached() {
            return Ok(cached);
        }
    }

    let client = fetch::pricing_client()?;
    let response = fetch::get_with_retry(&client, url, "LiteLLM").await?;
    let mut data = response
        .json::<PricingDataset>()
        .await
        .map_err(|error| describe_error(&error))?;
    data.retain(|_, pricing| pricing.has_any_usable_base_rate());
    if data.is_empty() {
        return Err("LiteLLM returned no usable pricing rows".to_string());
    }
    if let Err(e) = cache::save_cache(CACHE_FILENAME, &data) {
        eprintln!(
            "[tokscale] Warning: Failed to cache LiteLLM pricing at {}: {}",
            cache::get_cache_path(CACHE_FILENAME).display(),
            e
        );
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Serve one 200 response whose body is well-formed JSON that does not fit
    /// `PricingDataset` (a string where an f64 is expected) — the shape an
    /// upstream LiteLLM schema change would take.
    fn pricing_server(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });

        url
    }

    fn malformed_pricing_server() -> String {
        pricing_server(r#"{"some-model":{"input_cost_per_token":"not-a-number"}}"#)
    }

    /// Serve `MAX_RETRIES` responses with a retryable status, so every attempt
    /// is consumed. Mirrors `models_dev::tests::retryable_status_server`.
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

    /// A client that cannot outlive a wedged listener thread: without this the
    /// tests below block forever instead of failing if `accept` never fires.
    fn bounded_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap()
    }

    // Regression: retryable statuses never recorded `last_error`, so exhausting
    // the retries on 5xx/429 panicked out of `fetch` instead of returning Err.
    // That defeated the caller's whole "no single source may be fatal" contract,
    // because a panic never reaches the caller at all.
    #[tokio::test]
    async fn retryable_statuses_return_an_error_rather_than_panicking() {
        let url = retryable_status_server("HTTP/1.1 503 Service Unavailable");

        let result = fetch_inner(&url, false).await;

        assert!(
            result.is_err(),
            "exhausted retries on 503 must surface as Err so the caller can degrade"
        );
    }

    #[tokio::test]
    async fn rate_limit_status_returns_an_error_rather_than_panicking() {
        let url = retryable_status_server("HTTP/1.1 429 Too Many Requests");

        let result = fetch_inner(&url, false).await;

        assert!(result.is_err(), "429 is retried the same way 5xx is");
    }

    #[tokio::test]
    async fn tier_only_rows_are_not_cached_as_usable_pricing() {
        let url =
            pricing_server(r#"{"tier-only":{"input_cost_per_token_above_272k_tokens":0.00001}}"#);

        let error = fetch_inner(&url, false)
            .await
            .expect_err("a tier rate without a base rate cannot price all tokens");

        assert!(error.contains("no usable pricing rows"));
    }

    #[tokio::test]
    async fn tier_only_rows_are_removed_from_an_otherwise_usable_response() {
        let url = pricing_server(
            r#"{
                "tier-only":{"input_cost_per_token_above_272k_tokens":0.00001},
                "usable":{"input_cost_per_token":0.000005}
            }"#,
        );

        let data = fetch_inner(&url, false)
            .await
            .expect("the response contains one usable base-priced row");

        assert!(!data.contains_key("tier-only"));
        assert!(data.contains_key("usable"));
    }

    // Pins the mechanism behind #1002: reqwest's Display collapses ANY body
    // decode failure to one opaque sentence, so the reported message proves
    // only that a response arrived and could not be deserialized — it says
    // nothing about TLS, and cannot mean "no connection was made".
    //
    // Asserted as "Display omits what describe_error recovers" rather than
    // against reqwest's and serde_json's exact wording: the wording is upstream
    // prose that a dependency bump may reword, and pinning it would redden this
    // test without any tokscale defect.
    #[tokio::test]
    async fn reqwest_display_hides_the_decode_cause_that_describe_error_recovers() {
        let url = malformed_pricing_server();
        let error = bounded_client()
            .get(&url)
            .send()
            .await
            .expect("the request itself succeeds")
            .json::<PricingDataset>()
            .await
            .expect_err("the body must fail to deserialize");

        // Anchored on the offending value, which this fixture owns, rather than
        // on reqwest's or serde_json's phrasing, which it does not.
        let displayed = error.to_string();
        assert!(
            !displayed.contains("not-a-number"),
            "Display must say nothing about the payload — that is the bug: {}",
            displayed
        );

        let described = describe_error(&error);
        assert!(
            described.starts_with(&displayed) && described.len() > displayed.len(),
            "describe_error must extend Display with the source chain, got: {}",
            described
        );
        assert!(
            described.contains("not-a-number"),
            "describe_error must surface the serde cause naming the bad value, got: {}",
            described
        );
    }

    #[test]
    fn test_deserialize_model_pricing_with_above_200k_fields() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token": 0.0000015,
                "input_cost_per_token_above_200k_tokens": 0.000003,
                "output_cost_per_token": 0.0000075,
                "output_cost_per_token_above_200k_tokens": 0.000015,
                "cache_creation_input_token_cost": 0.000001875,
                "cache_creation_input_token_cost_above_200k_tokens": 0.00000375,
                "cache_read_input_token_cost": 0.00000015,
                "cache_read_input_token_cost_above_200k_tokens": 0.0000003
            }"#,
        )
        .unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(0.0000015));
        assert_eq!(
            pricing.input_cost_per_token_above_200k_tokens,
            Some(0.000003)
        );
        assert_eq!(pricing.output_cost_per_token, Some(0.0000075));
        assert_eq!(
            pricing.output_cost_per_token_above_200k_tokens,
            Some(0.000015)
        );
        assert_eq!(pricing.cache_creation_input_token_cost, Some(0.000001875));
        assert_eq!(
            pricing.cache_creation_input_token_cost_above_200k_tokens,
            Some(0.00000375)
        );
        assert_eq!(pricing.cache_read_input_token_cost, Some(0.00000015));
        assert_eq!(
            pricing.cache_read_input_token_cost_above_200k_tokens,
            Some(0.0000003)
        );
    }

    #[test]
    fn test_deserialize_model_pricing_without_above_200k_fields() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token": 0.00000125,
                "output_cost_per_token": 0.00001,
                "cache_creation_input_token_cost": 0.00000125,
                "cache_read_input_token_cost": 0.000000125
            }"#,
        )
        .unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(0.00000125));
        assert_eq!(pricing.input_cost_per_token_above_200k_tokens, None);
        assert_eq!(pricing.output_cost_per_token, Some(0.00001));
        assert_eq!(pricing.output_cost_per_token_above_200k_tokens, None);
        assert_eq!(pricing.cache_creation_input_token_cost, Some(0.00000125));
        assert_eq!(
            pricing.cache_creation_input_token_cost_above_200k_tokens,
            None
        );
        assert_eq!(pricing.cache_read_input_token_cost, Some(0.000000125));
        assert_eq!(pricing.cache_read_input_token_cost_above_200k_tokens, None);
    }

    #[test]
    fn test_deserialize_model_pricing_with_above_272k_fields() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token": 0.000005,
                "input_cost_per_token_above_272k_tokens": 0.000010,
                "output_cost_per_token": 0.000030,
                "output_cost_per_token_above_272k_tokens": 0.000045,
                "cache_read_input_token_cost": 0.0000005,
                "cache_read_input_token_cost_above_272k_tokens": 0.000001
            }"#,
        )
        .unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(0.000005));
        assert_eq!(
            pricing.input_cost_per_token_above_272k_tokens,
            Some(0.000010)
        );
        assert_eq!(pricing.output_cost_per_token, Some(0.000030));
        assert_eq!(
            pricing.output_cost_per_token_above_272k_tokens,
            Some(0.000045)
        );
        assert_eq!(pricing.cache_read_input_token_cost, Some(0.0000005));
        assert_eq!(
            pricing.cache_read_input_token_cost_above_272k_tokens,
            Some(0.000001)
        );
    }
}
