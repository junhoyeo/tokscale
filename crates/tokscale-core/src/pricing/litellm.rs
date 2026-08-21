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
    /// Every rate this row can carry, base rates and long-context tiers alike,
    /// as one list.
    ///
    /// The destructure below deliberately carries no `..` rest pattern: adding
    /// a field to `ModelPricing` breaks this function at compile time and
    /// forces the new rate into every predicate that reasons over *all* rates.
    /// Leaving that to a comment is not safe enough for
    /// `quotes_zero_for_every_published_rate`, where an overlooked rate is
    /// treated as zero and a new paid tier would read as free.
    pub(crate) fn all_rates(&self) -> [Option<f64>; 15] {
        let Self {
            input_cost_per_token,
            input_cost_per_token_above_128k_tokens,
            input_cost_per_token_above_200k_tokens,
            input_cost_per_token_above_256k_tokens,
            input_cost_per_token_above_272k_tokens,
            output_cost_per_token,
            output_cost_per_token_above_128k_tokens,
            output_cost_per_token_above_200k_tokens,
            output_cost_per_token_above_256k_tokens,
            output_cost_per_token_above_272k_tokens,
            cache_creation_input_token_cost,
            cache_creation_input_token_cost_above_200k_tokens,
            cache_read_input_token_cost,
            cache_read_input_token_cost_above_200k_tokens,
            cache_read_input_token_cost_above_272k_tokens,
        } = *self;

        [
            input_cost_per_token,
            input_cost_per_token_above_128k_tokens,
            input_cost_per_token_above_200k_tokens,
            input_cost_per_token_above_256k_tokens,
            input_cost_per_token_above_272k_tokens,
            output_cost_per_token,
            output_cost_per_token_above_128k_tokens,
            output_cost_per_token_above_200k_tokens,
            output_cost_per_token_above_256k_tokens,
            output_cost_per_token_above_272k_tokens,
            cache_creation_input_token_cost,
            cache_creation_input_token_cost_above_200k_tokens,
            cache_read_input_token_cost,
            cache_read_input_token_cost_above_200k_tokens,
            cache_read_input_token_cost_above_272k_tokens,
        ]
    }

    /// Whether every rate this row publishes is exactly `0.0`, so the buckets
    /// it never quotes can be read as zero as well.
    ///
    /// This is a statement about the row, not about the deal. Rows that quote
    /// `0.0` for the buckets upstream bothered to list and omit the rest reach
    /// us from two different worlds and are indistinguishable in the data:
    /// `opencode/nemotron-3-ultra-free` is a genuinely free row, and
    /// `kenari/claude-opus-4-7` is a premium model whose zeros mean
    /// "included in your subscription" — both arrive as
    /// `{input_cost_per_token: 0.0, output_cost_per_token: 0.0}` with null
    /// cache fields, byte for byte. Nothing in the dataset separates them, so
    /// tokscale reports both at $0.00 and cannot do better from this input
    /// alone. If upstream ever publishes a plan/subscription marker, that is
    /// the signal to split these two cases apart.
    ///
    /// A zero row must not be read as "the model is free": kenari's entire
    /// catalog is subscription-priced at zero (all 38 of its rows in models.dev
    /// are `{0, 0}`, `grok-4-5` and `claude-fable-5` included), and the same
    /// nemotron has 14 paid siblings — `nvidia/nvidia/...` at $0.50/$2.50,
    /// `openrouter/...` at $0.60/$3.60 — so a resolution that lands on a
    /// subscription row prices a paid model at $0.00. That is a lookup-quality
    /// problem, tracked separately, not something this predicate decides.
    ///
    /// That limitation predates this predicate rather than arriving with it:
    /// `0.0` has always satisfied `valid_rate`, so a plan-priced row already
    /// covered — and already reported at $0.00 — any usage without cache
    /// tokens. What this widens is the usage shapes it covers, extending the
    /// same $0.00 answer to cache-bearing usage instead of aborting the
    /// submission that carries it (#1021, #1035).
    ///
    /// Two conditions, and both matter:
    ///
    /// The row must quote base rates for *both* input and output. Those are
    /// the two buckets every completion populates and every provider prices,
    /// so quoting zero for both is the strongest signal the data offers. One
    /// zero rate is not: a row quoting free input while omitting output has
    /// said nothing about generation, which is where the money is, and
    /// extrapolating from it would report a paid model at $0.00.
    ///
    /// Every rate the row quotes must then be zero, tiers included. A zero
    /// base rate beside a paid above-128k tier is a promotional tier rather
    /// than an all-zero row, and keeps the strict coverage check below.
    fn quotes_zero_for_every_published_rate(&self) -> bool {
        let quoted_rate =
            |rate: Option<f64>| rate.is_some_and(|rate| rate.is_finite() && rate >= 0.0);
        quoted_rate(self.input_cost_per_token)
            && quoted_rate(self.output_cost_per_token)
            && self
                .all_rates()
                .into_iter()
                .flatten()
                // Rejects NaN as well as any real price, so a row carrying an
                // unusable rate is never mistaken for a published zero.
                .all(|rate| rate == 0.0)
    }

    /// Whether this row can price every populated token bucket under
    /// `compute_cost`'s current base-rate fallback semantics. Explicit zeroes
    /// are valid prices; a missing base rate is not covered by a later tier.
    pub(crate) fn covers_usage(&self, usage: &crate::TokenBreakdown) -> bool {
        // A row that quotes zero for every rate it publishes prices the ones
        // it omits at zero too, so it covers any usage shape. `compute_cost`
        // already reads an absent rate as 0.0, so this only stops a $0.00
        // submission being rejected — it changes no total.
        //
        // Answering true here deliberately pre-empts `resolve_for_usage`'s
        // canonical-borrow path (#1013), which exists to fill omitted cache
        // rates from the unhinted row. Nothing is lost: an all-zero row can
        // only borrow from a canonical row quoting the same base rates, so
        // every rate it could take is zero and the total is unchanged.
        if self.quotes_zero_for_every_published_rate() {
            return true;
        }

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

    fn every_bucket_usage() -> TokenBreakdown {
        TokenBreakdown {
            input: 1_000,
            output: 500,
            cache_read: 2_000,
            cache_write: 300,
            reasoning: 200,
        }
    }

    // #1021, #1035: a free model whose row omits the redundant cache zeros was
    // judged unpriced the moment a message carried one cached token, and the
    // whole submission aborted.
    #[test]
    fn a_row_priced_entirely_at_zero_covers_cache_usage_it_never_quotes() {
        let free = ModelPricing {
            input_cost_per_token: Some(0.0),
            output_cost_per_token: Some(0.0),
            ..Default::default()
        };

        assert!(free.covers_usage(&every_bucket_usage()));
    }

    // Absence of data is not a price of zero.
    #[test]
    fn a_row_with_no_rates_at_all_covers_nothing() {
        let empty = ModelPricing::default();

        assert!(!empty.covers_usage(&every_bucket_usage()));
        assert!(!empty.covers_usage(&cache_read_usage()));
    }

    // The zero shortcut must never borrow a real rate for a bucket the row
    // leaves unquoted: that would bill cached tokens at the input price.
    #[test]
    fn a_row_charging_for_input_still_does_not_cover_unquoted_cache_reads() {
        let paid = ModelPricing {
            input_cost_per_token: Some(1e-6),
            output_cost_per_token: Some(1e-5),
            ..Default::default()
        };

        assert!(!paid.covers_usage(&cache_read_usage()));
    }

    // A zero base rate beside a paid long-context tier is not an all-zero row,
    // so the strict rule still applies to the buckets it never quotes.
    #[test]
    fn a_zero_base_rate_with_a_paid_tier_does_not_cover_unquoted_cache_reads() {
        let promotional = ModelPricing {
            input_cost_per_token: Some(0.0),
            input_cost_per_token_above_128k_tokens: Some(1e-6),
            output_cost_per_token: Some(0.0),
            ..Default::default()
        };

        assert!(!promotional.covers_usage(&cache_read_usage()));
    }

    // One zero rate is not enough. A row quoting zero input while saying
    // nothing about output has said nothing about generation, so the buckets
    // it omits stay unpriced.
    #[test]
    fn a_row_quoting_only_a_zero_input_rate_does_not_cover_output() {
        let input_only = ModelPricing {
            input_cost_per_token: Some(0.0),
            ..Default::default()
        };

        assert!(!input_only.covers_usage(&every_bucket_usage()));
        assert!(input_only.covers_usage(&TokenBreakdown {
            input: 1_000,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        }));
    }

    // A tier-only row has no base rate to anchor its zeros, so they
    // must not make it cover usage the pricing path would bill at zero anyway.
    #[test]
    fn a_tier_only_zero_row_covers_nothing() {
        let tier_only = ModelPricing {
            input_cost_per_token_above_128k_tokens: Some(0.0),
            ..Default::default()
        };

        assert!(!tier_only.covers_usage(&every_bucket_usage()));
    }

    // Covering the usage is only useful if the price that follows is a real
    // 0.0: an unquoted bucket must not leak a NaN into the leaderboard totals.
    #[test]
    fn an_all_zero_row_prices_cache_usage_at_exactly_zero() {
        let free = ModelPricing {
            input_cost_per_token: Some(0.0),
            output_cost_per_token: Some(0.0),
            ..Default::default()
        };
        let usage = every_bucket_usage();

        let cost = crate::pricing::lookup::compute_cost(
            &free,
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
            usage.reasoning,
        );

        assert_eq!(cost, 0.0);
        assert!(cost.is_finite());
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

/// `use_disk_cache` governs BOTH halves of the on-disk cache — the read below
/// and the write at the end. It used to gate only the read, so a caller that
/// asked for a fresh fetch still published the result to
/// `~/.config/tokscale/cache/pricing-litellm.json`. Any fixture-server test
/// that fetched successfully therefore published its stub over the real
/// multi-thousand-model dataset for a full TTL, and while it was clobbered
/// LiteLLM contributed nothing to pricing lookups (#1021, #1035).
///
/// Gating the write here rather than isolating the tests behind
/// `TOKSCALE_CONFIG_DIR`: the override is process-global, so it only protects
/// the tests that remember to set it, and the next fixture-server test added
/// without it silently reintroduces the bug. A flag on the function makes the
/// opt-out total and local — the caller that declined the cache cannot write to
/// it by construction. The override still earns its keep in the regression
/// test, where it is the only way to observe the write without touching the
/// developer's real cache. No production caller wants read-bypass-with-write:
/// `fetch()` is the sole one and it passes `true`.
async fn fetch_inner(url: &str, use_disk_cache: bool) -> Result<PricingDataset, String> {
    if use_disk_cache {
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
    if use_disk_cache {
        if let Err(e) = cache::save_cache(CACHE_FILENAME, &data) {
            eprintln!(
                "[tokscale] Warning: Failed to cache LiteLLM pricing at {}: {}",
                cache::get_cache_path(CACHE_FILENAME).display(),
                e
            );
        }
    }
    Ok(data)
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

    /// `use_disk_cache` used to gate only the cache READ while the write ran
    /// unconditionally, so every fixture-server test in this module wrote its
    /// two-row fixture over the developer's real
    /// `~/.config/tokscale/cache/pricing-litellm.json`, evicting the genuine
    /// multi-thousand-model LiteLLM dataset for a full TTL. A clobbered cache
    /// contributes nothing to pricing lookups, which is exactly the spurious
    /// "pricing is unavailable for submitted token usage" submit failure
    /// reported in #1021 and #1035 — `cargo test` was manufacturing the bug.
    ///
    /// The assertion redirects `TOKSCALE_CONFIG_DIR` at a `TempDir` instead of
    /// probing the developer's home. `cache::get_cache_path` resolves through
    /// `paths::get_config_dir()` either way, so a write that would have landed
    /// in the real cache lands in the temp dir here: observable without the
    /// test depending on — or risking — whatever the developer's home contains.
    /// The `starts_with` assertion is what makes that substitution honest; if
    /// the redirect ever stopped taking effect the test would otherwise pass
    /// vacuously while the real cache was still being overwritten.
    ///
    /// `#[serial]` is load-bearing for the same reason. `TOKSCALE_CONFIG_DIR`
    /// is process-global, so a concurrent test that restores its own snapshot
    /// of it clears this redirect mid-run; the path captured at the top then
    /// stops being the path the code would write to, and the final assertion
    /// checks an empty temp dir while the real cache is clobbered. The
    /// `assert_eq!` after the fetch catches that breach directly instead of
    /// letting it read as a pass.
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

        let url = pricing_server(r#"{"usable":{"input_cost_per_token":0.000005}}"#);
        let data = fetch_inner(&url, false)
            .await
            .expect("the fixture serves one usable base-priced row");
        assert!(data.contains_key("usable"), "the fetch itself must succeed");

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
