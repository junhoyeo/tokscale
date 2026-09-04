pub mod aliases;
pub mod cache;
pub mod custom;
mod fetch;
pub mod litellm;
pub mod lookup;
pub mod models_dev;
pub mod openrouter;
mod self_hosted;

use custom::CustomPricing;
use lookup::{compute_cost, LookupResult, PricingLookup, ResolutionEvidence, ResolutionKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::TokenBreakdown;

pub use litellm::ModelPricing;

static PRICING_SERVICE: OnceCell<Arc<PricingService>> = OnceCell::const_new();

// @keep: documents non-obvious filtering behavior — without this, the next person
// will wonder why github_copilot entries disappear from the pricing data.
/// Provider prefixes in LiteLLM data that use subscription-based pricing ($0.00)
/// and should be excluded from pay-per-token cost estimation.
const EXCLUDED_LITELLM_PREFIXES: &[&str] = &["github_copilot/"];

// @keep: explains why we do not just print the error.
/// Flatten an error and its `source()` chain into one line.
///
/// `reqwest::Error`'s `Display` is deliberately terse: a body-decode failure
/// renders as the bare string "error decoding response body", and the
/// `serde_json` cause that names the offending field and byte offset hangs off
/// `source()`, which `{}` never walks. Issue #1002 was reported with exactly
/// that message, which is why it was impossible to tell a transport failure
/// from an upstream schema change and the reporter guessed at TLS. Printing the
/// chain makes the next such report actionable.
///
/// The same terseness hides transport failures. `send()` renders every one of
/// them as "error sending request for url (...)", so a certificate rejected by
/// an intercepting proxy, a refused connection and a DNS failure are one
/// string. #1238 was reported against a Windows firewall with exactly that
/// line, and neither the reporter nor I could tell which of the three it was.
///
/// Callers must not pass errors from clients whose request URLs embed tokens or
/// credentials in query parameters: `reqwest` includes the request URL verbatim
/// in its `Display`, and this function prints that string unredacted, so any
/// secret in the URL would leak into logs and error output. Current callers use
/// header-based auth with parameter-free URLs, which is safe.
pub fn describe_error(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(inner) = source {
        parts.push(inner.to_string());
        source = inner.source();
    }
    parts.join(": ")
}

pub struct PricingService {
    custom: CustomPricing,
    lookup: PricingLookup,
}

impl PricingService {
    pub fn new(
        litellm_data: HashMap<String, ModelPricing>,
        openrouter_data: HashMap<String, ModelPricing>,
    ) -> Self {
        Self::new_with_custom(CustomPricing::default(), litellm_data, openrouter_data)
    }

    pub fn new_with_custom(
        custom: CustomPricing,
        litellm_data: HashMap<String, ModelPricing>,
        openrouter_data: HashMap<String, ModelPricing>,
    ) -> Self {
        Self::new_with_custom_and_models_dev(custom, litellm_data, openrouter_data, HashMap::new())
    }

    pub fn new_with_custom_and_models_dev(
        custom: CustomPricing,
        litellm_data: HashMap<String, ModelPricing>,
        openrouter_data: HashMap<String, ModelPricing>,
        models_dev_data: HashMap<String, ModelPricing>,
    ) -> Self {
        Self {
            custom,
            lookup: PricingLookup::new_with_archive(
                litellm_data,
                openrouter_data,
                Self::build_cursor_overrides(),
                Self::build_sakana_overrides(),
                models_dev_data,
                Self::build_archive_overrides(),
            ),
        }
    }

    // @keep: the retain logic is non-trivial (lowercase + prefix match); this doc
    // explains *why* these entries are dropped, not just *what* the code does.
    /// Filter out LiteLLM entries from subscription-based providers (e.g. github_copilot/)
    /// whose $0.00 pricing is meaningless for per-token cost estimation.
    fn filter_litellm_data(
        mut data: HashMap<String, ModelPricing>,
    ) -> HashMap<String, ModelPricing> {
        data.retain(|key, _| {
            let lower = key.to_lowercase();
            let included_provider = !EXCLUDED_LITELLM_PREFIXES
                .iter()
                .any(|prefix| lower.starts_with(prefix));
            included_provider
        });
        data.retain(|_, pricing| pricing.has_any_usable_base_rate());
        data
    }

    // @keep: Cursor-sourced pricing for models not yet in LiteLLM/OpenRouter.
    // Checked after exact/prefix matches but before fuzzy matching in PricingLookup,
    // so real upstream entries (including provider-prefixed like openai/gpt-5.3-codex)
    // always win. Source citations are required for audit trail.
    fn build_cursor_overrides() -> HashMap<String, ModelPricing> {
        // @keep: the difference between `None` and `Some(0.0)` here is load-bearing.
        // The 5th field is cache CREATION. `None` means "rate unknown", and
        // `covers_usage` then reports the row as not covering any usage that
        // populates cache_write — which excludes it from submission entirely.
        // `Some(0.0)` means "documented free". `compute_cost` already reads an
        // absent rate as 0.0, so the two produce an identical cost; only the
        // coverage verdict differs. Set it ONLY where Cursor documents cache
        // creation as free — guessing a rate would invent spend.
        /// `(model id, input, output, cache read, cache creation)`, per token.
        ///
        /// Both cache rates distinguish "unknown" from "free": `None` means the
        /// rate is undocumented, `Some(0.0)` means Cursor publishes it as free.
        type CursorRateRow = (&'static str, f64, f64, Option<f64>, Option<f64>);

        let entries: &[CursorRateRow] = &[
            // GPT-5.3 family: $1.75/$14.00 per 1M tokens, $0.175 cache read
            // Source: Cursor docs (cursor.com/en-US/docs/models), llm-stats.com
            ("gpt-5.3", 0.00000175, 0.000014, Some(1.75e-7), None),
            ("gpt-5.3-codex", 0.00000175, 0.000014, Some(1.75e-7), None),
            (
                "gpt-5.3-codex-spark",
                0.00000175,
                0.000014,
                Some(1.75e-7),
                None,
            ),
            // Composer 1: $1.25/$10.00 per 1M tokens, $0.125 cache read
            // Source: Cursor docs (cursor.com/docs/models#model-pricing)
            ("composer 1", 0.00000125, 0.00001, Some(1.25e-7), None),
            ("composer-1", 0.00000125, 0.00001, Some(1.25e-7), None),
            // Composer 1.5: $3.50/$17.50 per 1M tokens, $0.35 cache read
            // Source: Cursor docs (cursor.com/docs/models#model-pricing), issue #276
            ("composer 1.5", 0.0000035, 0.0000175, Some(3.5e-7), None),
            ("composer-1.5", 0.0000035, 0.0000175, Some(3.5e-7), None),
            // Composer 2: $0.50/$2.50 per 1M input/output, $0.20/M cache read; cache creation free
            // Composer 2 Fast: $1.50/$7.50 per 1M, $0.35/M cache read; cache creation free
            // Source: Cursor docs (cursor.com/docs/models#model-pricing)
            ("composer 2", 5e-7, 2.5e-6, Some(2e-7), Some(0.0)),
            ("composer-2", 5e-7, 2.5e-6, Some(2e-7), Some(0.0)),
            ("composer 2 fast", 1.5e-6, 7.5e-6, Some(3.5e-7), Some(0.0)),
            ("composer-2-fast", 1.5e-6, 7.5e-6, Some(3.5e-7), Some(0.0)),
            // Composer 2: $0.50/$2.50 per 1M input/output, $0.20/M cache read; cache creation free
            // Composer 2 Fast: $1.50/$7.50 per 1M, $0.35/M cache read; cache creation free
            // Source: Cursor docs (cursor.com/docs/models#model-pricing)
            ("composer-2.5", 5e-7, 2.5e-6, Some(2e-7), Some(0.0)),
            ("composer-2.5-fast", 1.5e-6, 7.5e-6, Some(3.5e-7), Some(0.0)),
        ];

        let mut overrides = HashMap::with_capacity(entries.len());
        for (model_id, input, output, cache_read, cache_creation) in entries {
            overrides.insert(
                model_id.to_string(),
                ModelPricing {
                    input_cost_per_token: Some(*input),
                    output_cost_per_token: Some(*output),
                    cache_read_input_token_cost: *cache_read,
                    cache_creation_input_token_cost: *cache_creation,
                    ..Default::default()
                },
            );
        }
        // Grok 4.6: $2.00/$6.00 per 1M input/output, $0.50/M cache read;
        // >200K-context tier $4.00/$12.00 per 1M, $1.00/M cache read
        // Source: Cursor model docs (cursor.com/docs/models#model-pricing);
        // rates mirror models.dev xai/grok-4.6 including the >200K context tier
        overrides.insert(
            "grok-4.6".to_string(),
            ModelPricing {
                input_cost_per_token: Some(2e-6),
                output_cost_per_token: Some(6e-6),
                cache_read_input_token_cost: Some(5e-7),
                input_cost_per_token_above_200k_tokens: Some(4e-6),
                output_cost_per_token_above_200k_tokens: Some(12e-6),
                cache_read_input_token_cost_above_200k_tokens: Some(1e-6),
                ..Default::default()
            },
        );
        overrides
    }

    // @keep: Sakana-sourced pricing for `fugu-ultra`, a model not carried by
    // LiteLLM/OpenRouter/models.dev. Reports source label "Sakana" (NOT "Cursor")
    // and is consulted at the same precedence as the Cursor overrides in
    // PricingLookup — after exact/normalized/prefix upstream matches, before the
    // fuzzy stage — so any real upstream entry always wins. The ModelPricing
    // struct is built directly (not via the 4-tuple shorthand) so the >272K
    // long-context tier fields can be populated; compute_cost DOES read those
    // *_above_272k_tokens fields when input/output/cache-read token volume
    // crosses 272K, so they are live, not inert.
    //
    // Rates source: https://console.sakana.ai/pricing and https://sakana.ai/fugu/
    // (accessed 2026-06-22).
    //   fugu-ultra base: input $5/1M, output $30/1M, cache-read $0.50/1M.
    //   fugu-ultra >272K-context tier: input $10/1M, output $45/1M, cache-read $1/1M.
    //
    // NOTE: there is deliberately NO `fugu` (non-ultra) entry. `fugu` is a
    // router/orchestrator billed at "the standard rate of the underlying
    // top-tier model involved" (https://sakana.ai/fugu/, accessed 2026-06-22):
    // the effective rate is variable per request and is NOT recoverable from the
    // session log, which only records model="fugu" with no record of which
    // underlying model actually served the request. Assigning any fixed
    // per-token rate to bare `fugu` would therefore be incorrect, so it is left
    // unpriced (callers fall through to the normal lookup chain / report no price).
    fn build_sakana_overrides() -> HashMap<String, ModelPricing> {
        let mut overrides = HashMap::with_capacity(1);
        overrides.insert(
            "fugu-ultra".to_string(),
            ModelPricing {
                // Base rates.
                input_cost_per_token: Some(5e-6),
                output_cost_per_token: Some(3e-5),
                cache_read_input_token_cost: Some(5e-7),
                cache_creation_input_token_cost: None,
                // >272K-context tier (consumed by compute_cost's tiered walk).
                input_cost_per_token_above_272k_tokens: Some(1e-5),
                output_cost_per_token_above_272k_tokens: Some(4.5e-5),
                cache_read_input_token_cost_above_272k_tokens: Some(1e-6),
                ..Default::default()
            },
        );
        overrides
    }

    // @keep: a snapshot of published first-party rates for models no live
    // dataset carries first-party. LiteLLM, OpenRouter and models.dev all
    // describe the CURRENT catalogue from resellers as well as vendors: once
    // a provider retires a model those rows are dropped, and some current
    // models were never listed first-party at all (only `deepinfra/*`,
    // `z-ai/*`, `openrouter/*` reseller keys exist). In both cases every
    // historical session that used the model at the publishing endpoint
    // silently loses its price without this memory, and a reseller row that
    // merely shares the model name wins by precedence as an unverified
    // guess. This archive is the only place that memory lives.
    //
    // Precedence is the same slot as the Cursor/Sakana overrides in
    // PricingLookup - after every upstream exact/normalized/prefix match, before
    // the fuzzy stage - so while a row is still live upstream the archive never
    // answers, and once upstream drops it the archive answers with that model's
    // own last rate instead of letting fuzzy elect a neighbouring row.
    //
    // Keys are provider-qualified on purpose. Each vendor bills these rates
    // on its own API; a request routed through another endpoint is a
    // different tariff. With a matching root, a same-vendor hint resolves
    // ProviderScoped with matching provider identity (submission-safe), and
    // any other hint stays an estimate.
    //
    // Rates verified 2026-09-02 against both live datasets: models.dev keys them
    // exactly as written here, LiteLLM keys them bare (`claude-haiku-4-5`, ...)
    // at the same numbers. Cache read is 0.1x input and cache creation 1.25x
    // input, matching Anthropic's published multipliers.
    //
    // NOTE: there is deliberately NO `openai/codex-auto-review` entry, and no
    // entry for any other routing label. OpenAI publishes no tariff for it -
    // all three datasets carry zero `auto-review` keys (checked 2026-09-02), so
    // the number an earlier draft archived was not a published rate but
    // Tokscale's own fuzzy election of a neighbouring `gpt-5.1-codex-mini` row,
    // which the lookup path deliberately reports as not submission-safe.
    // Archiving it would promote that guess to authoritative and let it
    // overwrite real cost on the server. Leaving it unpriced is the intended
    // outcome: its date joins `incomplete_cost_dates` and the previously stored
    // cost is preserved.
    fn build_archive_overrides() -> HashMap<String, ModelPricing> {
        /// `(model id, input, output, cache read, cache creation)`, per token.
        /// Cache creation is `None` where the vendor publishes no such bucket
        /// (Zhipu, Xiaomi and Tencent document cached-input reads but no
        /// cache-write tariff): usage populating that bucket then stays
        /// unpriced rather than billed at an invented rate.
        type ArchivedRateRow = (&'static str, f64, f64, f64, Option<f64>);

        // Every first-party Claude row the narrowest dataset still carries, so
        // whichever one is retired next already has its last published rate
        // here. models.dev is that dataset: as of 2026-09-02 it lists
        // haiku-4-5, sonnet-4-5, sonnet-4-6, opus-4-5, opus-4-6, opus-4-7,
        // opus-4-8, opus-5, sonnet-5 and the two fable-5 rows. The current
        // flagships (opus-5, sonnet-5, fable-5*) are left out on purpose --
        // nothing retires the model that is shipping.
        let entries: &[ArchivedRateRow] = &[
            // Claude Haiku 4.5: $1.00/$5.00 per 1M, $0.10 cache read, $1.25 cache write.
            (
                "anthropic/claude-haiku-4-5",
                1e-6,
                5e-6,
                1e-7,
                Some(1.25e-6),
            ),
            // Claude Sonnet 4.5 / 4.6: $3.00/$15.00 per 1M, $0.30 cache read,
            // $3.75 cache write. 4.5 is the oldest Sonnet any of the three
            // still carries.
            (
                "anthropic/claude-sonnet-4-5",
                3e-6,
                1.5e-5,
                3e-7,
                Some(3.75e-6),
            ),
            (
                "anthropic/claude-sonnet-4-6",
                3e-6,
                1.5e-5,
                3e-7,
                Some(3.75e-6),
            ),
            // Claude Opus 4.5 / 4.6 / 4.7 / 4.8: $5.00/$25.00 per 1M, $0.50
            // cache read, $6.25 cache write. 4.5 is the oldest Opus carried by
            // all three, so it is the next one due to fall off -- LiteLLM and
            // OpenRouter still list opus-4 and opus-4-1 (and LiteLLM
            // claude-3-opus), but models.dev has already dropped them, so
            // their rates cannot be cross-checked and they are not archived.
            (
                "anthropic/claude-opus-4-5",
                5e-6,
                2.5e-5,
                5e-7,
                Some(6.25e-6),
            ),
            (
                "anthropic/claude-opus-4-6",
                5e-6,
                2.5e-5,
                5e-7,
                Some(6.25e-6),
            ),
            (
                "anthropic/claude-opus-4-7",
                5e-6,
                2.5e-5,
                5e-7,
                Some(6.25e-6),
            ),
            (
                "anthropic/claude-opus-4-8",
                5e-6,
                2.5e-5,
                5e-7,
                Some(6.25e-6),
            ),
            // Zhipu GLM-5.2 / GLM-5.3: $1.40/$4.40 per 1M, $0.26 cached
            // input, no published cache-write tariff (cached-input storage
            // is "Limited-time Free"). No live dataset carries a first-party
            // row -- only the `z-ai/*` reseller keys -- so a `zhipu` hint
            // resolved as an unverified guess. Verified 2026-09-03 against
            // https://docs.z.ai/guides/overview/pricing ("Latest Models":
            // GLM-5.3 and GLM-5.2 at $1.4 in / $0.26 cached / $4.4 out) and
            // https://bigmodel.cn/pricing (8元 in / 28元 out / 2元
            // cache-hit per M tokens, flat 1M context, no tiers).
            ("zhipu/glm-5.2", 1.4e-6, 4.4e-6, 0.26e-6, None),
            ("zhipu/glm-5.3", 1.4e-6, 4.4e-6, 0.26e-6, None),
            ("zai/glm-5.2", 1.4e-6, 4.4e-6, 0.26e-6, None),
            ("zai/glm-5.3", 1.4e-6, 4.4e-6, 0.26e-6, None),
            // Xiaomi MiMo-V2.5: $0.14/$0.28 per 1M, $0.0028 cached input,
            // no published cache-write tariff (only cache-hit vs cache-miss
            // input). No live dataset carries a first-party row -- only the
            // `openrouter/xiaomi/*` marketplace key -- so a `xiaomi` hint
            // resolved as an unverified guess. Verified 2026-09-03 against
            // https://mimo.mi.com/docs/en-US/pricing (MiMo-V2.5: $0.0028
            // cache-hit / $0.14 cache-miss input / $0.28 output per MTok,
            // flat to 1M context since the 2026-05-27 permanent cut).
            ("xiaomi/mimo-v2.5", 0.14e-6, 0.28e-6, 0.0028e-6, None),
            ("mimo/mimo-v2.5", 0.14e-6, 0.28e-6, 0.0028e-6, None),
            // Tencent Hunyuan HY3: $0.132/$0.528 per 1M, $0.033 cache-hit;
            // HY4-Preview: $0.834/$2.501 per 1M, $0.042 cache-hit. Neither
            // publishes a separate cache-write tariff. No live dataset
            // carries a first-party row -- only `deepinfra/tencent/*` and
            // `crossmodel/tencent/*` reseller keys -- so a `tencent` hint
            // resolved as an unverified guess. Verified 2026-09-03 against
            // https://cloud.tencent.com/document/product/1823/130055
            // (Hy3: 1元 in / 4元 out / 0.25元 cache-hit; Hy4 preview: 6元
            // in / 18元 out / 0.3元 cache-hit per M tokens, flat) and the
            // Intl sheet https://intl.cloud.tencent.com/document/product/1300/78937
            // (USD numbers archived here, matching the datasets' currency).
            ("tencent/hy3", 0.132e-6, 0.528e-6, 0.033e-6, None),
            ("tencent/hy4-preview", 0.834e-6, 2.501e-6, 0.042e-6, None),
            ("hunyuan/hy3", 0.132e-6, 0.528e-6, 0.033e-6, None),
            ("hunyuan/hy4-preview", 0.834e-6, 2.501e-6, 0.042e-6, None),
        ];

        let mut overrides = HashMap::with_capacity(entries.len());
        for (model_id, input, output, cache_read, cache_creation) in entries {
            overrides.insert(
                model_id.to_string(),
                ModelPricing {
                    input_cost_per_token: Some(*input),
                    output_cost_per_token: Some(*output),
                    cache_read_input_token_cost: Some(*cache_read),
                    cache_creation_input_token_cost: *cache_creation,
                    ..Default::default()
                },
            );
        }
        overrides
    }

    async fn fetch_inner() -> Result<Self, String> {
        let (litellm_result, openrouter_data, models_dev_result) = tokio::join!(
            litellm::fetch(),
            openrouter::fetch_all_mapped(),
            models_dev::fetch()
        );

        Self::combine_fetched_sources(
            litellm_result,
            openrouter_data,
            models_dev_result,
            litellm::load_cached_any_age,
            openrouter::load_cached_any_age,
            models_dev::load_cached_any_age,
            CustomPricing::load_from_default_path(),
        )
    }

    /// Degrade one failed source to its own stale cache, else to nothing.
    fn degrade_source(
        label: &str,
        result: Result<HashMap<String, ModelPricing>, String>,
        cached: impl FnOnce() -> Option<HashMap<String, ModelPricing>>,
    ) -> HashMap<String, ModelPricing> {
        match result {
            Ok(data) => data,
            Err(error) => {
                let cached = cached();
                eprintln!(
                    "[tokscale] Warning: {} pricing fetch failed ({}); {}",
                    label,
                    error,
                    if cached.is_some() {
                        "falling back to the cached copy"
                    } else {
                        "continuing with the remaining pricing sources"
                    }
                );
                cached.unwrap_or_default()
            }
        }
    }

    // @keep: the asymmetry this removes was load-bearing and non-obvious.
    /// Assemble a service from whatever the three upstream sources returned.
    ///
    /// No single source may be fatal. LiteLLM is the largest dataset, but it is
    /// not the only one, and propagating its fetch error made every command
    /// that prices tokens — `submit` included — dead in the water whenever
    /// raw.githubusercontent.com was unreachable or served something we could
    /// not decode (#1002). Every dynamic source now preserves fetch failure as
    /// an error here, degrades to its own stale cache, and finally to nothing;
    /// the surviving sources still price what they cover. Submission safety is
    /// checked against the actual filtered messages later, rather than treating
    /// an empty dynamic dataset as a construction failure: custom and bundled
    /// pricing remain useful during an outage.
    fn combine_fetched_sources(
        litellm_result: Result<HashMap<String, ModelPricing>, String>,
        openrouter_result: Result<HashMap<String, ModelPricing>, String>,
        models_dev_result: Result<HashMap<String, ModelPricing>, String>,
        litellm_cached: impl FnOnce() -> Option<HashMap<String, ModelPricing>>,
        openrouter_cached: impl FnOnce() -> Option<HashMap<String, ModelPricing>>,
        models_dev_cached: impl FnOnce() -> Option<HashMap<String, ModelPricing>>,
        custom: CustomPricing,
    ) -> Result<Self, String> {
        let litellm_data = Self::filter_litellm_data(Self::degrade_source(
            "LiteLLM",
            litellm_result,
            litellm_cached,
        ));
        let models_dev_data =
            Self::degrade_source("models.dev", models_dev_result, models_dev_cached);
        let openrouter_data =
            Self::degrade_source("OpenRouter", openrouter_result, openrouter_cached);

        Ok(Self::new_with_custom_and_models_dev(
            custom,
            litellm_data,
            openrouter_data,
            models_dev_data,
        ))
    }

    fn from_cached_datasets(
        litellm_data: Option<HashMap<String, ModelPricing>>,
        openrouter_data: Option<HashMap<String, ModelPricing>>,
        models_dev_data: Option<HashMap<String, ModelPricing>>,
    ) -> Option<Self> {
        if litellm_data.is_none() && openrouter_data.is_none() && models_dev_data.is_none() {
            return None;
        }

        Some(Self::new_with_custom_and_models_dev(
            CustomPricing::load_from_default_path(),
            Self::filter_litellm_data(litellm_data.unwrap_or_default()),
            openrouter_data.unwrap_or_default(),
            models_dev_data.unwrap_or_default(),
        ))
    }

    /// True when this service holds pricing from at least one source that can
    /// fail to load — the three fetchable upstreams, or the user's
    /// `custom-pricing.json`.
    ///
    /// Mirrors the signal `from_cached_datasets` uses to decide a cached
    /// service is worth building at all. Callers that must distinguish "this
    /// model has no published price" from "no pricing dataset loaded" need
    /// this, because an empty service answers `false` to every coverage
    /// question and is otherwise indistinguishable from healthy pricing that
    /// happens not to cover the model in hand.
    pub fn has_pricing_data(&self) -> bool {
        !self.custom.is_empty() || self.lookup.has_upstream_dataset()
    }

    pub fn load_cached_any_age() -> Option<Self> {
        Self::from_cached_datasets(
            litellm::load_cached_any_age(),
            openrouter::load_cached_any_age(),
            models_dev::load_cached_any_age(),
        )
    }

    pub async fn get_or_init() -> Result<Arc<PricingService>, String> {
        PRICING_SERVICE
            .get_or_try_init(|| async { Self::fetch_inner().await.map(Arc::new) })
            .await
            .map(Arc::clone)
    }

    pub fn lookup_with_source(
        &self,
        model_id: &str,
        force_source: Option<&str>,
    ) -> Option<LookupResult> {
        match force_source {
            Some(source) if source.eq_ignore_ascii_case("custom") => {
                return self.lookup_custom(model_id);
            }
            None => {
                if let Some(result) = self.lookup_custom(model_id) {
                    return Some(result);
                }
            }
            Some(_) => {}
        }

        self.lookup.lookup_with_source(model_id, force_source)
    }

    pub fn lookup_with_source_and_provider(
        &self,
        model_id: &str,
        force_source: Option<&str>,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        match force_source {
            Some(source) if source.eq_ignore_ascii_case("custom") => {
                return self.lookup_custom(model_id);
            }
            None => {
                if let Some(result) = self.lookup_custom(model_id) {
                    return Some(result);
                }
            }
            Some(_) => {}
        }

        self.lookup
            .lookup_with_source_and_provider(model_id, force_source, provider_id)
    }

    pub fn calculate_cost(
        &self,
        model_id: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
    ) -> f64 {
        let usage = TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
        };
        self.calculate_cost_with_provider(model_id, None, &usage)
    }

    pub fn calculate_cost_with_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
        usage: &TokenBreakdown,
    ) -> f64 {
        if let Some(result) = self.custom.lookup_with_key(model_id) {
            return compute_cost(
                result.pricing,
                usage.input,
                usage.output,
                usage.cache_read,
                usage.cache_write,
                usage.reasoning,
            );
        }

        self.lookup
            .calculate_cost_with_provider(model_id, provider_id, usage)
    }

    pub fn covers_usage_with_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
        usage: &TokenBreakdown,
    ) -> bool {
        self.resolve_for_usage_with_provider(model_id, provider_id, usage)
            .is_some_and(|result| {
                result.evidence.is_submission_safe() && result.pricing.covers_usage(usage)
            })
    }

    /// Resolve the exact pricing evidence used to judge this usage for a
    /// leaderboard submission.
    ///
    /// This differs from a plain lookup when a provider-scoped row borrows a
    /// missing bucket rate from the canonical row. Callers that explain a
    /// rejection must inspect this composed result, otherwise they can report
    /// an incomplete price while the real blocker is the donor row's unsafe
    /// model or price evidence.
    pub(crate) fn resolve_for_usage_with_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
        usage: &TokenBreakdown,
    ) -> Option<LookupResult> {
        if let Some(result) = self.lookup_custom(model_id) {
            return Some(result);
        }

        self.lookup.resolve_for_usage(model_id, provider_id, usage)
    }

    fn lookup_custom(&self, model_id: &str) -> Option<LookupResult> {
        self.custom
            .lookup_with_key(model_id)
            .map(|result| LookupResult {
                pricing: result.pricing.clone(),
                source: "Custom".into(),
                matched_key: result.matched_key.to_string(),
                evidence: ResolutionEvidence {
                    kind: ResolutionKind::Custom,
                    candidate_count: 1,
                    price_consensus: true,
                    exact_model_identity: true,
                    alias_applied: false,
                    // A custom entry can be reached through synthetic-model
                    // normalization, which is exactly the case provenance has
                    // to disclose: the matched key is not the requested id.
                    normalized: result.normalized,
                    stripped: false,
                    // A custom entry is the user stating a rate for their own
                    // id, not a dataset namespace being read as a price sheet.
                    subscription_namespace: false,
                },
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_pricing(input: f64, output: f64) -> ModelPricing {
        ModelPricing {
            input_cost_per_token: Some(input),
            output_cost_per_token: Some(output),
            ..Default::default()
        }
    }

    fn custom_service(
        custom: HashMap<String, ModelPricing>,
        litellm: HashMap<String, ModelPricing>,
        openrouter: HashMap<String, ModelPricing>,
    ) -> PricingService {
        PricingService::new_with_custom(CustomPricing::from_models(custom), litellm, openrouter)
    }

    fn fixture_models_dev() -> HashMap<String, ModelPricing> {
        models_dev::parse_dataset(include_str!("../../tests/fixtures/models_dev_pricing.json"))
            .unwrap()
    }

    fn custom_service_with_models_dev(
        custom: HashMap<String, ModelPricing>,
        litellm: HashMap<String, ModelPricing>,
        openrouter: HashMap<String, ModelPricing>,
        models_dev: HashMap<String, ModelPricing>,
    ) -> PricingService {
        PricingService::new_with_custom_and_models_dev(
            CustomPricing::from_models(custom),
            litellm,
            openrouter,
            models_dev,
        )
    }

    fn all_bucket_usage() -> TokenBreakdown {
        TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            reasoning: 0,
        }
    }

    fn assert_retired_anthropic_price(
        model: &str,
        input: f64,
        output: f64,
        cache_read: f64,
        cache_write: f64,
    ) {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = all_bucket_usage();
        let resolved = service
            .resolve_for_usage_with_provider(model, Some("anthropic"), &usage)
            .unwrap_or_else(|| panic!("{model} must retain its archived price"));
        assert_eq!(resolved.source, "Tokscale Archive", "model: {model}");
        assert!(resolved.evidence.is_submission_safe(), "model: {model}");

        let expected = input + output + cache_read + cache_write;
        let actual = service.calculate_cost_with_provider(model, Some("anthropic"), &usage);
        assert!(
            (actual - expected).abs() < 1e-9,
            "model: {model}, expected {expected}, got {actual}"
        );
    }

    #[test]
    fn retired_claude_haiku_4_5_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-haiku-4-5", 1.0, 5.0, 0.1, 1.25);
    }

    #[test]
    fn retired_claude_sonnet_4_5_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-sonnet-4-5", 3.0, 15.0, 0.3, 3.75);
    }

    #[test]
    fn retired_claude_opus_4_5_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-opus-4-5", 5.0, 25.0, 0.5, 6.25);
    }

    #[test]
    fn retired_claude_opus_4_6_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-opus-4-6", 5.0, 25.0, 0.5, 6.25);
    }

    #[test]
    fn retired_claude_opus_4_7_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-opus-4-7", 5.0, 25.0, 0.5, 6.25);
    }

    #[test]
    fn retired_claude_opus_4_8_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-opus-4-8", 5.0, 25.0, 0.5, 6.25);
    }

    /// Zhipu publishes no first-party row to any live dataset -- only the
    /// `z-ai/*` reseller keys -- so a `zhipu` hint resolved as an unverified
    /// guess. The archived first-party tariff ($1.40/$4.40 per 1M, $0.26
    /// cached input, verified 2026-09-03) wins for the publishing endpoint.
    #[test]
    fn zhipu_glm_rates_resolve_first_party_over_reseller_row() {
        fn reseller_row() -> ModelPricing {
            ModelPricing {
                input_cost_per_token: Some(1.4e-6),
                output_cost_per_token: Some(4.4e-6),
                cache_read_input_token_cost: Some(0.26e-6),
                ..Default::default()
            }
        }

        for model in ["glm-5.2", "glm-5.3"] {
            let mut litellm = HashMap::new();
            litellm.insert(format!("openrouter/z-ai/{model}"), reseller_row());
            let service = PricingService::new(litellm, HashMap::new());
            // No published cache-write tariff, so the covered usage carries
            // none; cache-write-bearing usage stays unpriced (pinned below).
            let usage = TokenBreakdown {
                input: 1_000_000,
                output: 1_000_000,
                cache_read: 1_000_000,
                cache_write: 0,
                reasoning: 0,
            };

            for (provider, test_model, expected_key) in [
                (Some("zhipu"), model.to_string(), format!("zhipu/{model}")),
                (None, format!("zhipu/{model}"), format!("zhipu/{model}")),
                (Some("zai"), model.to_string(), format!("zai/{model}")),
                (None, format!("zai/{model}"), format!("zai/{model}")),
            ] {
                let resolved = service
                    .resolve_for_usage_with_provider(&test_model, provider, &usage)
                    .unwrap_or_else(|| panic!("{test_model} must resolve first-party"));
                assert_eq!(resolved.source, "Tokscale Archive", "model: {test_model}");
                assert_eq!(
                    resolved.matched_key, expected_key,
                    "model: {test_model}, unexpected matched_key"
                );
                assert!(
                    resolved.evidence.is_submission_safe(),
                    "model: {test_model}"
                );
                assert!(
                    service.covers_usage_with_provider(&test_model, provider, &usage),
                    "model: {test_model}"
                );
                let actual = service.calculate_cost_with_provider(&test_model, provider, &usage);
                assert!(
                    (actual - 6.06).abs() < 1e-9,
                    "model: {test_model}, unexpected cost: {actual}"
                );
            }
        }
    }

    /// Zhipu documents cached-input reads but no cache-write tariff, so
    /// cache-write-bearing usage must stay unpriced rather than billed at an
    /// invented rate.
    #[test]
    fn zhipu_cache_write_usage_stays_unpriced() {
        let service = PricingService::new(HashMap::new(), HashMap::new());

        for model in ["glm-5.2", "glm-5.3"] {
            assert!(
                !service.covers_usage_with_provider(model, Some("zhipu"), &all_bucket_usage()),
                "model: {model}"
            );
        }
    }

    /// Xiaomi publishes no first-party row to any live dataset -- only the
    /// `openrouter/xiaomi/*` marketplace key -- so a `xiaomi` hint for
    /// `mimo-v2.5` resolved as an unverified guess. The archived first-party
    /// tariff ($0.14/$0.28 per 1M, $0.0028 cached input, verified 2026-09-03)
    /// wins for the publishing endpoint.
    #[test]
    fn xiaomi_mimo_rate_resolves_first_party_over_marketplace_row() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openrouter/xiaomi/mimo-v2.5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.14e-6),
                output_cost_per_token: Some(0.28e-6),
                cache_read_input_token_cost: Some(0.0028e-6),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        // No published cache-write tariff, so the covered usage carries none.
        let usage = TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        };

        for (provider, model_id, expected_key) in [
            (Some("xiaomi"), "mimo-v2.5", "xiaomi/mimo-v2.5"),
            (Some("mimo"), "mimo-v2.5", "mimo/mimo-v2.5"),
            (None, "xiaomi/mimo-v2.5", "xiaomi/mimo-v2.5"),
            (None, "mimo/mimo-v2.5", "mimo/mimo-v2.5"),
        ] {
            let resolved = service
                .resolve_for_usage_with_provider(model_id, provider, &usage)
                .expect("mimo-v2.5 must resolve first-party");
            assert_eq!(resolved.source, "Tokscale Archive");
            assert_eq!(resolved.matched_key, expected_key, "unexpected matched key");
            assert!(resolved.evidence.is_submission_safe());
            assert!(service.covers_usage_with_provider(model_id, provider, &usage));
            let actual = service.calculate_cost_with_provider(model_id, provider, &usage);
            assert!((actual - 0.4228).abs() < 1e-9, "unexpected cost: {actual}");
            assert!(
                !service.covers_usage_with_provider(model_id, provider, &all_bucket_usage()),
                "cache-write-bearing usage must stay unpriced: no published tariff"
            );
        }
    }

    /// Tencent publishes no first-party row to any live dataset -- only
    /// `deepinfra/tencent/*` and `crossmodel/tencent/*` reseller keys -- so a
    /// `tencent` hint for `hy3` / `hy4-preview` resolved as an unverified
    /// guess. The archived first-party tariffs (Intl USD sheet, verified
    /// 2026-09-03) win for the publishing endpoint. Each case mirrors the
    /// live reseller shape, exercising the LiteLLM path (hy3) and the
    /// models.dev path (hy4-preview).
    #[test]
    fn tencent_hunyuan_rates_resolve_first_party_over_reseller_rows() {
        fn usage_without_cache_write() -> TokenBreakdown {
            TokenBreakdown {
                input: 1_000_000,
                output: 1_000_000,
                cache_read: 1_000_000,
                cache_write: 0,
                reasoning: 0,
            }
        }

        let mut litellm = HashMap::new();
        litellm.insert(
            "deepinfra/tencent/Hy3".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.132e-6),
                output_cost_per_token: Some(0.528e-6),
                cache_read_input_token_cost: Some(0.033e-6),
                ..Default::default()
            },
        );
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "crossmodel/tencent/hy4-preview".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.834e-6),
                output_cost_per_token: Some(2.501e-6),
                cache_read_input_token_cost: Some(0.042e-6),
                ..Default::default()
            },
        );
        let service = PricingService::new_with_custom_and_models_dev(
            CustomPricing::default(),
            litellm,
            HashMap::new(),
            models_dev,
        );

        for (model, expected_cost) in [("hy3", 0.693), ("hy4-preview", 3.377)] {
            for (test_model, provider, expected_key) in [
                (
                    model.to_string(),
                    Some("tencent"),
                    format!("tencent/{model}"),
                ),
                (
                    model.to_string(),
                    Some("hunyuan"),
                    format!("hunyuan/{model}"),
                ),
                (format!("tencent/{model}"), None, format!("tencent/{model}")),
                (format!("hunyuan/{model}"), None, format!("hunyuan/{model}")),
            ] {
                let usage = usage_without_cache_write();
                let resolved = service
                    .resolve_for_usage_with_provider(&test_model, provider, &usage)
                    .unwrap_or_else(|| panic!("{test_model} must resolve first-party"));
                assert_eq!(resolved.source, "Tokscale Archive", "model: {test_model}");
                assert_eq!(
                    resolved.matched_key, expected_key,
                    "model: {test_model}, unexpected matched_key"
                );
                assert!(
                    resolved.evidence.is_submission_safe(),
                    "model: {test_model}"
                );
                assert!(
                    service.covers_usage_with_provider(&test_model, provider, &usage),
                    "model: {test_model}"
                );
                let actual = service.calculate_cost_with_provider(&test_model, provider, &usage);
                assert!(
                    (actual - expected_cost).abs() < 1e-9,
                    "model: {test_model}, unexpected cost: {actual}"
                );
                assert!(
                    !service.covers_usage_with_provider(&test_model, provider, &all_bucket_usage()),
                    "model: {test_model}, cache-write-bearing usage must stay unpriced"
                );
            }
        }
    }

    #[test]
    fn retired_claude_sonnet_4_6_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-sonnet-4-6", 3.0, 15.0, 0.3, 3.75);
    }

    /// A hosted endpoint publishes its own tariff, so an Anthropic-keyed
    /// archive row may not stand in for one.
    ///
    /// Bedrock shares no provider tag with an `anthropic/` key, so the archive
    /// never answers for it at all -- which is the correct outcome, because the
    /// live datasets price Bedrock's regional Claude rows 10% above first-party
    /// and its GovCloud rows 20% above.
    #[test]
    fn retired_anthropic_price_is_not_offered_to_bedrock() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = all_bucket_usage();

        assert!(service
            .resolve_for_usage_with_provider("claude-opus-4-8", Some("bedrock"), &usage)
            .is_none());
        assert!(!service.covers_usage_with_provider("claude-opus-4-8", Some("bedrock"), &usage));
    }

    /// An embedded provider root authorises the tariff exactly as a hint does.
    ///
    /// A reseller root is not first-party Anthropic, so the archive refuses it
    /// on the qualified id. The outer resolver then peels the unknown vendor
    /// prefix and retries the bare model, which the archive does answer -- but
    /// with nothing naming the publishing endpoint it is ModelPart evidence, so
    /// it is priced for display and never submission-safe.
    #[test]
    fn reseller_rooted_id_gets_an_archive_estimate_but_not_submission_safety() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = all_bucket_usage();

        let resold = service
            .resolve_for_usage_with_provider("openrouter/anthropic/claude-haiku-4.5", None, &usage)
            .expect("the peeled bare id still carries an estimate");
        assert_eq!(resold.source, "Tokscale Archive");
        assert_eq!(
            resold.evidence.submission_safety_gap(),
            Some(lookup::SubmissionSafetyGap::UnverifiedProviderIdentity),
            "a reseller-rooted id must not be submission-safe"
        );

        // The same id under its own root is submission-safe, so the gap above
        // is the reseller segment and not a normalization failure.
        let first_party = service
            .resolve_for_usage_with_provider("anthropic/claude-haiku-4.5", None, &usage)
            .expect("the first-party root must still resolve");
        assert_eq!(first_party.source, "Tokscale Archive");
        assert_eq!(first_party.matched_key, "anthropic/claude-haiku-4-5");
        assert!(first_party.evidence.is_submission_safe());
    }

    /// Vertex canonicalises to `anthropic`, so the archive stays reachable as an
    /// estimate -- but only as an estimate. Reaching a first-party row through a
    /// cross-provider alias proves nothing about what Vertex billed, so the
    /// result must not be submission-safe.
    #[test]
    fn retired_anthropic_price_reaches_vertex_only_as_an_estimate() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = all_bucket_usage();

        for provider in ["vertex", "vertex_ai", "Vertex"] {
            let resolved = service
                .resolve_for_usage_with_provider("claude-opus-4-7", Some(provider), &usage)
                .unwrap_or_else(|| panic!("{provider} should still receive an estimate"));
            assert_eq!(resolved.source, "Tokscale Archive", "provider: {provider}");
            assert_eq!(
                resolved.evidence.submission_safety_gap(),
                Some(lookup::SubmissionSafetyGap::UnverifiedProviderIdentity),
                "provider: {provider} must not be submission-safe"
            );
        }
    }

    #[test]
    fn live_upstream_price_wins_over_the_retired_model_archive() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "anthropic/claude-opus-4-8".to_string(),
            ModelPricing {
                input_cost_per_token: Some(4e-6),
                output_cost_per_token: Some(20e-6),
                cache_read_input_token_cost: Some(4e-7),
                cache_creation_input_token_cost: Some(5e-6),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = all_bucket_usage();

        let resolved = service
            .resolve_for_usage_with_provider("claude-opus-4-8", Some("anthropic"), &usage)
            .expect("live upstream row must resolve");
        assert_eq!(resolved.source, "LiteLLM");
        assert!(
            (service.calculate_cost_with_provider("claude-opus-4-8", Some("anthropic"), &usage,)
                - 29.4)
                .abs()
                < 1e-9
        );
    }

    /// A reseller row that only matches by model part must not shadow the
    /// publishing endpoint's own archived tariff.
    ///
    /// Live LiteLLM keys this as `deepinfra/anthropic/claude-opus-4-8` -- a
    /// nested provider segment, not a first-party root -- so an `anthropic`
    /// hint resolves it as an unverified ModelPart guess while the archive
    /// holds `anthropic/claude-opus-4-8` exactly. The guess used to win by
    /// precedence and the usage submitted at $0.00 with a cost-incomplete
    /// day; the proven row wins now.
    #[test]
    fn unsafe_reseller_row_yields_to_proven_archive() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "deepinfra/anthropic/claude-opus-4-8".to_string(),
            ModelPricing {
                input_cost_per_token: Some(5e-6),
                output_cost_per_token: Some(2.5e-5),
                cache_read_input_token_cost: Some(5e-7),
                cache_creation_input_token_cost: Some(6.25e-6),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = all_bucket_usage();

        let resolved = service
            .resolve_for_usage_with_provider("claude-opus-4-8", Some("anthropic"), &usage)
            .expect("proven archive row must resolve");
        assert_eq!(resolved.source, "Tokscale Archive");
        assert_eq!(resolved.matched_key, "anthropic/claude-opus-4-8");
        assert!(resolved.evidence.is_submission_safe());
        assert!(service.covers_usage_with_provider("claude-opus-4-8", Some("anthropic"), &usage));
        let actual =
            service.calculate_cost_with_provider("claude-opus-4-8", Some("anthropic"), &usage);
        assert!((actual - 36.75).abs() < 1e-9, "unexpected cost: {actual}");
    }

    /// The shared normalizer folds reasoning-effort suffixes, so the archive
    /// needs no suffix stripper of its own.
    #[test]
    fn retired_claude_reasoning_suffix_uses_the_archived_base_price() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = all_bucket_usage();

        let resolved = service
            .resolve_for_usage_with_provider(
                "claude-opus-4-7-thinking-xhigh",
                Some("anthropic"),
                &usage,
            )
            .expect("reasoning suffix must retain the base model price");
        assert_eq!(resolved.source, "Tokscale Archive");
        assert_eq!(resolved.matched_key, "anthropic/claude-opus-4-7");
    }

    /// Regression: the archive may only carry published rates.
    ///
    /// No dataset publishes a tariff for `codex-auto-review` -- LiteLLM,
    /// OpenRouter and models.dev all carry zero `auto-review` keys. Its price
    /// on this path has always been a fuzzy election of a neighbouring
    /// `gpt-5.1-codex-mini` row, which the lookup reports as an estimate.
    /// Archiving that number would stamp it authoritative and let a guess
    /// overwrite real recorded cost on the server, so no such row exists.
    #[test]
    fn codex_auto_review_is_never_served_from_the_archive() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        };

        let resolved =
            service.resolve_for_usage_with_provider("codex-auto-review", Some("openai"), &usage);
        assert!(
            resolved
                .as_ref()
                .is_none_or(|result| result.source != "Tokscale Archive"),
            "codex-auto-review has no published rate to archive"
        );
        assert!(
            resolved.is_none_or(|result| !result.evidence.is_submission_safe()),
            "an estimated price must never become submission-safe"
        );
    }

    /// Regression: real clients emit dated and suffixed ids, so an archive
    /// matched by bare-string equality would silently miss every model it
    /// exists to cover. The archive must go through the same normalization the
    /// live lookup path applies.
    #[test]
    fn archived_price_survives_the_id_spellings_clients_actually_emit() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = all_bucket_usage();

        for (model, expected_key) in [
            // Anthropic's dated release ids.
            ("claude-haiku-4-5-20251001", "anthropic/claude-haiku-4-5"),
            ("claude-sonnet-4-6-20260115", "anthropic/claude-sonnet-4-6"),
            // Long-context and reasoning-effort suffixes.
            ("claude-opus-4-8[1m]", "anthropic/claude-opus-4-8"),
            (
                "claude-opus-4-7-thinking-xhigh",
                "anthropic/claude-opus-4-7",
            ),
            // Dotted minor versions and an embedded provider root.
            ("claude-haiku-4.5", "anthropic/claude-haiku-4-5"),
            ("anthropic/claude-haiku-4.5", "anthropic/claude-haiku-4-5"),
        ] {
            let resolved = service
                .resolve_for_usage_with_provider(model, Some("anthropic"), &usage)
                .unwrap_or_else(|| panic!("{model} must retain its archived price"));
            assert_eq!(resolved.source, "Tokscale Archive", "model: {model}");
            assert_eq!(resolved.matched_key, expected_key, "model: {model}");
            assert!(resolved.evidence.is_submission_safe(), "model: {model}");
        }
    }

    /// The same Grok request must cost the same whichever dataset priced it.
    ///
    /// xAI documents the >200K rate as request-wide. The upstream `xai/grok-*`
    /// row was billed that way while the built-in Cursor row -- transcribed
    /// from the same published tariff, and keyed bare as `grok-4.6` -- fell
    /// through to progressive billing, halving the cost of a long request.
    #[test]
    fn cursor_grok_long_context_bills_request_wide_like_the_upstream_row() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 150_000,
            output: 10_000,
            cache_read: 50_000,
            cache_write: 0,
            reasoning: 0,
        };

        let resolved = service
            .lookup_with_source_and_provider("grok-4.6", None, Some("xai"))
            .expect("the built-in Cursor row prices grok-4.6");
        assert_eq!(resolved.source, "Cursor");
        assert_eq!(resolved.matched_key, "grok-4.6");

        // 210K total crosses the boundary, so every bucket bills at the high
        // rate: 150k*4 + 10k*12 + 50k*1 per million.
        let cost = service.calculate_cost_with_provider("grok-4.6", Some("xai"), &usage);
        assert!(
            (cost - 0.770).abs() < 1e-9,
            "request-wide pricing expected, got {cost}"
        );
    }

    /// Without an xAI hint a bare `grok-*` says nothing about who billed it, so
    /// the request-wide rule must not apply.
    #[test]
    fn an_unhinted_bare_grok_row_keeps_progressive_pricing() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 150_000,
            output: 10_000,
            cache_read: 50_000,
            cache_write: 0,
            reasoning: 0,
        };
        let cost = service.calculate_cost_with_provider("grok-4.6", Some("openrouter"), &usage);
        assert!(
            (cost - 0.770).abs() > 1e-9,
            "an unrelated provider hint must not get request-wide pricing: {cost}"
        );
    }

    fn cache_read_usage() -> TokenBreakdown {
        TokenBreakdown {
            input: 1_000_000,
            output: 0,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        }
    }

    // Regression: #1013. Submission validation judged bucket coverage against
    // the provider-hinted row alone. For `openai/gpt-5.2-codex` the hint lands
    // on an OpenRouter row with no cache-read rate while the canonical LiteLLM
    // row publishes one, so every Codex session — which always carries cached
    // tokens — was reported as unpriced and aborted the whole submission.
    #[test]
    fn hinted_row_missing_a_cache_rate_still_covers_usage() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "azure/codex-cache-gap".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1.75e-6),
                output_cost_per_token: Some(1.4e-5),
                ..Default::default()
            },
        );
        litellm.insert(
            "codex-cache-gap".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1.75e-6),
                output_cost_per_token: Some(1.4e-5),
                cache_read_input_token_cost: Some(1.75e-7),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = cache_read_usage();

        assert!(service.covers_usage_with_provider("codex-cache-gap", Some("azure"), &usage));
        let cost = service.calculate_cost_with_provider("codex-cache-gap", Some("azure"), &usage);
        assert!((cost - 1.925).abs() < 1e-9, "unexpected cost: {cost}");
    }

    // Regression: #1021, #1035. The unit tests around `covers_usage` pin the
    // row-level rule; this pins the behaviour the issues actually reported,
    // which is a submission aborting. It has to run through `PricingService`
    // because the shortcut is only reached via `resolve_for_usage`, whose
    // `normalize_provider_hint(..).is_none() || covers_usage(..)` condition
    // decides whether a hinted row is consulted at all — reordering that
    // condition would reintroduce the aborted submission while every
    // row-level test kept passing.
    #[test]
    fn a_hinted_all_zero_row_covers_cache_usage_through_the_service() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "kenari/nemotron-free-fixture".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = cache_read_usage();

        assert!(
            service.covers_usage_with_provider("nemotron-free-fixture", Some("kenari"), &usage),
            "cache-bearing usage on an all-zero row must not abort the submission"
        );
        let cost =
            service.calculate_cost_with_provider("nemotron-free-fixture", Some("kenari"), &usage);
        assert_eq!(cost, 0.0, "an all-zero row must price at exactly zero");
    }

    // Regression: the provider-hint guard only covers the model-part promotion
    // path. An unaliased `kimi-for-coding/<model>` id IS its own dataset key,
    // so it resolves through the full-key exact branch and was handed
    // `ResolutionKind::Exact`, which is submission-safe by construction. The
    // row publishes explicit 0.0 rates, so `covers_usage` accepted every
    // bucket and real usage submitted to the leaderboard at $0.00. Runs
    // through `PricingService` because the lookup helper alone does not show
    // what submission validation actually asks.
    //
    // The plan rate stays visible for reporting; only its publishability
    // changes. A qualified `moonshotai/*` key in the same dataset must keep
    // its submission-safe exact evidence, so the rule cannot be "the key is
    // qualified" or "the rates are zero".
    #[test]
    fn an_unaliased_kimi_subscription_key_is_not_submission_safe() {
        let models_dev = HashMap::from([
            (
                "kimi-for-coding/k3-unlisted-fixture".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(0.0),
                    output_cost_per_token: Some(0.0),
                    cache_read_input_token_cost: Some(0.0),
                    ..Default::default()
                },
            ),
            (
                "moonshotai/k3-metered-fixture".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(1e-6),
                    output_cost_per_token: Some(2e-6),
                    cache_read_input_token_cost: Some(1e-7),
                    ..Default::default()
                },
            ),
        ]);
        let service = PricingService::new_with_custom_and_models_dev(
            CustomPricing::default(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );
        let usage = cache_read_usage();

        for hint in [Some("kimi_for_coding"), Some("moonshot"), None] {
            let subscription = service
                .resolve_for_usage_with_provider(
                    "kimi-for-coding/k3-unlisted-fixture",
                    hint,
                    &usage,
                )
                .expect("the plan rate stays visible for reporting");
            assert_eq!(
                subscription.matched_key, "kimi-for-coding/k3-unlisted-fixture",
                "hint: {hint:?}"
            );
            assert_eq!(
                subscription.evidence.submission_safety_gap(),
                Some(lookup::SubmissionSafetyGap::UnverifiedProviderIdentity),
                "hint: {hint:?}"
            );
            assert!(
                !subscription.evidence.is_submission_safe(),
                "hint: {hint:?}"
            );
            assert!(
                !service.covers_usage_with_provider(
                    "kimi-for-coding/k3-unlisted-fixture",
                    hint,
                    &usage
                ),
                "a subscription-plan row must not authorize a submission, hint: {hint:?}"
            );
            // Reporting keeps the plan rate. The row is excluded from the
            // leaderboard, not dropped from the user's own cost view.
            assert_eq!(
                service.calculate_cost_with_provider(
                    "kimi-for-coding/k3-unlisted-fixture",
                    hint,
                    &usage
                ),
                0.0,
                "hint: {hint:?}"
            );

            let metered = service
                .resolve_for_usage_with_provider("moonshotai/k3-metered-fixture", hint, &usage)
                .expect("Moonshot's own metered row must still price");
            assert_eq!(
                metered.matched_key, "moonshotai/k3-metered-fixture",
                "hint: {hint:?}"
            );
            assert_eq!(
                metered.evidence.submission_safety_gap(),
                None,
                "hint: {hint:?}"
            );
            assert!(
                service.covers_usage_with_provider("moonshotai/k3-metered-fixture", hint, &usage),
                "hint: {hint:?}"
            );
        }
    }

    #[test]
    fn reasonix_uses_the_inferred_upstream_provider_for_pricing() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "deepseek/reasonix-fixture".to_string(),
            ModelPricing {
                input_cost_per_token: Some(2e-6),
                output_cost_per_token: Some(8e-6),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = TokenBreakdown {
            input: 1_000,
            output: 1_000,
            ..Default::default()
        };

        assert!(service.covers_usage_with_provider(
            "opencode/reasonix-fixture",
            Some("deepseek"),
            &usage,
        ));
        assert!(
            (service.calculate_cost_with_provider(
                "opencode/reasonix-fixture",
                Some("deepseek"),
                &usage,
            ) - 0.01)
                .abs()
                < 1e-12
        );
    }

    // The two rows must be the same deal before one lends the other a rate.
    // `azure_ai/grok-code-fast-1` bills $3.50/$17.50 per million with no
    // cache-read rate while the canonical `xai/` row bills $0.20/$1.50 with
    // one; borrowing across them would invent an Azure-base, xAI-cache tariff
    // that neither provider charges.
    #[test]
    fn differently_priced_canonical_row_does_not_lend_its_cache_rate() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "azure/grok-tariff-guard".to_string(),
            ModelPricing {
                input_cost_per_token: Some(3.5e-6),
                output_cost_per_token: Some(1.75e-5),
                ..Default::default()
            },
        );
        litellm.insert(
            "grok-tariff-guard".to_string(),
            ModelPricing {
                input_cost_per_token: Some(2e-7),
                output_cost_per_token: Some(1.5e-6),
                cache_read_input_token_cost: Some(2e-8),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = cache_read_usage();

        assert!(
            !service.covers_usage_with_provider("grok-tariff-guard", Some("azure"), &usage),
            "a differently priced row must not make the usage look priceable"
        );
        let cost = service.calculate_cost_with_provider("grok-tariff-guard", Some("azure"), &usage);
        assert!(
            (cost - 3.5).abs() < 1e-9,
            "the reseller's own rates must be the only ones applied: {cost}"
        );
    }

    // Guard for the fix above: borrowing must never reach a bucket the hinted
    // row already prices, otherwise a reseller row (e.g. `azure_ai/` at a
    // markup over `xai/`) would silently reprice to the author's cheaper rate.
    #[test]
    fn covered_hinted_row_is_not_replaced_by_the_canonical_row() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "azure/marked-up-model".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1e-5),
                cache_read_input_token_cost: Some(1e-6),
                ..Default::default()
            },
        );
        litellm.insert(
            "marked-up-model".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1e-7),
                cache_read_input_token_cost: Some(1e-8),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = cache_read_usage();

        assert!(service.covers_usage_with_provider("marked-up-model", Some("azure"), &usage));
        let cost = service.calculate_cost_with_provider("marked-up-model", Some("azure"), &usage);
        assert!(
            (cost - 11.0).abs() < 1e-9,
            "reseller markup must survive: {cost}"
        );
    }

    // A model nothing can price must still be rejected, so submissions never
    // silently bill genuinely unknown usage at zero.
    #[test]
    fn usage_stays_uncovered_when_no_resolution_prices_the_bucket() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "azure/no-cache-anywhere".to_string(),
            model_pricing(1e-5, 1e-4),
        );
        litellm.insert("no-cache-anywhere".to_string(), model_pricing(1e-6, 1e-5));
        let service = PricingService::new(litellm, HashMap::new());

        assert!(!service.covers_usage_with_provider(
            "no-cache-anywhere",
            Some("azure"),
            &cache_read_usage()
        ));
    }

    #[test]
    fn self_hosted_llama_cpp_usage_is_covered_at_zero_cost() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 25,
            cache_write: 10,
            reasoning: 5,
        };

        for provider in ["llama.cpp", "llama.cpp_modal"] {
            assert!(service.covers_usage_with_provider(
                "Qwen3.8-27B-ABLITERATED-Q4_K_M-MTP-Q4_0",
                Some(provider),
                &usage,
            ));
            assert_eq!(
                service.calculate_cost_with_provider(
                    "Qwen3.8-27B-ABLITERATED-Q4_K_M-MTP-Q4_0",
                    Some(provider),
                    &usage,
                ),
                0.0,
            );
        }
    }

    // Custom overrides are exact-only and provider-agnostic, so they must be
    // consulted before any provider-hinted resolution or bucket borrowing.
    #[test]
    fn custom_pricing_decides_coverage_before_any_fallback() {
        let mut custom = HashMap::new();
        custom.insert(
            "custom-covered-model".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1e-6),
                cache_read_input_token_cost: Some(1e-7),
                ..Default::default()
            },
        );
        let service = custom_service(custom, HashMap::new(), HashMap::new());

        assert!(service.covers_usage_with_provider(
            "custom-covered-model",
            Some("azure"),
            &cache_read_usage()
        ));
    }

    // Regression: #1002. A LiteLLM fetch failure used to propagate out of
    // fetch_inner, so `tokscale submit` died with "error decoding response
    // body" even though models.dev and openrouter were both reachable and
    // carried usable pricing.
    #[test]
    fn litellm_fetch_failure_is_not_fatal_when_another_source_has_data() {
        let mut models_dev = HashMap::new();
        models_dev.insert("test-model-alpha".to_string(), model_pricing(1e-6, 2e-6));

        let service = PricingService::combine_fetched_sources(
            Err("error decoding response body".to_string()),
            Err("OpenRouter unavailable".to_string()),
            Ok(models_dev),
            // Fresh install, as in the report: nothing cached yet.
            || None,
            || None,
            || None,
            CustomPricing::default(),
        )
        .expect("a LiteLLM failure must not be fatal while another source has pricing");

        let cost = service.calculate_cost("test-model-alpha", 1_000_000, 0, 0, 0, 0);
        assert!(
            (cost - 1.0).abs() < 1e-9,
            "models.dev pricing should still resolve after LiteLLM fails, got {}",
            cost
        );
    }

    // Regression: #1002. The reporter's workaround was hand-populating the
    // cache file. A cached copy older than the 1h TTL must be preferred over
    // dropping LiteLLM entirely, so that workaround keeps working unattended.
    #[test]
    fn litellm_fetch_failure_falls_back_to_stale_cache() {
        let mut cached = HashMap::new();
        cached.insert("test-model-beta".to_string(), model_pricing(3e-6, 4e-6));

        let service = PricingService::combine_fetched_sources(
            Err("error decoding response body".to_string()),
            Err("OpenRouter unavailable".to_string()),
            Ok(HashMap::new()),
            || Some(cached),
            || None,
            || None,
            CustomPricing::default(),
        )
        .expect("a stale LiteLLM cache must keep the service usable");

        let cost = service.calculate_cost("test-model-beta", 1_000_000, 0, 0, 0, 0);
        assert!(
            (cost - 3.0).abs() < 1e-9,
            "stale LiteLLM cache should price the model, got {}",
            cost
        );
    }

    // Regression: models.dev is a degradable source too. Its errors used to be
    // dropped straight to an empty map even though it keeps a cache of its own,
    // so a models.dev outage discarded pricing that was sitting on disk.
    #[test]
    fn models_dev_fetch_failure_falls_back_to_stale_cache() {
        let mut cached = HashMap::new();
        cached.insert("test-model-gamma".to_string(), model_pricing(5e-6, 6e-6));

        let service = PricingService::combine_fetched_sources(
            Ok(HashMap::new()),
            Err("OpenRouter unavailable".to_string()),
            Err("models.dev unreachable".to_string()),
            || None,
            || None,
            || Some(cached),
            CustomPricing::default(),
        )
        .expect("a stale models.dev cache must keep the service usable");

        let cost = service.calculate_cost("test-model-gamma", 1_000_000, 0, 0, 0, 0);
        assert!(
            (cost - 5.0).abs() < 1e-9,
            "stale models.dev cache should price the model, got {}",
            cost
        );
    }

    #[test]
    fn custom_pricing_keeps_service_available_during_dynamic_outage() {
        let mut custom = HashMap::new();
        custom.insert("custom-only".to_string(), model_pricing(3e-6, 4e-6));
        let service = PricingService::combine_fetched_sources(
            Err("error decoding response body: expected f64".to_string()),
            Err("OpenRouter unreachable".to_string()),
            Err("models.dev unreachable".to_string()),
            || None,
            || None,
            || None,
            CustomPricing::from_models(custom),
        )
        .expect("custom pricing should remain usable during an upstream outage");
        assert!(service.lookup_with_source("custom-only", None).is_some());
    }

    #[test]
    fn openrouter_fetch_failure_falls_back_to_stale_cache() {
        let mut cached = HashMap::new();
        cached.insert("openrouter-only".to_string(), model_pricing(7e-6, 8e-6));

        let service = PricingService::combine_fetched_sources(
            Err("LiteLLM unavailable".to_string()),
            Err("OpenRouter unavailable".to_string()),
            Err("models.dev unavailable".to_string()),
            || None,
            || Some(cached),
            || None,
            CustomPricing::default(),
        )
        .expect("a stale OpenRouter cache must keep the service usable");

        assert!(service
            .lookup_with_source("openrouter-only", None)
            .is_some());
    }

    #[test]
    fn models_dev_parses_fixture_prices_per_token() {
        let data = fixture_models_dev();
        let pricing = data.get("openai/gpt-fixture-model").unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(0.00000125));
        assert_eq!(pricing.output_cost_per_token, Some(0.00001));
        assert_eq!(pricing.cache_read_input_token_cost, Some(0.000000125));
        assert_eq!(pricing.cache_creation_input_token_cost, Some(0.000001875));
        assert!(!data.contains_key("openai/missing-output-price"));
    }

    #[test]
    fn models_dev_fills_provider_aware_fallback_prices() {
        let service = custom_service_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );

        let result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();

        assert_eq!(result.source, "Models.dev");
        assert_eq!(result.matched_key, "openai/gpt-fixture-model");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000125));
    }

    #[test]
    fn models_dev_cache_prices_are_used_for_cost_fallback() {
        let service = custom_service_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );
        let usage = TokenBreakdown {
            input: 1_000_000,
            output: 100_000,
            cache_read: 50_000,
            cache_write: 20_000,
            reasoning: 0,
        };

        let cost =
            service.calculate_cost_with_provider("gpt-fixture-model", Some("openai"), &usage);

        let expected = 1.25 + 1.0 + 0.00625 + 0.0375;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn existing_sources_beat_models_dev_fallback() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-fixture-model".into(),
            model_pricing(0.000002, 0.000008),
        );
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-fixture-sonnet".into(),
            model_pricing(0.000004, 0.000016),
        );

        let service = custom_service_with_models_dev(
            HashMap::new(),
            litellm,
            openrouter,
            fixture_models_dev(),
        );

        let litellm_result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();
        assert_eq!(litellm_result.source, "LiteLLM");
        assert_eq!(litellm_result.pricing.input_cost_per_token, Some(0.000002));

        let openrouter_result = service
            .lookup_with_source_and_provider("claude-fixture-sonnet", None, Some("anthropic"))
            .unwrap();
        assert_eq!(openrouter_result.source, "OpenRouter");
        assert_eq!(
            openrouter_result.pricing.input_cost_per_token,
            Some(0.000004)
        );
    }

    #[test]
    fn models_dev_respects_forced_source_boundaries() {
        let service = custom_service_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );

        assert!(service
            .lookup_with_source_and_provider("gpt-fixture-model", Some("litellm"), Some("openai"))
            .is_none());
        assert!(service
            .lookup_with_source_and_provider(
                "gpt-fixture-model",
                Some("openrouter"),
                Some("openai")
            )
            .is_none());

        let result = service
            .lookup_with_source_and_provider(
                "gpt-fixture-model",
                Some("models.dev"),
                Some("openai"),
            )
            .unwrap();
        assert_eq!(result.source, "Models.dev");
    }

    #[test]
    fn custom_override_beats_models_dev_fallback() {
        let mut custom = HashMap::new();
        custom.insert(
            "gpt-fixture-model".into(),
            model_pricing(0.000009, 0.000018),
        );

        let service = custom_service_with_models_dev(
            custom,
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );

        let result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000009));
    }

    #[test]
    fn test_filter_excludes_github_copilot() {
        let mut data = HashMap::new();
        data.insert(
            "github_copilot/gpt-5.3-codex".into(),
            ModelPricing::default(),
        );
        data.insert("github_copilot/gpt-4o".into(), ModelPricing::default());
        data.insert(
            "gpt-5.2".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000175),
                ..Default::default()
            },
        );
        data.insert(
            "openai/gpt-5.2".into(),
            ModelPricing {
                output_cost_per_token: Some(0.000014),
                ..Default::default()
            },
        );
        data.insert(
            "tier-only".into(),
            ModelPricing {
                input_cost_per_token_above_272k_tokens: Some(0.00001),
                ..Default::default()
            },
        );

        let filtered = PricingService::filter_litellm_data(data);
        assert!(!filtered.contains_key("github_copilot/gpt-5.3-codex"));
        assert!(!filtered.contains_key("github_copilot/gpt-4o"));
        assert!(filtered.contains_key("gpt-5.2"));
        assert!(filtered.contains_key("openai/gpt-5.2"));
        assert!(!filtered.contains_key("tier-only"));
    }

    #[test]
    fn test_cursor_returns_pricing_when_not_in_upstream() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000175));
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000014));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(1.75e-7));
    }

    #[test]
    fn test_cursor_yields_to_litellm_exact() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.002),
                output_cost_per_token: Some(0.016),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.002));
    }

    #[test]
    fn test_cursor_yields_to_openrouter_prefix() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "openai/gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.003),
                output_cost_per_token: Some(0.012),
                ..Default::default()
            },
        );
        let service = PricingService::new(HashMap::new(), openrouter);
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.003));
    }

    #[test]
    fn test_cursor_skipped_when_force_source_set() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        assert!(service
            .lookup_with_source("gpt-5.3-codex", Some("litellm"))
            .is_none());
        assert!(service
            .lookup_with_source("gpt-5.3-codex", Some("openrouter"))
            .is_none());
    }

    #[test]
    fn test_cursor_matches_after_version_normalization() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("gpt-5-3-codex", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "gpt-5.3-codex");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000175));
    }

    #[test]
    fn test_cursor_matches_provider_prefixed_input() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("openai/gpt-5.3-codex", None)
            .unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "gpt-5.3-codex");
    }

    #[test]
    fn test_cursor_provider_prefix_yields_to_upstream() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "openai/gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.003),
                output_cost_per_token: Some(0.012),
                ..Default::default()
            },
        );
        let service = PricingService::new(HashMap::new(), openrouter);
        let result = service
            .lookup_with_source("openai/gpt-5.3-codex", None)
            .unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.003));
    }

    #[test]
    fn test_cursor_matches_via_suffix_stripping() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("gpt-5.3-codex-high", None)
            .unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "gpt-5.3-codex");
    }

    #[test]
    fn test_cursor_calculate_cost() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("gpt-5.3-codex", 1_000_000, 100_000, 0, 0, 0);
        let expected = 1_000_000.0 * 0.00000175 + 100_000.0 * 0.000014;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_1() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 1", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 1");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000125));
        assert_eq!(result.pricing.output_cost_per_token, Some(0.00001));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(1.25e-7));
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_1() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("Composer 1", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 0.00000125 + 100_000.0 * 0.00001 + 50_000.0 * 1.25e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_returns_pricing_for_hyphenated_composer_1() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-1", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-1");
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_1_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 1.5", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 1.5");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.0000035));
        assert_eq!(result.pricing.output_cost_per_token, Some(0.0000175));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(3.5e-7));
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_1_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("Composer 1.5", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 0.0000035 + 100_000.0 * 0.0000175 + 50_000.0 * 3.5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_returns_pricing_for_hyphenated_composer_1_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-1.5", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-1.5");
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-2", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2");
        assert_eq!(result.pricing.input_cost_per_token, Some(5e-7));
        assert_eq!(result.pricing.output_cost_per_token, Some(2.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(2e-7));
        // Cursor documents cache creation as FREE for the Composer 2 family.
        // Some(0.0) and None compute the same cost, but only Some(0.0)
        // makes covers_usage accept cache_write usage for submission.
        assert_eq!(result.pricing.cache_creation_input_token_cost, Some(0.0));
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_spaced() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 2", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 2");
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-2-fast", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2-fast");
        assert_eq!(result.pricing.input_cost_per_token, Some(1.5e-6));
        assert_eq!(result.pricing.output_cost_per_token, Some(7.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(3.5e-7));
        // Cursor documents cache creation as FREE for the Composer 2 family.
        // Some(0.0) and None compute the same cost, but only Some(0.0)
        // makes covers_usage accept cache_write usage for submission.
        assert_eq!(result.pricing.cache_creation_input_token_cost, Some(0.0));
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_fast_spaced() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 2 Fast", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 2 fast");
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 5e-7 + 100_000.0 * 2.5e-6 + 50_000.0 * 2e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2", 0, 0, 0, 0, 0);
        assert!((with_write - without_write).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2-fast", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 1.5e-6 + 100_000.0 * 7.5e-6 + 50_000.0 * 3.5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_fast_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2-fast", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2-fast", 0, 0, 0, 0, 0);
        assert!(
            (with_write - without_write).abs() < 1e-10,
            "Cache creation should be free for Composer 2 Fast"
        );
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-2.5", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2.5");
        assert_eq!(result.pricing.input_cost_per_token, Some(5e-7));
        assert_eq!(result.pricing.output_cost_per_token, Some(2.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(2e-7));
        // Cursor documents cache creation as FREE for the Composer 2 family.
        // Some(0.0) and None compute the same cost, but only Some(0.0)
        // makes covers_usage accept cache_write usage for submission.
        assert_eq!(result.pricing.cache_creation_input_token_cost, Some(0.0));
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_5_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("composer-2.5-fast", None)
            .unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2.5-fast");
        assert_eq!(result.pricing.input_cost_per_token, Some(1.5e-6));
        assert_eq!(result.pricing.output_cost_per_token, Some(7.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(3.5e-7));
        // Cursor documents cache creation as FREE for the Composer 2 family.
        // Some(0.0) and None compute the same cost, but only Some(0.0)
        // makes covers_usage accept cache_write usage for submission.
        assert_eq!(result.pricing.cache_creation_input_token_cost, Some(0.0));
    }

    #[test]
    fn test_grok_composer_2_5_fast_uses_composer_2_5_fast_override() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("grok-composer-2.5-fast", None)
            .unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2.5-fast");
        assert_eq!(result.pricing.input_cost_per_token, Some(1.5e-6));
        assert_eq!(result.pricing.output_cost_per_token, Some(7.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(3.5e-7));

        let cost =
            service.calculate_cost("grok-composer-2.5-fast", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 1.5e-6 + 100_000.0 * 7.5e-6 + 50_000.0 * 3.5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2.5", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 5e-7 + 100_000.0 * 2.5e-6 + 50_000.0 * 2e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_5_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2.5", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2.5", 0, 0, 0, 0, 0);
        assert!((with_write - without_write).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2_5_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2.5-fast", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 1.5e-6 + 100_000.0 * 7.5e-6 + 50_000.0 * 3.5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_5_fast_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2.5-fast", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2.5-fast", 0, 0, 0, 0, 0);
        assert!(
            (with_write - without_write).abs() < 1e-10,
            "Cache creation should be free for Composer 2.5 Fast"
        );
    }

    #[test]
    fn test_cursor_composer_lookup_case_insensitive() {
        let service = PricingService::new(HashMap::new(), HashMap::new());

        let lower = service.lookup_with_source("composer 1", None);
        let upper = service.lookup_with_source("COMPOSER 1", None);
        let mixed = service.lookup_with_source("Composer 1", None);

        assert!(lower.is_some(), "lowercase should resolve");
        assert!(upper.is_some(), "UPPERCASE should resolve");
        assert!(mixed.is_some(), "Mixed Case should resolve");

        assert_eq!(
            lower.unwrap().pricing.input_cost_per_token,
            upper.unwrap().pricing.input_cost_per_token
        );
    }

    /// Regression: Composer 2's cache creation is documented FREE, but it was
    /// encoded as `None` ("rate unknown"), so `covers_usage` reported the row
    /// as not covering any usage with cache_write and submission excluded it.
    /// The cost is unchanged either way — `compute_cost` reads an absent rate
    /// as 0.0 — so this is purely about the coverage verdict.
    #[test]
    fn cursor_documented_free_cache_creation_covers_cache_write_usage() {
        let overrides = PricingService::build_cursor_overrides();
        let usage = crate::TokenBreakdown {
            input: 1_000,
            output: 500,
            cache_read: 200,
            cache_write: 300,
            ..Default::default()
        };

        let composer2 = overrides.get("composer-2").expect("composer-2 override");
        assert_eq!(composer2.cache_creation_input_token_cost, Some(0.0));
        assert!(
            composer2.covers_usage(&usage),
            "documented-free cache creation must count as covered"
        );

        // Composer 1 has no documented cache-creation rate, so it stays unknown
        // rather than being guessed at zero. Excluding it is the honest answer.
        let composer1 = overrides.get("composer-1").expect("composer-1 override");
        assert_eq!(composer1.cache_creation_input_token_cost, None);
        assert!(!composer1.covers_usage(&usage));
    }

    #[test]
    fn test_sakana_returns_pricing_for_fugu_ultra() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("fugu-ultra", None).unwrap();
        assert_eq!(result.source, "Sakana");
        assert_eq!(result.matched_key, "fugu-ultra");
        assert_eq!(result.pricing.input_cost_per_token, Some(5e-6));
        assert_eq!(result.pricing.output_cost_per_token, Some(3e-5));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(5e-7));
        assert_eq!(result.pricing.cache_creation_input_token_cost, None);
        // >272K tier fields are populated (compute_cost reads them).
        assert_eq!(
            result.pricing.input_cost_per_token_above_272k_tokens,
            Some(1e-5)
        );
        assert_eq!(
            result.pricing.output_cost_per_token_above_272k_tokens,
            Some(4.5e-5)
        );
        assert_eq!(
            result.pricing.cache_read_input_token_cost_above_272k_tokens,
            Some(1e-6)
        );
    }

    #[test]
    fn test_sakana_calculate_cost_for_fugu_ultra() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        // Stay under the 272K threshold so only base rates apply.
        let cost = service.calculate_cost("fugu-ultra", 100_000, 10_000, 50_000, 0, 0);
        let expected = 100_000.0 * 5e-6 + 10_000.0 * 3e-5 + 50_000.0 * 5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_sakana_yields_to_litellm_exact() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "fugu-ultra".into(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let result = service.lookup_with_source("fugu-ultra", None).unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.001));
    }

    #[test]
    fn test_sakana_does_not_price_bare_fugu() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        // Bare `fugu` is a router/orchestrator — deliberately unpriced by Sakana.
        let result = service.lookup_with_source("fugu", None);
        assert!(
            result.as_ref().is_none_or(|r| r.source != "Sakana"),
            "bare `fugu` must not resolve to a Sakana price"
        );
    }

    #[test]
    fn test_sakana_resolves_dated_fugu_ultra_alias() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("fugu-ultra-20260615", None)
            .unwrap();
        assert_eq!(result.source, "Sakana");
        assert_eq!(result.matched_key, "fugu-ultra");
        assert_eq!(result.pricing.input_cost_per_token, Some(5e-6));
    }

    #[test]
    fn test_from_cached_datasets_returns_none_when_both_sources_missing() {
        assert!(PricingService::from_cached_datasets(None, None, None).is_none());
    }

    #[test]
    fn test_from_cached_datasets_filters_subscription_only_litellm_entries() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "github_copilot/gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        litellm.insert(
            "gpt-5.2".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000175),
                ..Default::default()
            },
        );

        let service = PricingService::from_cached_datasets(Some(litellm), None, None).unwrap();

        assert!(service
            .lookup_with_source("github_copilot/gpt-5.3-codex", Some("litellm"))
            .is_none());
        assert!(service
            .lookup_with_source("gpt-5.2", Some("litellm"))
            .is_some());
    }

    #[test]
    fn test_from_cached_datasets_uses_models_dev_when_other_sources_missing() {
        let service =
            PricingService::from_cached_datasets(None, None, Some(fixture_models_dev())).unwrap();

        let result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();

        assert_eq!(result.source, "Models.dev");
        assert_eq!(result.matched_key, "openai/gpt-fixture-model");
    }

    #[test]
    fn custom_override_wins_over_litellm() {
        let mut custom = HashMap::new();
        custom.insert("gpt-4o".into(), model_pricing(0.000002, 0.000008));
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.00001, 0.00003));

        let service = custom_service(custom, litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-4o", None).unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.matched_key, "gpt-4o");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_override_wins_over_openrouter() {
        let mut custom = HashMap::new();
        custom.insert("grok-code".into(), model_pricing(0.000002, 0.000008));
        let mut openrouter = HashMap::new();
        openrouter.insert("x-ai/grok-code".into(), model_pricing(0.00001, 0.00003));

        let service = custom_service(custom, HashMap::new(), openrouter);
        let result = service.lookup_with_source("grok-code", None).unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.matched_key, "grok-code");
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000008));
    }

    #[test]
    fn custom_override_respects_force_source() {
        let mut custom = HashMap::new();
        custom.insert("gpt-4o".into(), model_pricing(0.000002, 0.000008));
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.00001, 0.00003));
        let mut openrouter = HashMap::new();
        openrouter.insert("openai/gpt-4o".into(), model_pricing(0.000003, 0.000012));

        let service = custom_service(custom, litellm, openrouter);

        let litellm_result = service
            .lookup_with_source("gpt-4o", Some("litellm"))
            .unwrap();
        assert_eq!(litellm_result.source, "LiteLLM");
        assert_eq!(litellm_result.pricing.input_cost_per_token, Some(0.00001));

        let openrouter_result = service
            .lookup_with_source("gpt-4o", Some("openrouter"))
            .unwrap();
        assert_eq!(openrouter_result.source, "OpenRouter");
        assert_eq!(
            openrouter_result.pricing.input_cost_per_token,
            Some(0.000003)
        );

        let custom_result = service
            .lookup_with_source("gpt-4o", Some("custom"))
            .unwrap();
        assert_eq!(custom_result.source, "Custom");
        assert_eq!(custom_result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_force_source_does_not_fall_through_on_miss() {
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.0000025, 0.00001));

        let service = custom_service(HashMap::new(), litellm, HashMap::new());

        assert!(service
            .lookup_with_source("gpt-4o", Some("custom"))
            .is_none());
    }

    #[test]
    fn custom_override_raw_match_wins() {
        let mut custom = HashMap::new();
        custom.insert(
            "accounts/fireworks/routers/kimi-k2p6-turbo".into(),
            model_pricing(0.000002, 0.000008),
        );
        let mut litellm = HashMap::new();
        litellm.insert("kimi-k2.6".into(), model_pricing(0.00000095, 0.000004));

        let service = custom_service(custom, litellm, HashMap::new());
        let result = service
            .lookup_with_source("accounts/fireworks/routers/kimi-k2p6-turbo", None)
            .unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(
            result.matched_key,
            "accounts/fireworks/routers/kimi-k2p6-turbo"
        );
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_override_normalized_match_wins() {
        let mut custom = HashMap::new();
        custom.insert("kimi-k2p6".into(), model_pricing(0.00000095, 0.000004));
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4-turbo".into(), model_pricing(0.00001, 0.00003));

        let service = custom_service(custom, litellm, HashMap::new());
        let result = service
            .lookup_with_source("accounts/fireworks/models/kimi-k2p6", None)
            .unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.matched_key, "kimi-k2p6");
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000004));
    }

    #[test]
    fn custom_override_raw_beats_normalized() {
        let mut custom = HashMap::new();
        custom.insert("kimi-k2p6-turbo".into(), model_pricing(0.000001, 0.000004));
        custom.insert(
            "accounts/fireworks/models/kimi-k2p6-turbo".into(),
            model_pricing(0.000002, 0.000008),
        );

        let service = custom_service(custom, HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("accounts/fireworks/models/kimi-k2p6-turbo", None)
            .unwrap();

        assert_eq!(
            result.matched_key,
            "accounts/fireworks/models/kimi-k2p6-turbo"
        );
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_override_skips_fuzzy_chain() {
        let mut custom = HashMap::new();
        custom.insert("kimi-k2p6-turbo".into(), model_pricing(0.000002, 0.000008));

        let service = custom_service(custom, HashMap::new(), HashMap::new());

        assert!(service
            .lookup_with_source("my-kimi-k2p6-turbo", None)
            .is_none());
    }

    #[test]
    fn no_custom_falls_through_to_litellm() {
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.0000025, 0.00001));

        let service = custom_service(HashMap::new(), litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-4o", None).unwrap();

        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.0000025));
    }

    #[test]
    fn custom_calculate_cost_uses_override() {
        let mut custom = HashMap::new();
        custom.insert(
            "accounts/fireworks/routers/kimi-k2p6-turbo".into(),
            model_pricing(0.000002, 0.000008),
        );
        let mut litellm = HashMap::new();
        litellm.insert(
            "accounts/fireworks/routers/kimi-k2p6-turbo".into(),
            model_pricing(0.00001, 0.00003),
        );

        let service = custom_service(custom, litellm, HashMap::new());
        let cost = service.calculate_cost(
            "accounts/fireworks/routers/kimi-k2p6-turbo",
            1_000_000,
            100_000,
            0,
            0,
            0,
        );

        let expected = 1_000_000.0 * 0.000002 + 100_000.0 * 0.000008;
        assert!((cost - expected).abs() < 1e-10);
    }

    /// A custom entry keyed by the normalized model name still prices a raw
    /// synthetic id, and provenance has to say the match came from that
    /// normalized key rather than from the id as written.
    #[test]
    fn custom_match_through_synthetic_normalization_reports_normalized_provenance() {
        let service = custom_service(
            HashMap::from([("glm-4.7".to_string(), model_pricing(0.000001, 0.000002))]),
            HashMap::new(),
            HashMap::new(),
        );

        let normalized = service
            .lookup_with_source("hf:zai-org/GLM-4.7", None)
            .expect("the normalized custom key prices the synthetic id");
        assert_eq!(normalized.source, "Custom");
        assert_eq!(normalized.matched_key, "glm-4.7");
        assert!(
            normalized.evidence.normalized,
            "a match reached through synthetic-model normalization is normalized"
        );
        assert!(normalized.evidence.is_submission_safe());

        // The raw key is untouched: only the fallback path is normalized.
        let raw = service
            .lookup_with_source("glm-4.7", None)
            .expect("the custom key still matches as written");
        assert_eq!(raw.matched_key, "glm-4.7");
        assert!(!raw.evidence.normalized);
    }

    #[test]
    fn cross_provider_model_part_price_remains_an_estimate_only() {
        let service = PricingService::new(
            HashMap::new(),
            HashMap::from([(
                "vendor/atlas-chat".to_string(),
                model_pricing(0.000001, 0.000002),
            )]),
        );
        let usage = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };

        let resolution = service
            .lookup_with_source("atlas-chat", None)
            .expect("lenient reporting keeps the model-part estimate visible");
        assert_eq!(resolution.matched_key, "vendor/atlas-chat");
        assert_eq!(resolution.evidence.kind, ResolutionKind::ModelPart);
        assert!(!resolution.evidence.is_submission_safe());
        assert!(service.calculate_cost_with_provider("atlas-chat", None, &usage) > 0.0);
        assert!(!service.covers_usage_with_provider("atlas-chat", None, &usage));
    }

    #[test]
    fn provider_prefix_and_cross_endpoint_aliases_remain_estimates_only() {
        let service = PricingService::new(
            HashMap::from([
                (
                    "anthropic/atlas-chat".to_string(),
                    model_pricing(0.000001, 0.000002),
                ),
                (
                    "vertex_ai/vertex-chat".to_string(),
                    model_pricing(0.000003, 0.000006),
                ),
                (
                    "vertex_ai/accounts/anthropic/models/vertex-chat".to_string(),
                    model_pricing(0.000003, 0.000006),
                ),
            ]),
            HashMap::from([(
                "vertex_ai/accounts/anthropic/models/vertex-chat".to_string(),
                model_pricing(0.000003, 0.000006),
            )]),
        );
        let usage = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };

        let prefixed = service
            .lookup_with_source_and_provider("atlas-chat", None, Some("synthetic"))
            .expect("the provider-prefix estimate remains visible");
        assert_eq!(prefixed.evidence.kind, ResolutionKind::ProviderPrefix);
        assert!(!prefixed.evidence.is_submission_safe());
        assert!(!service.covers_usage_with_provider("atlas-chat", Some("synthetic"), &usage));

        let aliased = service
            .lookup_with_source_and_provider("vertex-chat", None, Some("anthropic"))
            .expect("the cross-endpoint alias estimate remains visible");
        assert_eq!(aliased.evidence.kind, ResolutionKind::ModelPart);
        assert!(!aliased.evidence.is_submission_safe());
        assert!(!service.covers_usage_with_provider("vertex-chat", Some("anthropic"), &usage));

        assert!(
            service.calculate_cost_with_provider("atlas-chat", Some("synthetic"), &usage) > 0.0
        );
        assert!(
            service.calculate_cost_with_provider("vertex-chat", Some("anthropic"), &usage) > 0.0
        );

        for source in [None, Some("litellm"), Some("openrouter")] {
            let scoped = service
                .lookup_with_source_and_provider(
                    "accounts/anthropic/models/vertex-chat",
                    source,
                    Some("anthropic"),
                )
                .unwrap_or_else(|| {
                    panic!("the {source:?} scoped cross-endpoint alias remains visible")
                });
            assert_eq!(scoped.evidence.kind, ResolutionKind::ModelPart);
            assert!(!scoped.evidence.is_submission_safe());
        }
        assert!(!service.covers_usage_with_provider(
            "accounts/anthropic/models/vertex-chat",
            Some("anthropic"),
            &usage,
        ));
    }

    #[test]
    fn ambiguous_fuzzy_price_remains_visible_but_does_not_cover_submission() {
        let litellm = HashMap::from([
            (
                "vendor-a/atlas-chat-preview".into(),
                model_pricing(0.000001, 0.000002),
            ),
            (
                "vendor-b/atlas-chat-beta".into(),
                model_pricing(0.000003, 0.000006),
            ),
        ]);
        let service = PricingService::new(litellm, HashMap::new());
        let usage = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };

        let resolution = service
            .lookup_with_source("atlas-chat", None)
            .expect("lenient reporting keeps the estimated price visible");
        assert!(!resolution.evidence.is_submission_safe());
        assert!(service.calculate_cost_with_provider("atlas-chat", None, &usage) > 0.0);
        assert!(!service.covers_usage_with_provider("atlas-chat", None, &usage));
    }

    /// Regression: an archived row is one model's own last published tariff,
    /// not a family price, so the generic suffix stripper must not reach it.
    ///
    /// `mimo-v2.5-pro` (the id the MiMo Code fixture records), `glm-5.3-flash`
    /// and `hy3-instruct` are SKUs no vendor page prices. Peeling the trailing
    /// segment and answering from the base model's archived row billed them at
    /// a neighbouring model's rate and -- under that vendor's own hint --
    /// stamped the result submission-safe, which would publish a made-up
    /// price. With no dataset row of their own they stay unpriced, exactly as
    /// they were before the archive carried these vendors.
    #[test]
    fn unknown_sku_suffix_never_reaches_the_archive() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        };

        for (model, provider) in [
            ("mimo-v2.5-pro", Some("xiaomi")),
            ("mimo-v2.5-pro", Some("mimo")),
            ("glm-5.2-air", Some("zhipu")),
            ("glm-5.2-flash", Some("zhipu")),
            ("glm-5.2-x", Some("zhipu")),
            ("glm-5.2-thinking", Some("zhipu")),
            ("glm-5.3-air", Some("zhipu")),
            ("glm-5.3-flash", Some("zhipu")),
            ("hy3-instruct", Some("tencent")),
            ("hy3-instruct", Some("hunyuan")),
        ] {
            assert!(
                service
                    .resolve_for_usage_with_provider(model, provider, &usage)
                    .is_none(),
                "model: {model}, an unknown SKU has no archived rate of its own"
            );
            assert!(
                !service.covers_usage_with_provider(model, provider, &usage),
                "model: {model}, a suffix-eroded price must never become submission-safe"
            );
        }

        // The base SKUs the archive does carry keep resolving: the guard
        // refuses eroded candidates, not the archive.
        for (model, provider) in [
            ("mimo-v2.5", Some("xiaomi")),
            ("glm-5.2", Some("zhipu")),
            ("hy3", Some("tencent")),
        ] {
            let resolved = service
                .resolve_for_usage_with_provider(model, provider, &usage)
                .unwrap_or_else(|| panic!("{model} must keep its archived price"));
            assert_eq!(resolved.source, "Tokscale Archive", "model: {model}");
            assert!(resolved.evidence.is_submission_safe(), "model: {model}");
        }
    }

    /// Regression: the guard declines the ARCHIVE for an eroded candidate, not
    /// the whole lookup. Filtering the composed result instead threw the live
    /// row away with it: inside `lookup_auto` the archive substitutes itself
    /// for an unverified upstream hit, so the marketplace row that prices
    /// `mimo-v2.5-pro` was already gone by the time the filter saw the source,
    /// and merely ADDING an archive row dropped every MiMo Code session to $0.
    #[test]
    fn suffix_eroded_sku_keeps_its_live_dataset_price() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openrouter/xiaomi/mimo-v2.5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.14e-6),
                output_cost_per_token: Some(0.28e-6),
                cache_read_input_token_cost: Some(0.0028e-6),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let usage = TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        };

        // `mimo-v2.5-pro` under a `mimo` hint is verbatim what the MiMo Code
        // parser records, and the marketplace row is the only live key for it.
        let resolved = service
            .resolve_for_usage_with_provider("mimo-v2.5-pro", Some("mimo"), &usage)
            .expect("the marketplace row must keep pricing the unknown -pro SKU");
        assert_eq!(resolved.source, "LiteLLM");
        assert_eq!(resolved.matched_key, "openrouter/xiaomi/mimo-v2.5");
        assert!(service.calculate_cost_with_provider("mimo-v2.5-pro", Some("mimo"), &usage) > 0.0);

        // The id the archive does carry still takes the first-party tariff.
        let base = service
            .resolve_for_usage_with_provider("mimo-v2.5", Some("mimo"), &usage)
            .expect("mimo-v2.5 must resolve first-party");
        assert_eq!(base.source, "Tokscale Archive");
    }
}
