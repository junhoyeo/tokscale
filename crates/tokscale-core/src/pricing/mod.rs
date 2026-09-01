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

use crate::{provider_identity, TokenBreakdown};

pub use litellm::ModelPricing;

static PRICING_SERVICE: OnceCell<Arc<PricingService>> = OnceCell::const_new();

/// Last known per-token tariff for a provider model that may disappear from
/// the live pricing datasets after retirement.
struct ArchivedPriceRow {
    provider_id: &'static str,
    model_id: &'static str,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

// @keep: these are immutable retirement snapshots, not aliases or estimates
// that should track a similarly named current model.
//
// The Claude rows preserve the final rates Tokscale resolved from LiteLLM on
// 2026-08-31 before the upstream lifecycle could remove them:
//   Haiku 4.5  $1/$5,   cache read/write $0.10/$1.25 per 1M
//   Opus 4.7   $5/$25,  cache read/write $0.50/$6.25 per 1M
//   Opus 4.8   $5/$25,  cache read/write $0.50/$6.25 per 1M
//   Sonnet 4.6 $3/$15,  cache read/write $0.30/$3.75 per 1M
//
// OpenAI's Codex catalog exposes `codex-auto-review` as a concrete hidden API
// model ("Automatic approval review model for Codex") but publishes no tariff.
// Before strict submission evidence rejected the fuzzy result, Tokscale's last
// deterministic selection was OpenRouter's `openai/gpt-5.1-codex-mini` row:
// $0.25/$2 and $0.025 cached input per 1M. Cache creation is explicitly free
// for OpenAI prompt caching, so the snapshot covers every Tokscale bucket.
const ARCHIVED_MODEL_PRICES: &[ArchivedPriceRow] = &[
    ArchivedPriceRow {
        provider_id: "anthropic",
        model_id: "claude-haiku-4-5",
        input: 1e-6,
        output: 5e-6,
        cache_read: 1e-7,
        cache_write: 1.25e-6,
    },
    ArchivedPriceRow {
        provider_id: "anthropic",
        model_id: "claude-opus-4-7",
        input: 5e-6,
        output: 25e-6,
        cache_read: 5e-7,
        cache_write: 6.25e-6,
    },
    ArchivedPriceRow {
        provider_id: "anthropic",
        model_id: "claude-opus-4-8",
        input: 5e-6,
        output: 25e-6,
        cache_read: 5e-7,
        cache_write: 6.25e-6,
    },
    ArchivedPriceRow {
        provider_id: "anthropic",
        model_id: "claude-sonnet-4-6",
        input: 3e-6,
        output: 15e-6,
        cache_read: 3e-7,
        cache_write: 3.75e-6,
    },
    ArchivedPriceRow {
        provider_id: "openai",
        model_id: "codex-auto-review",
        input: 2.5e-7,
        output: 2e-6,
        cache_read: 2.5e-8,
        cache_write: 0.0,
    },
];

fn strip_archived_reasoning_suffix(model_id: &str) -> &str {
    for suffix in [
        "-thinking-xhigh",
        "-thinking-high",
        "-thinking-medium",
        "-thinking-low",
        "-thinking",
    ] {
        if let Some(base) = model_id.strip_suffix(suffix) {
            return base;
        }
    }
    model_id
}

/// Provider spellings that [`provider_identity::canonical_provider`] folds into
/// a first-party vendor tag even though they name a different commercial
/// endpoint with its own price sheet.
///
/// Folding is right for attribution — a Vertex-served Claude is still a Claude —
/// but wrong for authorising a *first-party* archived tariff, for exactly the
/// reason `provider_identity` refuses to fold the `-cn` regional endpoints: the
/// hosted rates can differ, and the archive only holds the vendor's own. Note
/// this is not the same question as spelling variants like `openai_codex`,
/// which is first-party OpenAI and must keep resolving.
fn is_rehosted_endpoint(raw: &str) -> bool {
    raw.split('/').any(|segment| {
        matches!(
            segment
                .trim()
                .to_ascii_lowercase()
                .replace('-', "_")
                .as_str(),
            "vertex" | "vertex_ai"
        )
    })
}

fn archived_pricing_result(model_id: &str, provider_id: Option<&str>) -> Option<LookupResult> {
    let lower = model_id.trim().to_ascii_lowercase();
    let (embedded_raw, bare_model) = lower
        .rsplit_once('/')
        .map(|(provider, model)| (Some(provider), model))
        .unwrap_or((None, lower.as_str()));

    // The archive carries first-party tariffs only, so an endpoint that merely
    // shares the canonical vendor tag must fall through to a live resolver
    // rather than be priced — and marked submission-safe — from it.
    if provider_id.is_some_and(is_rehosted_endpoint)
        || embedded_raw.is_some_and(is_rehosted_endpoint)
    {
        return None;
    }

    let embedded_provider = embedded_raw.and_then(provider_identity::canonical_provider);
    let requested_provider = provider_id.and_then(provider_identity::canonical_provider);

    if requested_provider.is_some()
        && embedded_provider.is_some()
        && requested_provider != embedded_provider
    {
        return None;
    }

    let effective_provider = requested_provider.or(embedded_provider);
    let bare_model = strip_archived_reasoning_suffix(bare_model);
    let row = ARCHIVED_MODEL_PRICES.iter().find(|row| {
        effective_provider
            .as_deref()
            .is_none_or(|provider| provider == row.provider_id)
            && bare_model == row.model_id
    })?;

    Some(LookupResult {
        pricing: ModelPricing {
            input_cost_per_token: Some(row.input),
            output_cost_per_token: Some(row.output),
            cache_read_input_token_cost: Some(row.cache_read),
            cache_creation_input_token_cost: Some(row.cache_write),
            ..Default::default()
        },
        source: "Tokscale Archive".into(),
        matched_key: format!("{}/{}", row.provider_id, row.model_id),
        evidence: ResolutionEvidence::deterministic(ResolutionKind::BuiltIn),
    })
}

fn prefer_submission_safe_or_archived(
    dynamic: Option<LookupResult>,
    model_id: &str,
    provider_id: Option<&str>,
    usage: Option<&TokenBreakdown>,
) -> Option<LookupResult> {
    let dynamic_is_complete = dynamic.as_ref().is_some_and(|result| {
        result.evidence.is_submission_safe()
            && usage.is_none_or(|usage| result.pricing.covers_usage(usage))
    });
    if dynamic_is_complete {
        return dynamic;
    }

    let archived = archived_pricing_result(model_id, provider_id).or_else(|| {
        dynamic
            .as_ref()
            .and_then(|result| archived_pricing_result(&result.matched_key, provider_id))
    });
    if archived
        .as_ref()
        .is_some_and(|result| usage.is_none_or(|usage| result.pricing.covers_usage(usage)))
    {
        return archived;
    }

    dynamic
}

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
            lookup: PricingLookup::new_with_models_dev(
                litellm_data,
                openrouter_data,
                Self::build_cursor_overrides(),
                Self::build_sakana_overrides(),
                models_dev_data,
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

        let dynamic = self.lookup.lookup_with_source(model_id, force_source);
        if force_source.is_some() {
            dynamic
        } else {
            prefer_submission_safe_or_archived(dynamic, model_id, None, None)
        }
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

        let dynamic =
            self.lookup
                .lookup_with_source_and_provider(model_id, force_source, provider_id);
        if force_source.is_some() {
            dynamic
        } else {
            prefer_submission_safe_or_archived(dynamic, model_id, provider_id, None)
        }
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

        let resolved = self.resolve_for_usage_with_provider(model_id, provider_id, usage);
        if let Some(result) = resolved.filter(|result| result.source == "Tokscale Archive") {
            return compute_cost(
                &result.pricing,
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

        let dynamic = self.lookup.resolve_for_usage(model_id, provider_id, usage);
        prefer_submission_safe_or_archived(dynamic, model_id, provider_id, Some(usage))
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
    #[test]
    fn archived_tariff_is_not_shared_across_hosted_endpoints() {
        // `vertex` and `vertex_ai` canonicalise to `anthropic`, but a hosted
        // endpoint publishes its own tariff -- the same reason
        // `provider_identity` refuses to fold the `-cn` regional endpoints. The
        // first-party archive must not authorise Vertex usage at Anthropic
        // rates, whether the endpoint arrives as the provider or embedded in
        // the model id.
        for provider in ["vertex", "vertex_ai", "Vertex"] {
            assert!(
                archived_pricing_result("claude-opus-4-7", Some(provider)).is_none(),
                "{provider} must not be priced from the Anthropic archive"
            );
        }
        assert!(archived_pricing_result("vertex/claude-opus-4-7", None).is_none());

        // First-party Anthropic still resolves, including its own spellings.
        assert!(archived_pricing_result("claude-opus-4-7", Some("anthropic")).is_some());
        assert!(archived_pricing_result("claude-opus-4-7", Some("Anthropic")).is_some());
        assert!(archived_pricing_result("anthropic/claude-opus-4-7", None).is_some());
        assert!(archived_pricing_result("claude-opus-4-7", None).is_some());
    }

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
    fn retired_claude_opus_4_7_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-opus-4-7", 5.0, 25.0, 0.5, 6.25);
    }

    #[test]
    fn retired_claude_opus_4_8_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-opus-4-8", 5.0, 25.0, 0.5, 6.25);
    }

    #[test]
    fn retired_claude_sonnet_4_6_keeps_its_last_known_price() {
        assert_retired_anthropic_price("claude-sonnet-4-6", 3.0, 15.0, 0.3, 3.75);
    }

    #[test]
    fn retired_anthropic_price_does_not_cross_provider_boundaries() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = all_bucket_usage();

        assert!(!service.covers_usage_with_provider("claude-opus-4-8", Some("bedrock"), &usage,));
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

    #[test]
    fn codex_auto_review_keeps_its_last_resolved_price() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        };

        let resolved = service
            .resolve_for_usage_with_provider("codex-auto-review", Some("openai"), &usage)
            .expect("codex-auto-review must retain its archived price");
        assert_eq!(resolved.source, "Tokscale Archive");
        assert_eq!(resolved.matched_key, "openai/codex-auto-review");
        assert!(resolved.evidence.is_submission_safe());
        assert!(
            (service.calculate_cost_with_provider("codex-auto-review", Some("openai"), &usage,)
                - 2.275)
                .abs()
                < 1e-9
        );
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
}
