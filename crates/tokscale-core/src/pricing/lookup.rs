use super::{aliases, litellm::ModelPricing};
use crate::{provider_identity, strip_parenthesized_reasoning_tier, TokenBreakdown};
use std::collections::HashMap;
use std::sync::RwLock;

const PROVIDER_PREFIXES: &[&str] = &[
    "openai/",
    "anthropic/",
    "google/",
    "meta-llama/",
    "mistralai/",
    "minimax/",
    "deepseek/",
    "qwen/",
    "cohere/",
    "perplexity/",
    "x-ai/",
];

const ORIGINAL_PROVIDER_PREFIXES: &[&str] = &[
    "x-ai/",
    "xai/",
    "anthropic/",
    "openai/",
    "google/",
    "meta-llama/",
    "mistralai/",
    "minimax/",
    "deepseek/",
    "z-ai/",
    "qwen/",
    "cohere/",
    "perplexity/",
    "moonshotai/",
];

const RESELLER_PROVIDER_PREFIXES: &[&str] = &[
    "azure/",
    "azure_ai/",
    "bedrock/",
    "vertex_ai/",
    "together/",
    "together_ai/",
    "fireworks_ai/",
    "groq/",
    "openrouter/",
    "orcarouter/",
];

// Bare brand tokens ("claude", "anthropic", "gemini") are blocked because they
// contain no model information: a fuzzy hit from them can land on any model of
// the brand (e.g. retired `claude-2.1` eroding to `claude` and billing at an
// opus-fast key, or `gemini-default` eroding to `gemini` and landing on a
// native-audio preview key), so such a match is never trustworthy.
//
// Generic English words ("model", "router", "default") are blocked for the same
// reason: they carry no model identity, yet substring-match real priced keys
// (`azure_ai/model_router`, `kilo/switchpoint/router`, `fireworks-ai-default`).
// Without this guard an id whose only fuzzy-eligible remnant after suffix
// stripping is the word `model` (e.g. `model-zero-usage-v1` -> stripped
// `model`) misprices at the router key's rate. See
// `fuzzy_match_does_not_resolve_generic_model_token`.
//
// `default` is the same failure with a live victim: the generic routing label
// `gemini-default` strips to `default`, which fuzzy-hits LiteLLM's real
// `fireworks-ai-default` row. That row prices at 0.0/0.0, and
// `ModelPricing::covers_usage` treats an explicit zero as a real rate, so the
// label looked *priced* — enough to slip past
// `exclude_unpriced_submission_messages` and be submitted at
// Fireworks AI's rates. A Google routing label is not a Fireworks model.
// See `fuzzy_match_does_not_resolve_generic_default_token`.
const FUZZY_BLOCKLIST: &[&str] = &[
    "auto",
    "mini",
    "chat",
    "base",
    "claude",
    "anthropic",
    "gemini",
    "model",
    "router",
    "default",
];

const MAX_LOOKUP_CACHE_ENTRIES: usize = 512;
const TIERED_PRICING_THRESHOLD_128K_TOKENS: f64 = 128_000.0;
const TIERED_PRICING_THRESHOLD_200K_TOKENS: f64 = 200_000.0;
const TIERED_PRICING_THRESHOLD_256K_TOKENS: f64 = 256_000.0;
const TIERED_PRICING_THRESHOLD_272K_TOKENS: f64 = 272_000.0;

const MIN_FUZZY_MATCH_LEN: usize = 5;

/// Minimum length for a model name candidate after prefix/suffix stripping.
/// Prevents false positives like "pro" or "flash" being matched alone.
const MIN_MODEL_NAME_LEN: usize = 2;

/// Maximum number of leading segments that can be treated as a routing prefix.
/// Limits how aggressively we strip (e.g., "a-b-claude-3" strips at most "a-b-").
const MAX_PREFIX_STRIP_SEGMENTS: usize = 2;

/// Maximum number of trailing segments that can be treated as a routing suffix.
/// Handles tier suffixes (-high, -low) and variant suffixes (-thinking, -codex, -codex-max-xhigh).
const MAX_SUFFIX_STRIP_SEGMENTS: usize = 4;

#[derive(Clone)]
struct CachedResult {
    pricing: ModelPricing,
    source: String,
    matched_key: String,
    evidence: ResolutionEvidence,
}

struct KeyModelPart {
    key: String,
    lower_model_part: String,
}

struct ProviderScopedModelPath<'a> {
    provider: &'a str,
    terminal_model_id: &'a str,
}

pub struct PricingLookup {
    litellm: HashMap<String, ModelPricing>,
    openrouter: HashMap<String, ModelPricing>,
    cursor: HashMap<String, ModelPricing>,
    sakana: HashMap<String, ModelPricing>,
    models_dev: HashMap<String, ModelPricing>,
    litellm_keys: Vec<String>,
    openrouter_keys: Vec<String>,
    litellm_key_parts: Vec<KeyModelPart>,
    openrouter_key_parts: Vec<KeyModelPart>,
    models_dev_key_parts: Vec<KeyModelPart>,
    litellm_lower: HashMap<String, String>,
    openrouter_lower: HashMap<String, String>,
    models_dev_lower: HashMap<String, String>,
    openrouter_model_part: HashMap<String, String>,
    models_dev_model_part: HashMap<String, String>,
    cursor_lower: HashMap<String, String>,
    sakana_lower: HashMap<String, String>,
    lookup_cache: RwLock<HashMap<String, Option<CachedResult>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionKind {
    Exact,
    ModelPart,
    ProviderPrefix,
    /// The provider was established by an explicit scoped path or a provider
    /// hint matched against the qualified catalog candidates.
    ProviderScoped,
    BuiltIn,
    Fuzzy,
    Custom,
}

impl ResolutionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::ModelPart => "model_part",
            Self::ProviderPrefix => "provider_prefix",
            Self::ProviderScoped => "provider_scoped",
            Self::BuiltIn => "built_in",
            Self::Fuzzy => "fuzzy",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionEvidence {
    pub kind: ResolutionKind,
    /// Number of usable candidates considered by a fuzzy lookup. Deterministic
    /// paths report one.
    pub candidate_count: usize,
    /// Whether every considered candidate publishes the same complete rate
    /// vector. This is deliberately stricter than comparing only input/output.
    pub price_consensus: bool,
    /// Whether resolution established exact model identity. Deterministic
    /// paths establish this by construction; fuzzy paths require the selected
    /// key's terminal model segment to exactly name the requested model.
    pub exact_model_identity: bool,
    pub alias_applied: bool,
    pub normalized: bool,
    pub stripped: bool,
}

impl ResolutionEvidence {
    fn deterministic(kind: ResolutionKind) -> Self {
        Self {
            kind,
            candidate_count: 1,
            price_consensus: true,
            exact_model_identity: true,
            alias_applied: false,
            normalized: false,
            stripped: false,
        }
    }

    /// Why this resolution cannot be published, or `None` when it can.
    ///
    /// Submission diagnostics report the returned gap verbatim, so this is the
    /// single place that decides both whether a row is publishable and what to
    /// say about a row that is not. Deriving the message from the same match
    /// that decides safety is what keeps a diagnostic from claiming candidates
    /// disagreed when the lookup only ever saw one.
    pub fn submission_safety_gap(&self) -> Option<SubmissionSafetyGap> {
        match self.kind {
            // These fallbacks start from a bare id and add or borrow a
            // provider-qualified row. Matching the model spelling alone does
            // not establish that the request used that provider's price.
            ResolutionKind::ModelPart | ResolutionKind::ProviderPrefix => {
                return Some(SubmissionSafetyGap::UnverifiedProviderIdentity);
            }
            ResolutionKind::Fuzzy | ResolutionKind::ProviderScoped => {}
            _ => return None,
        }
        if !self.price_consensus {
            return Some(SubmissionSafetyGap::PriceDisagreement);
        }
        if !self.exact_model_identity {
            return Some(SubmissionSafetyGap::UnverifiedModelIdentity);
        }
        None
    }

    pub fn is_submission_safe(&self) -> bool {
        self.submission_safety_gap().is_none()
    }

    /// Compose evidence for a row whose missing rates were filled from
    /// `donor`.
    ///
    /// The filled row publishes `donor`'s rate under its own key, so it can be
    /// no stronger than the resolution that rate came from. Without this, a
    /// submission-safe hinted row filled from an ambiguous fuzzy canonical row
    /// launders that ambiguity into a submitted price: the hinted row's own
    /// evidence says nothing about the borrowed bucket, and the leaderboard
    /// receives a rate the resolver had already judged too weak to publish.
    fn borrowing_from(&self, donor: &Self) -> Self {
        Self {
            // A donor that could not be published on its own is the composed
            // row's weakest link, so report its kind rather than the stronger
            // kind of the row being filled.
            kind: if donor.is_submission_safe() {
                self.kind
            } else {
                donor.kind
            },
            candidate_count: self.candidate_count.max(donor.candidate_count),
            price_consensus: self.price_consensus && donor.price_consensus,
            exact_model_identity: self.exact_model_identity && donor.exact_model_identity,
            alias_applied: self.alias_applied || donor.alias_applied,
            normalized: self.normalized || donor.normalized,
            stripped: self.stripped || donor.stripped,
        }
    }
}

/// Why a resolution is not safe to publish to the shared leaderboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionSafetyGap {
    /// The considered candidates do not publish the same rates, so the
    /// selected candidate's price is one of several conflicting answers.
    PriceDisagreement,
    /// No candidate names the requested model exactly, so the price belongs to
    /// a model that merely resembles the one that was used.
    UnverifiedModelIdentity,
    /// A bare model id resolved through another provider's qualified catalog
    /// key, without evidence that the request used that provider's price.
    UnverifiedProviderIdentity,
}

#[derive(Debug, Clone)]
pub struct LookupResult {
    pub pricing: ModelPricing,
    pub source: String,
    pub matched_key: String,
    pub evidence: ResolutionEvidence,
}

impl LookupResult {
    fn with_kind(mut self, kind: ResolutionKind) -> Self {
        self.evidence.kind = kind;
        self
    }

    fn with_alias(mut self) -> Self {
        self.evidence.alias_applied = true;
        self
    }

    fn with_normalization(mut self) -> Self {
        self.evidence.normalized = true;
        self
    }

    fn with_stripping(mut self) -> Self {
        self.evidence.stripped = true;
        self
    }
}

impl PricingLookup {
    pub fn new(
        litellm: HashMap<String, ModelPricing>,
        openrouter: HashMap<String, ModelPricing>,
        cursor: HashMap<String, ModelPricing>,
    ) -> Self {
        // Bare `new` keeps the legacy 3-source shape (no Sakana built-in
        // overrides); production wiring goes through `new_with_models_dev`
        // which threads the Sakana map alongside Cursor.
        Self::new_with_models_dev(litellm, openrouter, cursor, HashMap::new(), HashMap::new())
    }

    // @keep: the omission of cursor/sakana is the whole point and reads like a bug otherwise.
    /// True when at least one *fetchable* upstream dataset loaded.
    ///
    /// The `cursor` and `sakana` tables are compiled-in constants that are
    /// present on every run, so they are deliberately not consulted: counting
    /// them would report healthy pricing during a total upstream outage, which
    /// is exactly the condition callers use this to detect.
    pub fn has_upstream_dataset(&self) -> bool {
        !self.litellm.is_empty() || !self.openrouter.is_empty() || !self.models_dev.is_empty()
    }

    pub fn new_with_models_dev(
        litellm: HashMap<String, ModelPricing>,
        openrouter: HashMap<String, ModelPricing>,
        cursor: HashMap<String, ModelPricing>,
        sakana: HashMap<String, ModelPricing>,
        models_dev: HashMap<String, ModelPricing>,
    ) -> Self {
        // Longest key first, then alphabetical. The alphabetical leg only pins
        // equal-length ties so a run is reproducible; it carries no pricing
        // meaning, and the cheaper or more authoritative row does not win by
        // being sorted earlier.
        let mut litellm_keys: Vec<String> = litellm.keys().cloned().collect();
        litellm_keys.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

        let mut openrouter_keys: Vec<String> = openrouter.keys().cloned().collect();
        openrouter_keys.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

        let mut models_dev_keys: Vec<String> = models_dev.keys().cloned().collect();
        models_dev_keys.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

        let mut litellm_lower = HashMap::with_capacity(litellm.len());
        for key in &litellm_keys {
            litellm_lower.insert(key.to_lowercase(), key.clone());
        }

        let mut openrouter_lower = HashMap::with_capacity(openrouter.len());
        let mut openrouter_model_part = HashMap::with_capacity(openrouter.len());
        for key in &openrouter_keys {
            let lower = key.to_lowercase();
            openrouter_lower.insert(lower.clone(), key.clone());
            if let Some(model_part) = lower.split('/').next_back() {
                if model_part != lower {
                    openrouter_model_part.insert(model_part.to_string(), key.clone());
                }
            }
        }

        let mut models_dev_lower = HashMap::with_capacity(models_dev.len());
        let mut models_dev_model_part: HashMap<String, String> =
            HashMap::with_capacity(models_dev.len());
        for key in &models_dev_keys {
            let lower = key.to_lowercase();
            models_dev_lower.insert(lower.clone(), key.clone());
            // Only priced entries enter the model-part index: the
            // deterministic anthropic-first preference must choose among
            // keys that can actually price usage, otherwise an unpriced
            // `anthropic/<model>` row would shadow a priced reseller row
            // and bill the model at zero cost. (The models.dev loader only
            // emits entries with input+output costs — see
            // `models_dev::cost_to_pricing` — but this constructor is
            // public, so the index guards itself too.)
            if !models_dev.get(key).is_some_and(has_any_usable_pricing) {
                continue;
            }
            if let Some(model_part) = lower.split('/').next_back() {
                if model_part != lower {
                    match models_dev_model_part.entry(model_part.to_string()) {
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            if prefers_model_part_key(key, entry.get()) {
                                entry.insert(key.clone());
                            }
                        }
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(key.clone());
                        }
                    }
                }
            }
        }

        let mut cursor_lower = HashMap::with_capacity(cursor.len());
        for key in cursor.keys() {
            cursor_lower.insert(key.to_lowercase(), key.clone());
        }

        let mut sakana_lower = HashMap::with_capacity(sakana.len());
        for key in sakana.keys() {
            sakana_lower.insert(key.to_lowercase(), key.clone());
        }

        let build_key_parts = |keys: &[String]| -> Vec<KeyModelPart> {
            keys.iter()
                .map(|key| {
                    let lower = key.to_lowercase();
                    let model_part = lower.split('/').next_back().unwrap_or(&lower).to_string();
                    KeyModelPart {
                        key: key.clone(),
                        lower_model_part: model_part,
                    }
                })
                .collect()
        };

        let litellm_key_parts = build_key_parts(&litellm_keys);
        let openrouter_key_parts = build_key_parts(&openrouter_keys);
        let models_dev_key_parts = build_key_parts(&models_dev_keys);

        Self {
            litellm,
            openrouter,
            cursor,
            sakana,
            models_dev,
            litellm_keys,
            openrouter_keys,
            litellm_key_parts,
            openrouter_key_parts,
            models_dev_key_parts,
            litellm_lower,
            openrouter_lower,
            models_dev_lower,
            openrouter_model_part,
            models_dev_model_part,
            cursor_lower,
            sakana_lower,
            lookup_cache: RwLock::new(HashMap::with_capacity(64)),
        }
    }

    pub fn lookup(&self, model_id: &str) -> Option<LookupResult> {
        self.lookup_with_provider(model_id, None)
    }

    pub fn lookup_with_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        let provider_id = normalize_provider_hint(provider_id);
        let cache_key = build_lookup_cache_key(model_id, provider_id);
        if let Some(cached) = self
            .lookup_cache
            .read()
            .ok()
            .and_then(|c| c.get(&cache_key).cloned())
        {
            return cached.map(|c| LookupResult {
                pricing: c.pricing,
                source: c.source,
                matched_key: c.matched_key,
                evidence: c.evidence,
            });
        }

        let result = self.lookup_with_source_and_provider(model_id, None, provider_id);

        if let Ok(mut cache) = self.lookup_cache.write() {
            if cache.len() >= MAX_LOOKUP_CACHE_ENTRIES {
                // Evict ~25% of entries instead of clearing everything.
                // This avoids a thundering-herd cache miss storm that happens
                // when clear() wipes all entries at once.
                let evict_count = cache.len() / 4;
                let keys_to_remove: Vec<String> = cache.keys().take(evict_count).cloned().collect();
                for key in keys_to_remove {
                    cache.remove(&key);
                }
            }
            cache.insert(
                cache_key,
                result.as_ref().map(|r| CachedResult {
                    pricing: r.pricing.clone(),
                    source: r.source.clone(),
                    matched_key: r.matched_key.clone(),
                    evidence: r.evidence.clone(),
                }),
            );
        }

        result
    }

    pub fn lookup_with_source(
        &self,
        model_id: &str,
        force_source: Option<&str>,
    ) -> Option<LookupResult> {
        self.lookup_with_source_and_provider(model_id, force_source, None)
    }

    pub fn lookup_with_source_and_provider(
        &self,
        model_id: &str,
        force_source: Option<&str>,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        // A router is not a model. Resolving one by model-part match elects
        // whatever unrelated vendor publishes the same word, and the result is
        // billed as if it were the real thing (#1062).
        if is_routing_label(model_id) {
            return None;
        }

        let provider_id = normalize_provider_hint(provider_id);
        let resolved_alias = aliases::resolve_alias(model_id);
        let canonical = resolved_alias.unwrap_or(model_id);
        let alias_applied = resolved_alias.is_some();
        let lower = canonical.to_lowercase();

        // CLIProxyAPI strips `(level)` reasoning-effort suffixes before routing,
        // so for pricing lookup we resolve to the base model regardless of tier.
        // Mirrors the dash-suffix path (e.g. `-xhigh`), which is handled by
        // `try_strip_unknown_suffix` below.
        let normalized_owned = strip_parenthesized_reasoning_tier(&lower).map(str::to_owned);

        // A tier suffix does not turn a router into a model: `auto(high)`
        // normalizes to `auto` below and would otherwise reach the model-part
        // fallback and elect an unrelated vendor, exactly as the bare form did.
        if normalized_owned.as_deref().is_some_and(is_routing_label) {
            return None;
        }

        // Guard against silent misresolution: if the input ends with `(...)`
        // but the contents are not a recognized CLIProxyAPI level, refuse the
        // lookup. Falling through to `try_strip_unknown_suffix` would split on
        // `-` and could match a shorter, unrelated model id by peeling the
        // parenthesized fragment off (e.g. `gpt-5.2-codex(invalid)` would
        // strip `-codex(invalid)` and resolve to `gpt-5.2`).
        if normalized_owned.is_none()
            && lower
                .strip_suffix(')')
                .and_then(|inner| inner.rsplit_once('('))
                .is_some()
        {
            return None;
        }

        let lower_ref: &str = normalized_owned.as_deref().unwrap_or(&lower);

        // Helper to perform lookup with the given source constraint
        let do_lookup = |id: &str| match force_source {
            Some("litellm") => self.lookup_litellm_only(id, provider_id),
            Some("openrouter") => self.lookup_openrouter_only(id, provider_id),
            Some("models.dev") | Some("modelsdev") | Some("models_dev") => {
                self.lookup_models_dev_only(id, provider_id)
            }
            _ => self.lookup_auto(id, provider_id),
        };
        let requested_family = claude_family(lower_ref);
        let requested_version = requested_claude_version(lower_ref);
        let unparsed_modern_version = requested_family.is_some()
            && requested_version.is_none()
            && contains_delimited_modern_major_minor(lower_ref);
        let unsafe_claude_resolution = |result: &LookupResult| {
            resolves_unsafe_claude_version(
                requested_family,
                requested_version.as_deref(),
                unparsed_modern_version,
                result,
            )
        };

        let annotate_direct = |mut result: LookupResult| {
            if alias_applied {
                result = result.with_alias();
            }
            if normalized_owned.is_some() {
                result = result.with_normalization();
            }
            result
        };

        // 1. Try direct lookup
        if let Some(result) = do_lookup(lower_ref).map(&annotate_direct) {
            if unsafe_claude_resolution(&result) {
                return None;
            }
            return Some(result);
        }

        if parse_provider_scoped_model_path(lower_ref).is_some() {
            return None;
        }

        let guarded_lookup = |candidate: &str| {
            do_lookup(candidate)
                .map(&annotate_direct)
                .map(LookupResult::with_stripping)
                .filter(|result| !unsafe_claude_resolution(result))
        };

        // 1.5. Generic provider-routing prefix fallback: ids coming from a
        // router/proxy (e.g. `cx/gpt-5.5` via an `omniroute` provider) carry a
        // prefix outside the curated `PROVIDER_PREFIXES` list, so the
        // known-prefix stripping inside `lookup_auto` never fires for them.
        // The direct exact lookup above already had first crack at the full
        // id, so a dataset key that legitimately keeps its prefix (e.g.
        // `anthropic/claude-fable-5`) resolves there and never reaches this
        // fallback. Only the terminal path segment is retried here, matching
        // the `/`-scoped fallbacks already used by the Cursor/Sakana exact
        // matchers.
        if let Some(terminal) = strip_generic_provider_prefix(lower_ref) {
            // Reaching here means no dataset key matched the qualified id, so
            // an unrecognized vendor prefix is being dropped to retry the bare
            // model. A real `morph/auto` resolved long before this point; a
            // made-up `cx/auto` would arrive here and be billed as Morph.
            if is_routing_label(terminal) {
                return None;
            }

            if let Some(result) = guarded_lookup(terminal) {
                return Some(result);
            }

            // The terminal segment can still carry a tier suffix, and the two
            // transformations have to compose here or they never meet: the
            // suffix stage below only ever sees the prefixed id, and it splits
            // on `-`, so it can peel `-xhigh` off `cx/gpt-5.5-xhigh` but is
            // left with `cx/gpt-5.5`, which is not a dataset key either. Both
            // halves resolve alone while the combination billed $0 (#846).
            if let Some(result) = try_strip_unknown_suffix(terminal, guarded_lookup) {
                return Some(result);
            }
        }

        // 2. Try stripping unknown suffixes (e.g., -thinking, -high, -codex)
        if let Some(result) = try_strip_unknown_suffix(lower_ref, guarded_lookup) {
            return Some(result);
        }

        // 3. Try stripping unknown prefixes (e.g., antigravity-, myplugin-)
        //    For each prefix candidate, also try suffix stripping
        if let Some(result) = try_strip_unknown_prefix(lower_ref, guarded_lookup) {
            return Some(result);
        }

        None
    }

    fn lookup_auto(&self, model_id: &str, provider_id: Option<&str>) -> Option<LookupResult> {
        if let Some(result) = self.lookup_provider_scoped_path(model_id, provider_id) {
            return Some(scope_resolution_to_provider(result, model_id));
        }
        if parse_provider_scoped_model_path(model_id).is_some() {
            return None;
        }

        if let Some(stripped) = strip_known_provider_prefix(model_id) {
            let prefix_matches_hint =
                provider_id.is_none() || model_prefix_matches_provider(model_id, provider_id);

            if prefix_matches_hint {
                if let Some(exact_litellm) = self.exact_match_litellm(model_id) {
                    return Some(exact_litellm);
                }

                let exact_openrouter = self.exact_match_openrouter(model_id);
                let stripped_litellm = self
                    .exact_or_normalized_litellm(stripped, provider_id)
                    .map(LookupResult::with_stripping);

                if let (Some(litellm), Some(openrouter)) = (&stripped_litellm, &exact_openrouter) {
                    if has_meaningful_tier_support(&litellm.pricing)
                        && !has_any_valid_above_tier_value(&openrouter.pricing)
                    {
                        return stripped_litellm;
                    }
                }

                if let Some(result) = exact_openrouter {
                    return Some(result);
                }
                if let Some(result) = stripped_litellm {
                    return Some(result);
                }
                if let Some(result) = self.exact_match_models_dev(model_id) {
                    return Some(result);
                }
                if let Some(result) =
                    self.exact_match_models_dev_with_provider(stripped, provider_id)
                {
                    return Some(result.with_stripping());
                }
            } else {
                if let Some(result) = choose_best_source_result_with_models_dev(
                    self.exact_match_litellm_for_provider(stripped, provider_id),
                    self.exact_match_openrouter_for_provider(stripped, provider_id),
                    self.exact_match_models_dev_for_provider(stripped, provider_id),
                    provider_id,
                ) {
                    return Some(result.with_stripping());
                }
                if let Some(result) = self.exact_or_normalized_litellm(stripped, provider_id) {
                    return Some(result.with_stripping());
                }
                if let Some(result) =
                    self.exact_match_models_dev_with_provider(stripped, provider_id)
                {
                    return Some(result.with_stripping());
                }
            }
        }

        let exact_litellm = self.exact_match_litellm(model_id);
        if should_prefer_openai_tiered_litellm(model_id, provider_id, exact_litellm.as_ref()) {
            return exact_litellm;
        }

        if let Some(result) = choose_best_source_result_with_models_dev(
            self.exact_match_litellm_for_provider(model_id, provider_id),
            self.exact_match_openrouter_for_provider(model_id, provider_id),
            self.exact_match_models_dev_for_provider(model_id, provider_id),
            provider_id,
        ) {
            return Some(result);
        }

        if let Some(result) = exact_litellm {
            return Some(result);
        }
        // An unscoped OpenRouter FULL-KEY match is the id's own canonical key,
        // so it wins even under a provider hint. The MODEL-PART fallback does
        // not: it matches "some other provider's model whose model-part equals
        // this id", which is exactly what a provider hint must override.
        if let Some(result) = self.exact_match_openrouter_full_key(model_id) {
            return Some(result);
        }

        // A provider hint pins the lookup to that provider's catalog: the
        // provider-scoped models.dev pass must run before BOTH the unscoped
        // OpenRouter model-part fallback here and the separator-normalized
        // fallback below. Otherwise a hinted lookup (e.g. `venice` + dotted
        // `claude-opus-4.6-fast`, which already matches OpenRouter's
        // `anthropic/claude-opus-4.6-fast` model-part) would take the canonical
        // price instead of the hinted provider's own key. A hint with no
        // matching key falls through to the canonical resolution below.
        if provider_id.is_some() {
            if let Some(result) = self.exact_match_models_dev_for_provider(model_id, provider_id) {
                return Some(result);
            }
        }
        if let Some(result) = self.exact_match_openrouter_model_part(model_id) {
            return Some(result);
        }

        // Separator-normalized exact passes against the canonical sources
        // (LiteLLM + OpenRouter) run BEFORE the models.dev model-part pass so
        // ids like `claude-opus-4-6-fast` hit the canonical
        // `anthropic/claude-opus-4.6-fast` key instead of a reseller's
        // `venice/claude-opus-4-6-fast` markup. models.dev stays the
        // long-tail fallback below. This reorder only preempts models.dev
        // for UNhinted lookups: the provider-scoped passes above and below
        // keep provider-hinted resolutions pinned to the hinted provider.
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = choose_best_source_result_with_models_dev(
                self.exact_match_litellm_for_provider(&version_normalized, provider_id),
                self.exact_match_openrouter_for_provider(&version_normalized, provider_id),
                self.exact_match_models_dev_for_provider(&version_normalized, provider_id),
                provider_id,
            ) {
                return Some(result.with_normalization());
            }
            if provider_id.is_some() {
                if let Some(result) =
                    self.exact_match_models_dev_for_provider(&version_normalized, provider_id)
                {
                    return Some(result.with_normalization());
                }
            }
            if let Some(result) = self.exact_match_litellm(&version_normalized) {
                return Some(result.with_normalization());
            }
            if let Some(result) = self.exact_match_openrouter(&version_normalized) {
                return Some(result.with_normalization());
            }
        }

        if let Some(result) = self.exact_match_models_dev_with_provider(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) =
                self.exact_match_models_dev_with_provider(&version_normalized, provider_id)
            {
                return Some(result.with_normalization());
            }
        }

        if let Some(normalized) = normalize_model_name(model_id) {
            if let Some(result) = choose_best_source_result_with_models_dev(
                self.exact_match_litellm_for_provider(&normalized, provider_id),
                self.exact_match_openrouter_for_provider(&normalized, provider_id),
                self.exact_match_models_dev_for_provider(&normalized, provider_id),
                provider_id,
            ) {
                return Some(result.with_normalization());
            }
            if let Some(result) = self.exact_match_litellm(&normalized) {
                return Some(result.with_normalization());
            }
            if let Some(result) = self.exact_match_openrouter(&normalized) {
                return Some(result.with_normalization());
            }
            if let Some(result) =
                self.exact_match_models_dev_with_provider(&normalized, provider_id)
            {
                return Some(result.with_normalization());
            }
        }

        if let Some(result) = self.prefix_match_litellm(model_id, provider_id) {
            return Some(result);
        }
        if let Some(result) = self.prefix_match_openrouter(model_id, provider_id) {
            return Some(result);
        }
        if let Some(result) = self.prefix_match_models_dev(model_id, provider_id) {
            return Some(result);
        }

        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.prefix_match_litellm(&version_normalized, provider_id) {
                return Some(result.with_normalization());
            }
            if let Some(result) = self.prefix_match_openrouter(&version_normalized, provider_id) {
                return Some(result.with_normalization());
            }
            if let Some(result) = self.prefix_match_models_dev(&version_normalized, provider_id) {
                return Some(result.with_normalization());
            }
        }

        if let Some(result) = self.exact_match_cursor(model_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.exact_match_cursor(&version_normalized) {
                return Some(result.with_normalization());
            }
        }

        // Sakana built-in overrides sit at the SAME precedence as Cursor:
        // upstream real prices (litellm/openrouter/models.dev exact + prefix)
        // already won above, so Sakana only catches ids upstream doesn't price,
        // while still beating the fuzzy guesses below.
        if let Some(result) = self.exact_match_sakana(model_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.exact_match_sakana(&version_normalized) {
                return Some(result.with_normalization());
            }
        }

        if !is_fuzzy_eligible(model_id) {
            return None;
        }

        let litellm_result = self.fuzzy_match_litellm(model_id, provider_id);
        let openrouter_result = self.fuzzy_match_openrouter(model_id, provider_id);
        let fuzzy_results = [litellm_result.as_ref(), openrouter_result.as_ref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let candidate_count = fuzzy_results
            .iter()
            .map(|result| result.evidence.candidate_count)
            .sum();
        let price_consensus = fuzzy_results.first().is_some_and(|first| {
            first.evidence.price_consensus
                && fuzzy_results.iter().skip(1).all(|result| {
                    result.evidence.price_consensus
                        && pricing_rows_equal(&first.pricing, &result.pricing)
                })
        });
        let exact_model_identity = !fuzzy_results.is_empty()
            && fuzzy_results
                .iter()
                .all(|result| result.evidence.exact_model_identity);

        choose_best_source_result(litellm_result, openrouter_result, provider_id).map(
            |mut result| {
                result.evidence.candidate_count = candidate_count;
                result.evidence.price_consensus = price_consensus;
                result.evidence.exact_model_identity = exact_model_identity;
                result
            },
        )
    }

    fn exact_or_normalized_litellm(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if let Some(result) = self.exact_match_litellm_for_provider(model_id, provider_id) {
            return Some(result);
        }
        if let Some(result) = self.exact_match_litellm(model_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) =
                self.exact_match_litellm_for_provider(&version_normalized, provider_id)
            {
                return Some(result.with_normalization());
            }
            if let Some(result) = self.exact_match_litellm(&version_normalized) {
                return Some(result.with_normalization());
            }
        }
        if let Some(normalized) = normalize_model_name(model_id) {
            if let Some(result) = self.exact_match_litellm_for_provider(&normalized, provider_id) {
                return Some(result.with_normalization());
            }
            if let Some(result) = self.exact_match_litellm(&normalized) {
                return Some(result.with_normalization());
            }
        }
        None
    }

    fn lookup_models_dev_only(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if parse_provider_scoped_model_path(model_id).is_some() {
            return None;
        }

        if let Some(result) = self.exact_match_models_dev_with_provider(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) =
                self.exact_match_models_dev_with_provider(&version_normalized, provider_id)
            {
                return Some(result.with_normalization());
            }
        }
        if let Some(normalized) = normalize_model_name(model_id) {
            if let Some(result) =
                self.exact_match_models_dev_with_provider(&normalized, provider_id)
            {
                return Some(result.with_normalization());
            }
        }
        if let Some(result) = self.prefix_match_models_dev(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.prefix_match_models_dev(&version_normalized, provider_id) {
                return Some(result.with_normalization());
            }
        }
        None
    }

    fn lookup_litellm_only(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if let Some(result) = self.lookup_provider_scoped_path_litellm(model_id, provider_id) {
            return Some(scope_resolution_to_provider(result, model_id));
        }
        if parse_provider_scoped_model_path(model_id).is_some() {
            return None;
        }

        if let Some(result) = self.exact_or_normalized_litellm(model_id, provider_id) {
            return Some(result);
        }
        if let Some(stripped) = strip_known_provider_prefix(model_id) {
            if let Some(result) = self.exact_or_normalized_litellm(stripped, provider_id) {
                return Some(result.with_stripping());
            }
        }
        if let Some(result) = self.prefix_match_litellm(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.prefix_match_litellm(&version_normalized, provider_id) {
                return Some(result.with_normalization());
            }
        }
        if is_fuzzy_eligible(model_id) {
            if let Some(result) = self.fuzzy_match_litellm(model_id, provider_id) {
                return Some(result);
            }
        }
        None
    }

    fn lookup_openrouter_only(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if let Some(result) = self.lookup_provider_scoped_path_openrouter(model_id, provider_id) {
            return Some(scope_resolution_to_provider(result, model_id));
        }
        if parse_provider_scoped_model_path(model_id).is_some() {
            return None;
        }

        if let Some(result) = self.exact_match_openrouter_with_provider(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) =
                self.exact_match_openrouter_with_provider(&version_normalized, provider_id)
            {
                return Some(result.with_normalization());
            }
        }
        if let Some(normalized) = normalize_model_name(model_id) {
            if let Some(result) =
                self.exact_match_openrouter_with_provider(&normalized, provider_id)
            {
                return Some(result.with_normalization());
            }
        }
        if let Some(result) = self.prefix_match_openrouter(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.prefix_match_openrouter(&version_normalized, provider_id) {
                return Some(result.with_normalization());
            }
        }
        if is_fuzzy_eligible(model_id) {
            if let Some(result) = self.fuzzy_match_openrouter(model_id, provider_id) {
                return Some(result);
            }
        }
        None
    }

    fn lookup_provider_scoped_path(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        let scoped = parse_provider_scoped_model_path(model_id)?;
        if !provider_hint_matches_scoped_provider(provider_id, scoped.provider) {
            return None;
        }

        choose_best_source_result(
            self.lookup_provider_scoped_path_litellm(model_id, provider_id),
            self.lookup_provider_scoped_path_openrouter(model_id, provider_id),
            Some(scoped.provider),
        )
    }

    fn lookup_provider_scoped_path_litellm(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        let scoped = parse_provider_scoped_model_path(model_id)?;
        if !provider_hint_matches_scoped_provider(provider_id, scoped.provider) {
            return None;
        }

        if let Some(result) = self.exact_match_litellm(model_id) {
            return Some(result);
        }

        let scoped_tags = provider_identity::provider_tags(scoped.provider);
        for prefix in RESELLER_PROVIDER_PREFIXES {
            if !provider_prefix_matches_scoped_provider(prefix, &scoped_tags) {
                continue;
            }

            let key = format!("{}{}", prefix, model_id);
            if let Some(litellm_key) = self.litellm_lower.get(&key) {
                if let Some(pricing) = self.litellm.get(litellm_key) {
                    if let Some(result) = lookup_result_if_usable(pricing, "LiteLLM", litellm_key) {
                        return Some(result);
                    }
                }
            }
        }

        self.exact_match_litellm_for_provider(scoped.terminal_model_id, Some(scoped.provider))
    }

    fn lookup_provider_scoped_path_openrouter(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        let scoped = parse_provider_scoped_model_path(model_id)?;
        if !provider_hint_matches_scoped_provider(provider_id, scoped.provider) {
            return None;
        }

        self.exact_match_openrouter(model_id).or_else(|| {
            self.exact_match_openrouter_for_provider(
                scoped.terminal_model_id,
                Some(scoped.provider),
            )
        })
    }

    fn exact_match_litellm_for_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        exact_match_with_provider_prefixes(
            model_id,
            provider_id,
            &self.litellm_key_parts,
            &self.litellm,
            "LiteLLM",
        )
    }

    fn exact_match_openrouter_for_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        exact_match_with_provider_prefixes(
            model_id,
            provider_id,
            &self.openrouter_key_parts,
            &self.openrouter,
            "OpenRouter",
        )
    }

    fn exact_match_openrouter_with_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        self.exact_match_openrouter_for_provider(model_id, provider_id)
            .or_else(|| self.exact_match_openrouter(model_id))
    }

    fn exact_match_models_dev_for_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        exact_match_with_provider_prefixes(
            model_id,
            provider_id,
            &self.models_dev_key_parts,
            &self.models_dev,
            "Models.dev",
        )
    }

    fn exact_match_models_dev_with_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        self.exact_match_models_dev_for_provider(model_id, provider_id)
            .or_else(|| self.exact_match_models_dev(model_id))
    }

    fn exact_match_litellm(&self, model_id: &str) -> Option<LookupResult> {
        let key = self.litellm_lower.get(model_id)?;
        let pricing = self.litellm.get(key)?;
        lookup_result_if_usable(pricing, "LiteLLM", key)
    }

    fn exact_match_openrouter(&self, model_id: &str) -> Option<LookupResult> {
        self.exact_match_openrouter_full_key(model_id)
            .or_else(|| self.exact_match_openrouter_model_part(model_id))
    }

    /// Full-key (`provider/model`) exact match against OpenRouter — the id's
    /// own canonical key. This wins even under a provider hint.
    fn exact_match_openrouter_full_key(&self, model_id: &str) -> Option<LookupResult> {
        let key = self.openrouter_lower.get(model_id)?;
        let pricing = self.openrouter.get(key)?;
        lookup_result_if_usable(pricing, "OpenRouter", key)
    }

    /// Model-part exact match against OpenRouter — matches any provider whose
    /// model-part equals `model_id`. A provider hint must take precedence over
    /// this (see `lookup_auto`), otherwise a hinted lookup leaks to a different
    /// provider's canonical key.
    ///
    /// The model-part index is a cross-provider fallback in the same trust
    /// class as fuzzy matching: it lands the id on "some other provider's
    /// model whose model-part equals this id". Generic tokens on the
    /// `FUZZY_BLOCKLIST` carry no model identity, and #1070's resolver-top
    /// `is_routing_label` guard already refuses the router labels it knows
    /// (`auto`, `agent_review`). This blocklist gate is the second layer:
    /// it covers generic tokens no parser emits today but any provider could
    /// publish as a model part tomorrow (`default`, `router`, `mini`, ...),
    /// and it protects any path that reaches the model-part index without
    /// passing through that guard. Full-key matches, which are the id's own
    /// canonical key, stay honored.
    fn exact_match_openrouter_model_part(&self, model_id: &str) -> Option<LookupResult> {
        if FUZZY_BLOCKLIST.contains(&model_id) {
            return None;
        }
        let key = self.openrouter_model_part.get(model_id)?;
        let pricing = self.openrouter.get(key)?;
        lookup_result_if_usable(pricing, "OpenRouter", key)
            .map(|result| result.with_kind(ResolutionKind::ModelPart))
    }

    fn exact_match_models_dev(&self, model_id: &str) -> Option<LookupResult> {
        if let Some(key) = self.models_dev_lower.get(model_id) {
            if let Some(pricing) = self.models_dev.get(key) {
                return Some(LookupResult {
                    pricing: pricing.clone(),
                    source: "Models.dev".into(),
                    matched_key: key.clone(),
                    evidence: ResolutionEvidence::deterministic(ResolutionKind::Exact),
                });
            }
        }
        // Same cross-provider fallback trust class as the OpenRouter model-part
        // index: #1070's resolver-top guard plus this blocklist gate keep bare
        // generic tokens off another provider's model part, while the id's own
        // full dataset key (`morph/auto`) still resolves.
        if !FUZZY_BLOCKLIST.contains(&model_id) {
            if let Some(key) = self.models_dev_model_part.get(model_id) {
                if let Some(pricing) = self.models_dev.get(key) {
                    return Some(LookupResult {
                        pricing: pricing.clone(),
                        source: "Models.dev".into(),
                        matched_key: key.clone(),
                        evidence: ResolutionEvidence::deterministic(ResolutionKind::ModelPart),
                    });
                }
            }
        }
        None
    }

    fn exact_match_cursor(&self, model_id: &str) -> Option<LookupResult> {
        if let Some(key) = self.cursor_lower.get(model_id) {
            return lookup_result_if_usable(self.cursor.get(key).unwrap(), "Cursor", key)
                .map(|result| result.with_kind(ResolutionKind::BuiltIn));
        }
        if let Some(model_part) = model_id.split('/').next_back() {
            if model_part != model_id {
                if let Some(key) = self.cursor_lower.get(model_part) {
                    return lookup_result_if_usable(self.cursor.get(key).unwrap(), "Cursor", key)
                        .map(|result| result.with_kind(ResolutionKind::BuiltIn));
                }
            }
        }
        None
    }

    fn exact_match_sakana(&self, model_id: &str) -> Option<LookupResult> {
        if let Some(key) = self.sakana_lower.get(model_id) {
            return lookup_result_if_usable(self.sakana.get(key).unwrap(), "Sakana", key)
                .map(|result| result.with_kind(ResolutionKind::BuiltIn));
        }
        if let Some(model_part) = model_id.split('/').next_back() {
            if model_part != model_id {
                if let Some(key) = self.sakana_lower.get(model_part) {
                    return lookup_result_if_usable(self.sakana.get(key).unwrap(), "Sakana", key)
                        .map(|result| result.with_kind(ResolutionKind::BuiltIn));
                }
            }
        }
        None
    }

    fn prefix_match_litellm(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if let Some(result) = self.exact_match_litellm_for_provider(model_id, provider_id) {
            return Some(result);
        }

        for prefix in PROVIDER_PREFIXES {
            let key = format!("{}{}", prefix, model_id);
            if let Some(litellm_key) = self.litellm_lower.get(&key) {
                if let Some(pricing) = self.litellm.get(litellm_key) {
                    if let Some(result) = lookup_result_if_usable(pricing, "LiteLLM", litellm_key) {
                        return Some(result.with_kind(ResolutionKind::ProviderPrefix));
                    }
                }
            }
        }
        None
    }

    fn prefix_match_openrouter(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if let Some(result) = self.exact_match_openrouter_for_provider(model_id, provider_id) {
            return Some(result);
        }

        for prefix in PROVIDER_PREFIXES {
            let key = format!("{}{}", prefix, model_id);
            if let Some(or_key) = self.openrouter_lower.get(&key) {
                if let Some(pricing) = self.openrouter.get(or_key) {
                    if let Some(result) = lookup_result_if_usable(pricing, "OpenRouter", or_key) {
                        return Some(result.with_kind(ResolutionKind::ProviderPrefix));
                    }
                }
            }
        }
        None
    }

    fn prefix_match_models_dev(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if let Some(result) = self.exact_match_models_dev_for_provider(model_id, provider_id) {
            return Some(result);
        }

        for prefix in PROVIDER_PREFIXES {
            let key = format!("{}{}", prefix, model_id);
            if let Some(models_dev_key) = self.models_dev_lower.get(&key) {
                if let Some(pricing) = self.models_dev.get(models_dev_key) {
                    return Some(LookupResult {
                        pricing: pricing.clone(),
                        source: "Models.dev".into(),
                        matched_key: models_dev_key.clone(),
                        evidence: ResolutionEvidence::deterministic(ResolutionKind::ProviderPrefix),
                    });
                }
            }
        }
        None
    }

    fn fuzzy_match_litellm(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        let family = extract_model_family(model_id);
        let mut family_matches_list: Vec<&String> = Vec::new();

        for key in &self.litellm_keys {
            let lower_key = key.to_lowercase();
            if family_matches(&lower_key, &family) && contains_model_id(&lower_key, model_id) {
                family_matches_list.push(key);
            }
        }

        if let Some(result) = select_best_match(
            &family_matches_list,
            &self.litellm,
            "LiteLLM",
            provider_id,
            ResolutionKind::Fuzzy,
            model_id,
        ) {
            return Some(result);
        }

        let mut all_matches: Vec<&String> = Vec::new();
        for key in &self.litellm_keys {
            let lower_key = key.to_lowercase();
            if contains_model_id(&lower_key, model_id) {
                all_matches.push(key);
            }
        }

        select_best_match(
            &all_matches,
            &self.litellm,
            "LiteLLM",
            provider_id,
            ResolutionKind::Fuzzy,
            model_id,
        )
    }

    fn fuzzy_match_openrouter(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        let family = extract_model_family(model_id);
        let mut family_matches_list: Vec<&String> = Vec::new();

        for key in &self.openrouter_keys {
            let lower_key = key.to_lowercase();
            let model_part = lower_key.split('/').next_back().unwrap_or(&lower_key);
            if family_matches(model_part, &family) && contains_model_id(model_part, model_id) {
                family_matches_list.push(key);
            }
        }

        if let Some(result) = select_best_match(
            &family_matches_list,
            &self.openrouter,
            "OpenRouter",
            provider_id,
            ResolutionKind::Fuzzy,
            model_id,
        ) {
            return Some(result);
        }

        let mut all_matches: Vec<&String> = Vec::new();
        for key in &self.openrouter_keys {
            let lower_key = key.to_lowercase();
            let model_part = lower_key.split('/').next_back().unwrap_or(&lower_key);
            if contains_model_id(model_part, model_id) {
                all_matches.push(key);
            }
        }

        select_best_match(
            &all_matches,
            &self.openrouter,
            "OpenRouter",
            provider_id,
            ResolutionKind::Fuzzy,
            model_id,
        )
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
        let provider_id = normalize_provider_hint(provider_id);
        let result = match self.resolve_for_usage(model_id, provider_id, usage) {
            Some(r) => r,
            None => return 0.0,
        };

        compute_cost_for_lookup(&result, provider_id, usage)
    }

    /// Resolve `model_id` for pricing `usage`, borrowing the rates the
    /// provider-hinted row omits from the canonical unhinted row.
    ///
    /// A provider hint can steer resolution onto a gateway or reseller key
    /// that lists input and output rates only — OpenRouter's
    /// `openai/gpt-5.2-codex` and LiteLLM's `gmi/google/gemini-3-pro-preview`
    /// both do — while the canonical key for the same model publishes the
    /// cache rates as well. Pricing the hinted row alone bills cached tokens
    /// at zero and makes `covers_usage` false, which aborted whole
    /// submissions for every Codex session (#1013).
    ///
    /// Only buckets the hinted row cannot price are filled, so a reseller row
    /// keeps its own markup rather than silently repricing to the author's
    /// cheaper rate. If the filled row still cannot cover the usage, the
    /// hinted row is returned unchanged and the usage stays unpriced.
    pub(super) fn resolve_for_usage(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
        usage: &TokenBreakdown,
    ) -> Option<LookupResult> {
        let hinted = self.lookup_with_provider(model_id, provider_id)?;
        if normalize_provider_hint(provider_id).is_none() || hinted.pricing.covers_usage(usage) {
            return Some(hinted);
        }

        let Some(canonical) = self.lookup_with_provider(model_id, None) else {
            return Some(hinted);
        };
        if canonical.matched_key == hinted.matched_key
            || !quote_same_base_rates(&hinted.pricing, &canonical.pricing)
        {
            return Some(hinted);
        }

        let filled = hinted
            .pricing
            .with_missing_rates_from(&canonical.pricing, usage);
        if !filled.covers_usage(usage) {
            return Some(hinted);
        }

        // Keep the hinted row's source and matched key: `compute_cost_for_lookup`
        // branches on both for OpenAI's full-request 272k tiering, so borrowing
        // rates must not change which pricing model applies. The evidence is
        // composed rather than kept, because the filled row now quotes the
        // canonical row's rates and has to be judged on the weaker of the two
        // resolutions. The estimate stays visible either way; only its
        // publishability changes.
        let evidence = hinted.evidence.borrowing_from(&canonical.evidence);
        Some(LookupResult {
            pricing: filled,
            evidence,
            ..hinted
        })
    }
}

/// Whether two rows price the same deal, judged on the base rates they both
/// publish.
///
/// Borrowing a rate across rows that disagree would invent a tariff neither
/// provider charges: `azure_ai/grok-code-fast-1` bills $3.50/$17.50 per
/// million with no cache-read rate, while the canonical `xai/` row bills
/// $0.20/$1.50 with one, so an Azure row must never inherit xAI's cache
/// price. Rows must also agree on at least one bucket — without a single
/// shared rate there is no evidence they describe the same deal at all.
fn quote_same_base_rates(hinted: &ModelPricing, canonical: &ModelPricing) -> bool {
    let mut shared = false;

    for (hinted_rate, canonical_rate) in [
        (hinted.input_cost_per_token, canonical.input_cost_per_token),
        (
            hinted.output_cost_per_token,
            canonical.output_cost_per_token,
        ),
        (
            hinted.cache_read_input_token_cost,
            canonical.cache_read_input_token_cost,
        ),
        (
            hinted.cache_creation_input_token_cost,
            canonical.cache_creation_input_token_cost,
        ),
    ] {
        let (Some(hinted_rate), Some(canonical_rate)) = (hinted_rate, canonical_rate) else {
            continue;
        };
        if !hinted_rate.is_finite() || !canonical_rate.is_finite() {
            return false;
        }
        if (hinted_rate - canonical_rate).abs() > canonical_rate.abs() * 1e-9 {
            return false;
        }
        shared = true;
    }

    shared
}

fn matches_model_or_snapshot(model_id: &str, base: &str) -> bool {
    model_id == base
        || model_id
            .strip_prefix(base)
            .is_some_and(|suffix| suffix.starts_with("-20"))
}

fn is_openai_full_request_272k_model(model_id: &str) -> bool {
    let key = model_id.to_ascii_lowercase();
    let model_id = key.split('/').next_back().unwrap_or(&key);

    [
        "gpt-5.4",
        "gpt-5.4-pro",
        "gpt-5.5",
        // Priced identically to gpt-5.4-pro in LiteLLM ($30/$180 base,
        // $60/$270 above 272k) with the same full-request semantics.
        "gpt-5.5-pro",
        "gpt-5.6",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
    ]
    .into_iter()
    .any(|base| matches_model_or_snapshot(model_id, base))
}

fn should_prefer_openai_tiered_litellm(
    model_id: &str,
    provider_id: Option<&str>,
    litellm: Option<&LookupResult>,
) -> bool {
    provider_id.is_some_and(|provider| {
        provider_identity::canonical_provider(provider).as_deref() == Some("openai")
    }) && is_openai_full_request_272k_model(model_id)
        && litellm.is_some_and(|result| has_complete_openai_272k_pricing(&result.pricing))
}

// A fully-absent cache_read pair used to count as "complete" here (only a
// present-but-partial pair failed), which let the 272k LiteLLM preference
// fire over an OpenRouter entry that actually had cache-read pricing,
// silently dropping it. cache_read is now required present+valid like
// input/output, symmetric with them, for this preference decision only.
fn has_complete_openai_272k_pricing(pricing: &ModelPricing) -> bool {
    let valid_pair = |base: Option<f64>, above: Option<f64>| {
        base.is_some_and(is_valid_price_value) && above.is_some_and(is_valid_price_value)
    };

    valid_pair(
        pricing.input_cost_per_token,
        pricing.input_cost_per_token_above_272k_tokens,
    ) && valid_pair(
        pricing.output_cost_per_token,
        pricing.output_cost_per_token_above_272k_tokens,
    ) && valid_pair(
        pricing.cache_read_input_token_cost,
        pricing.cache_read_input_token_cost_above_272k_tokens,
    )
}

fn uses_openai_full_request_272k_pricing(result: &LookupResult, provider_id: Option<&str>) -> bool {
    if result.source != "LiteLLM"
        || is_reseller_provider(&result.matched_key)
        || provider_id.is_some_and(|provider| {
            provider_identity::canonical_provider(provider).as_deref() != Some("openai")
        })
    {
        return false;
    }

    let key = result.matched_key.to_ascii_lowercase();
    if key.contains('/') && !key.starts_with("openai/") {
        return false;
    }

    is_openai_full_request_272k_model(&key)
}

fn compute_cost_for_lookup(
    result: &LookupResult,
    provider_id: Option<&str>,
    usage: &TokenBreakdown,
) -> f64 {
    let calculate = |pricing| {
        compute_cost(
            pricing,
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
            usage.reasoning,
        )
    };
    let total_input = usage
        .input
        .max(0)
        .saturating_add(usage.cache_read.max(0))
        .saturating_add(usage.cache_write.max(0));
    if !uses_openai_full_request_272k_pricing(result, provider_id) {
        return calculate(&result.pricing);
    }

    let mut pricing = result.pricing.clone();
    if total_input <= TIERED_PRICING_THRESHOLD_272K_TOKENS as i64 {
        pricing.input_cost_per_token_above_272k_tokens = None;
        pricing.output_cost_per_token_above_272k_tokens = None;
        pricing.cache_read_input_token_cost_above_272k_tokens = None;
        return calculate(&pricing);
    }

    if let Some(high) = pricing
        .input_cost_per_token_above_272k_tokens
        .filter(|price| is_valid_price_value(*price))
    {
        let input_multiplier = pricing
            .input_cost_per_token
            .filter(|base| is_valid_price_value(*base) && *base > 0.0)
            .map(|base| high / base);
        for rate in [
            &mut pricing.input_cost_per_token,
            &mut pricing.input_cost_per_token_above_128k_tokens,
            &mut pricing.input_cost_per_token_above_200k_tokens,
            &mut pricing.input_cost_per_token_above_256k_tokens,
            &mut pricing.input_cost_per_token_above_272k_tokens,
        ] {
            *rate = Some(high);
        }

        if let (Some(multiplier), Some(cache_write_price)) = (
            input_multiplier,
            pricing
                .cache_creation_input_token_cost
                .filter(|price| is_valid_price_value(*price)),
        ) {
            let high = Some(cache_write_price * multiplier);
            pricing.cache_creation_input_token_cost = high;
            pricing.cache_creation_input_token_cost_above_200k_tokens = high;
        }
    }
    if let Some(high) = pricing
        .output_cost_per_token_above_272k_tokens
        .filter(|price| is_valid_price_value(*price))
    {
        for rate in [
            &mut pricing.output_cost_per_token,
            &mut pricing.output_cost_per_token_above_128k_tokens,
            &mut pricing.output_cost_per_token_above_200k_tokens,
            &mut pricing.output_cost_per_token_above_256k_tokens,
            &mut pricing.output_cost_per_token_above_272k_tokens,
        ] {
            *rate = Some(high);
        }
    }
    if let Some(high) = pricing
        .cache_read_input_token_cost_above_272k_tokens
        .filter(|price| is_valid_price_value(*price))
    {
        for rate in [
            &mut pricing.cache_read_input_token_cost,
            &mut pricing.cache_read_input_token_cost_above_200k_tokens,
            &mut pricing.cache_read_input_token_cost_above_272k_tokens,
        ] {
            *rate = Some(high);
        }
    }

    calculate(&pricing)
}

pub fn compute_cost(
    pricing: &ModelPricing,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
) -> f64 {
    let safe_price = |opt: Option<f64>| opt.filter(|v| is_valid_price_value(*v)).unwrap_or(0.0);
    let tiered_cost = |tokens: f64, base: Option<f64>, tiers: &[(f64, Option<f64>)]| {
        let base_price = safe_price(base);
        let mut cost = 0.0;
        let mut lower_bound = 0.0;
        let mut active_price = base_price;

        for (threshold, tier_price) in tiers {
            let Some(tier_price) = tier_price.filter(|v| is_valid_price_value(*v)) else {
                continue;
            };

            if !threshold.is_finite() || *threshold <= lower_bound {
                continue;
            }

            if tokens <= *threshold {
                return cost + (tokens - lower_bound).max(0.0) * active_price;
            }

            cost += (*threshold - lower_bound) * active_price;
            lower_bound = *threshold;
            active_price = tier_price;
        }

        cost + (tokens - lower_bound).max(0.0) * active_price
    };

    let input_clamped = input.max(0) as f64;
    let output_clamped = output.max(0).saturating_add(reasoning.max(0)) as f64;
    let cache_read_clamped = cache_read.max(0) as f64;
    let cache_write_clamped = cache_write.max(0) as f64;

    let input_cost = tiered_cost(
        input_clamped,
        pricing.input_cost_per_token,
        &[
            (
                TIERED_PRICING_THRESHOLD_128K_TOKENS,
                pricing.input_cost_per_token_above_128k_tokens,
            ),
            (
                TIERED_PRICING_THRESHOLD_200K_TOKENS,
                pricing.input_cost_per_token_above_200k_tokens,
            ),
            (
                TIERED_PRICING_THRESHOLD_256K_TOKENS,
                pricing.input_cost_per_token_above_256k_tokens,
            ),
            (
                TIERED_PRICING_THRESHOLD_272K_TOKENS,
                pricing.input_cost_per_token_above_272k_tokens,
            ),
        ],
    );
    let output_cost = tiered_cost(
        output_clamped,
        pricing.output_cost_per_token,
        &[
            (
                TIERED_PRICING_THRESHOLD_128K_TOKENS,
                pricing.output_cost_per_token_above_128k_tokens,
            ),
            (
                TIERED_PRICING_THRESHOLD_200K_TOKENS,
                pricing.output_cost_per_token_above_200k_tokens,
            ),
            (
                TIERED_PRICING_THRESHOLD_256K_TOKENS,
                pricing.output_cost_per_token_above_256k_tokens,
            ),
            (
                TIERED_PRICING_THRESHOLD_272K_TOKENS,
                pricing.output_cost_per_token_above_272k_tokens,
            ),
        ],
    );
    // Cache-read tiers stay limited to the 200k and 272k thresholds
    // because upstream LiteLLM does not currently declare 128k or 256k
    // cache-read pricing for any model. If upstream begins emitting
    // those keys, also add matching fields to `ModelPricing`,
    // `has_any_valid_above_tier_value`, and `has_meaningful_tier_support`;
    // otherwise tier walks will silently undercost long-context cache reads
    // on those models. `has_any_usable_pricing` and
    // `quotes_zero_for_every_published_rate` need no entry here: they read
    // `ModelPricing::all_rates`, whose exhaustive destructure fails to
    // compile until the new field is added there.
    let cache_read_cost = tiered_cost(
        cache_read_clamped,
        pricing.cache_read_input_token_cost,
        &[
            (
                TIERED_PRICING_THRESHOLD_200K_TOKENS,
                pricing.cache_read_input_token_cost_above_200k_tokens,
            ),
            (
                TIERED_PRICING_THRESHOLD_272K_TOKENS,
                pricing.cache_read_input_token_cost_above_272k_tokens,
            ),
        ],
    );
    let cache_write_cost = tiered_cost(
        cache_write_clamped,
        pricing.cache_creation_input_token_cost,
        &[(
            TIERED_PRICING_THRESHOLD_200K_TOKENS,
            pricing.cache_creation_input_token_cost_above_200k_tokens,
        )],
    );

    input_cost + output_cost + cache_read_cost + cache_write_cost
}

fn extract_model_family(model_id: &str) -> String {
    let lower = model_id.to_lowercase();

    if lower.contains("gpt-5") {
        return "gpt-5".into();
    }
    if lower.contains("gpt-4.1") {
        return "gpt-4.1".into();
    }
    if lower.contains("gpt-4o") {
        return "gpt-4o".into();
    }
    if lower.contains("gpt-4") {
        return "gpt-4".into();
    }
    if lower.contains("o3") {
        return "o3".into();
    }
    if lower.contains("o4") {
        return "o4".into();
    }

    if lower.contains("opus") {
        return "opus".into();
    }
    if lower.contains("sonnet") {
        return "sonnet".into();
    }
    if lower.contains("haiku") {
        return "haiku".into();
    }
    if lower.contains("claude") {
        return "claude".into();
    }

    if lower.contains("gemini-3") {
        return "gemini-3".into();
    }
    if lower.contains("gemini-2.5") {
        return "gemini-2.5".into();
    }
    if lower.contains("gemini-2") {
        return "gemini-2".into();
    }
    if lower.contains("gemini") {
        return "gemini".into();
    }

    if lower.contains("llama") {
        return "llama".into();
    }
    if lower.contains("mistral") {
        return "mistral".into();
    }
    if lower.contains("deepseek") {
        return "deepseek".into();
    }
    if lower.contains("qwen") {
        return "qwen".into();
    }

    lower
        .split(['-', '_', '.'])
        .next()
        .unwrap_or(&lower)
        .to_string()
}

fn family_matches(key: &str, family: &str) -> bool {
    if family.is_empty() {
        return true;
    }
    key.contains(family)
}

fn contains_model_id(key: &str, model_id: &str) -> bool {
    if let Some(pos) = key.find(model_id) {
        let before_ok = pos == 0 || !key[..pos].chars().last().unwrap().is_alphanumeric();
        let after_pos = pos + model_id.len();
        let after_ok =
            after_pos == key.len() || !key[after_pos..].chars().next().unwrap().is_alphanumeric();
        before_ok && after_ok
    } else {
        false
    }
}

fn normalize_model_name(model_id: &str) -> Option<String> {
    let lower = model_id.to_lowercase();
    let family = claude_family(&lower)?;

    // Modern Claude line (major >= 4): explicit single-digit minor parsed
    // straight from the id, in either order (claude-sonnet-4-6, opus-4.8,
    // claude-4-6-sonnet). New minor releases need no code change.
    if let Some(model) = normalize_claude_family_minor(&lower) {
        return Some(model);
    }

    // Never degrade: a delimited `major(-|.)minor` version whose minor was
    // not recognized above (4-60, 4-0, 5-0, dated 4-20250514) must stay
    // unresolved rather than fall through to a coarser or older key.
    if contains_delimited_modern_major_minor(&lower) {
        return None;
    }

    // Bare modern major adjacent to the family token (claude-sonnet-5,
    // opus-5, 4-opus). Resolves only via an exact dataset hit downstream.
    if let Some(model) = normalize_claude_family_bare_major(&lower) {
        return Some(model);
    }

    // Catch-alls preserved from the hardcoded matcher: a delimited `4`
    // anywhere still maps opus/sonnet to the bare 4.0 key, and the legacy
    // 3.x line uses irregular naming (family after the version, dotted 3.5).
    match family {
        "opus" if contains_delimited_fragment(&lower, "4") => Some("claude-opus-4".into()),
        "sonnet" => {
            if contains_delimited_fragment(&lower, "4") {
                Some("claude-sonnet-4".into())
            } else if contains_delimited_fragment(&lower, "3.7")
                || contains_delimited_fragment(&lower, "3-7")
            {
                Some("claude-3-7-sonnet".into())
            } else if contains_delimited_fragment(&lower, "3.5")
                || contains_delimited_fragment(&lower, "3-5")
            {
                Some("claude-3.5-sonnet".into())
            } else {
                None
            }
        }
        "haiku" => {
            if contains_delimited_fragment(&lower, "3.5")
                || contains_delimited_fragment(&lower, "3-5")
            {
                Some("claude-3.5-haiku".into())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Family tokens of the modern Claude model line.
const CLAUDE_FAMILY_TOKENS: &[&str] = &["opus", "sonnet", "haiku", "fable"];

/// The Claude family token contained in `lower`, if any.
fn claude_family(lower: &str) -> Option<&'static str> {
    CLAUDE_FAMILY_TOKENS
        .iter()
        .copied()
        .find(|family| lower.contains(family))
}

/// Modern Claude majors are single digits >= 4. The 3.x line uses irregular
/// naming and is matched explicitly by the legacy branches.
fn is_modern_claude_major(value: &str) -> bool {
    value.len() == 1 && value.as_bytes()[0].is_ascii_digit() && value.as_bytes()[0] >= b'4'
}

/// Canonical `claude-{family}-{major}-{minor}` key parsed from an id carrying
/// an explicit single-digit minor for a modern major (>= 4), in either
/// `family-major-minor` (claude-sonnet-4-6, opus-4.8) or reversed
/// `major-minor-family` (claude-4-6-sonnet, 4-8-opus) order. Generalization
/// of the former opus-only `normalize_claude_opus_4_minor` across families.
fn normalize_claude_family_minor(lower: &str) -> Option<String> {
    let parts: Vec<&str> = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect();

    for window in parts.windows(3) {
        if CLAUDE_FAMILY_TOKENS.contains(&window[0])
            && is_modern_claude_major(window[1])
            && is_single_digit_minor(window[2])
        {
            return Some(format!("claude-{}-{}-{}", window[0], window[1], window[2]));
        }
        if is_modern_claude_major(window[0])
            && is_single_digit_minor(window[1])
            && CLAUDE_FAMILY_TOKENS.contains(&window[2])
        {
            return Some(format!("claude-{}-{}-{}", window[2], window[0], window[1]));
        }
    }

    None
}

/// Canonical `claude-{family}-{major}` key for an id naming a modern major
/// (>= 4) without a minor (claude-sonnet-5, opus-5, 4-opus). The major must
/// be adjacent to the family token; in forward order it must not be followed
/// by another digit run (dated `4-20250514` shapes are version-like, not
/// bare), and in reversed order it must not itself be the minor of a
/// preceding legacy major (claude-3-5-sonnet).
fn normalize_claude_family_bare_major(lower: &str) -> Option<String> {
    let parts: Vec<&str> = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect();
    let all_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());

    for (idx, part) in parts.iter().enumerate() {
        if !CLAUDE_FAMILY_TOKENS.contains(part) {
            continue;
        }
        if let Some(major) = parts
            .get(idx + 1)
            .copied()
            .filter(|p| is_modern_claude_major(p))
        {
            if parts.get(idx + 2).is_none_or(|next| !all_digits(next)) {
                return Some(format!("claude-{part}-{major}"));
            }
        }
        if idx >= 1
            && is_modern_claude_major(parts[idx - 1])
            && (idx < 2 || !all_digits(parts[idx - 2]))
        {
            return Some(format!("claude-{part}-{}", parts[idx - 1]));
        }
    }

    None
}

/// True if the id carries a delimited modern `major(-|.)minor` version
/// (4-6, 4.8, 5-0, 4-60, 4-20250514). Generalizes the former
/// `contains_delimited_major_minor(lower, '4')` checks across all modern
/// majors so the never-degrade contract also covers major 5 and up.
fn contains_delimited_modern_major_minor(haystack: &str) -> bool {
    ('4'..='9').any(|major| contains_delimited_major_minor(haystack, major))
}

/// The version-pinned canonical key a Claude id requests, used to veto
/// fuzzy/stripped resolutions that would land on a different version.
///
/// - An explicit single-digit minor (claude-sonnet-4-7) always pins; this is
///   main's opus-only minor guard generalized across families.
/// - A bare major pins from major 5 up (claude-opus-5 must never bill as any
///   opus 4.x key). Bare major 4 is deliberately left unpinned to preserve
///   the long-standing behavior of e.g. `claude-opus-4` resolving to a
///   dated or regional 4.x dataset key.
fn requested_claude_version(lower: &str) -> Option<String> {
    if let Some(model) = normalize_claude_family_minor(lower) {
        return Some(model);
    }
    normalize_claude_family_bare_major(lower).filter(|model| !model.ends_with("-4"))
}

/// Veto for resolutions that violate the never-degrade contract:
/// cross-family (a sonnet id billed at an opus key), cross-version (a 4-7 id
/// billed at a 4-6 key, a major-5 id billed at a 4.x key), or any
/// modern-Claude resolution for an id whose `major-minor` version could not
/// be parsed (4-60, 5-0, dated forms). Exact dataset hits stay allowed: they
/// either normalize back to the requested version or, for unparseable
/// versions, do not normalize at all. Generalization of the former
/// `resolves_different_claude_opus_4_minor`.
fn resolves_unsafe_claude_version(
    requested_family: Option<&'static str>,
    requested_version: Option<&str>,
    unparsed_modern_version: bool,
    result: &LookupResult,
) -> bool {
    let Some(requested_family) = requested_family else {
        return false;
    };
    let matched_lower = result.matched_key.to_lowercase();

    if claude_family(&matched_lower).is_some_and(|family| family != requested_family) {
        return true;
    }

    let resolved = normalize_model_name(&matched_lower);
    if let Some(requested_version) = requested_version {
        return resolved.is_some_and(|resolved| resolved != requested_version);
    }
    unparsed_modern_version && resolved.is_some()
}

fn is_single_digit_minor(value: &str) -> bool {
    value.len() == 1 && value.as_bytes()[0].is_ascii_digit() && value.as_bytes()[0] != b'0'
}

fn normalize_version_separator(model_id: &str) -> Option<String> {
    let mut result = String::with_capacity(model_id.len());
    let chars: Vec<char> = model_id.chars().collect();
    let mut changed = false;

    for i in 0..chars.len() {
        if chars[i] == '-'
            && i > 0
            && i < chars.len() - 1
            && chars[i - 1].is_ascii_digit()
            && chars[i + 1].is_ascii_digit()
        {
            let is_multi_digit_before = i >= 2 && chars[i - 2].is_ascii_digit();
            let is_multi_digit_after = i + 2 < chars.len() && chars[i + 2].is_ascii_digit();
            let looks_like_date = is_multi_digit_before || is_multi_digit_after;

            if looks_like_date {
                result.push(chars[i]);
            } else {
                result.push('.');
                changed = true;
            }
        } else {
            result.push(chars[i]);
        }
    }

    if changed {
        Some(result)
    } else {
        None
    }
}

fn strip_known_provider_prefix(model_id: &str) -> Option<&str> {
    for prefix in PROVIDER_PREFIXES {
        if let Some(stripped) = model_id.strip_prefix(prefix) {
            if !stripped.is_empty() {
                return Some(stripped);
            }
        }
    }
    None
}

/// Generic routing-prefix fallback for ids whose leading segment is not one
/// of the curated `PROVIDER_PREFIXES` (e.g. `cx/gpt-5.5` routed through an
/// `omniroute` proxy, or any other CLI/router-assigned alias). Returns the
/// terminal path segment — the part after the last `/` — when the id
/// actually contains a `/`, so `cx/gpt-5.5` resolves to `gpt-5.5`.
///
/// This is intentionally unconditional (unlike `strip_known_provider_prefix`,
/// which only recognizes canonical LLM provider names): the caller only
/// invokes it as a fallback AFTER the exact/direct lookup on the full id has
/// already failed, so dataset keys that legitimately keep their prefix (e.g.
/// `anthropic/claude-fable-5`) are resolved by their own exact key first and
/// never reach this fallback.
fn strip_generic_provider_prefix(model_id: &str) -> Option<&str> {
    let terminal = model_id.rsplit('/').next()?;
    if terminal.is_empty() || terminal == model_id {
        return None;
    }
    Some(terminal)
}

fn is_valid_price_value(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// Returns true if the pricing entry has at least one usable cost field
/// (base or above-200k tier). Entries with all-None pricing (e.g.
/// subscription-based providers like Perplexity) are useless for
/// pay-per-token cost estimation and should be deprioritized.
fn has_any_usable_pricing(pricing: &ModelPricing) -> bool {
    pricing
        .all_rates()
        .into_iter()
        .any(|opt| opt.is_some_and(is_valid_price_value))
}

fn lookup_result_if_usable(
    pricing: &ModelPricing,
    source: &str,
    matched_key: &str,
) -> Option<LookupResult> {
    has_any_usable_pricing(pricing).then(|| LookupResult {
        pricing: pricing.clone(),
        source: source.into(),
        matched_key: matched_key.into(),
        evidence: ResolutionEvidence::deterministic(ResolutionKind::Exact),
    })
}

fn has_any_valid_above_tier_value(pricing: &ModelPricing) -> bool {
    [
        pricing.input_cost_per_token_above_128k_tokens,
        pricing.input_cost_per_token_above_200k_tokens,
        pricing.input_cost_per_token_above_256k_tokens,
        pricing.input_cost_per_token_above_272k_tokens,
        pricing.output_cost_per_token_above_128k_tokens,
        pricing.output_cost_per_token_above_200k_tokens,
        pricing.output_cost_per_token_above_256k_tokens,
        pricing.output_cost_per_token_above_272k_tokens,
        pricing.cache_read_input_token_cost_above_200k_tokens,
        pricing.cache_read_input_token_cost_above_272k_tokens,
        pricing.cache_creation_input_token_cost_above_200k_tokens,
    ]
    .into_iter()
    .flatten()
    .any(is_valid_price_value)
}

fn has_meaningful_tier_support(pricing: &ModelPricing) -> bool {
    [
        (
            pricing.input_cost_per_token,
            pricing.input_cost_per_token_above_128k_tokens,
        ),
        (
            pricing.input_cost_per_token,
            pricing.input_cost_per_token_above_200k_tokens,
        ),
        (
            pricing.input_cost_per_token,
            pricing.input_cost_per_token_above_256k_tokens,
        ),
        (
            pricing.input_cost_per_token,
            pricing.input_cost_per_token_above_272k_tokens,
        ),
        (
            pricing.output_cost_per_token,
            pricing.output_cost_per_token_above_128k_tokens,
        ),
        (
            pricing.output_cost_per_token,
            pricing.output_cost_per_token_above_200k_tokens,
        ),
        (
            pricing.output_cost_per_token,
            pricing.output_cost_per_token_above_256k_tokens,
        ),
        (
            pricing.output_cost_per_token,
            pricing.output_cost_per_token_above_272k_tokens,
        ),
    ]
    .into_iter()
    .any(|(base, above)| match (base, above) {
        (Some(base), Some(above)) => base.is_finite() && base >= 0.0 && is_valid_price_value(above),
        _ => false,
    })
}

fn contains_delimited_fragment(haystack: &str, fragment: &str) -> bool {
    if fragment.is_empty() {
        return false;
    }

    for (pos, _) in haystack.match_indices(fragment) {
        let before_ok = pos == 0 || !haystack[..pos].chars().last().unwrap().is_alphanumeric();
        let after_pos = pos + fragment.len();
        let after_ok = after_pos == haystack.len()
            || !haystack[after_pos..]
                .chars()
                .next()
                .unwrap()
                .is_alphanumeric();

        if before_ok && after_ok {
            return true;
        }
    }

    false
}

fn contains_delimited_major_minor(haystack: &str, major: char) -> bool {
    for (pos, _) in haystack.match_indices(major) {
        let before_ok = pos == 0 || !haystack[..pos].chars().last().unwrap().is_alphanumeric();
        let after_pos = pos + major.len_utf8();
        let mut after = haystack[after_pos..].chars();
        let Some(separator) = after.next() else {
            continue;
        };
        let Some(minor_start) = after.next() else {
            continue;
        };

        if before_ok && matches!(separator, '.' | '-') && minor_start.is_ascii_digit() {
            return true;
        }
    }

    false
}

fn is_fuzzy_eligible(model_id: &str) -> bool {
    if model_id.len() < MIN_FUZZY_MATCH_LEN {
        return false;
    }
    !FUZZY_BLOCKLIST.contains(&model_id)
}

/// Attempts to find a model by progressively stripping trailing segments.
/// Handles arbitrary suffixes (e.g., "claude-sonnet-4-5-thinking" → "claude-sonnet-4-5").
/// This replaces the hardcoded TIER_SUFFIXES and FALLBACK_SUFFIXES approach.
fn try_strip_unknown_suffix<F>(model_id: &str, do_lookup: F) -> Option<LookupResult>
where
    F: Fn(&str) -> Option<LookupResult>,
{
    if has_unrecognized_claude_four_minor(model_id) {
        return None;
    }

    let parts: Vec<&str> = model_id.split('-').collect();

    if parts.len() < 2 {
        return None;
    }

    let max_strip = std::cmp::min(parts.len() - 1, MAX_SUFFIX_STRIP_SEGMENTS);

    for strip in 1..=max_strip {
        let candidate: String = parts[..parts.len() - strip].join("-");

        if candidate.len() >= MIN_MODEL_NAME_LEN {
            if strips_claude_numeric_minor(&candidate, parts[parts.len() - strip]) {
                continue;
            }

            if let Some(result) = do_lookup(&candidate) {
                return Some(result);
            }
        }
    }

    None
}

fn strips_claude_numeric_minor(candidate: &str, first_stripped_segment: &str) -> bool {
    if !is_version_segment(first_stripped_segment) {
        return false;
    }
    let claude_branded = candidate.contains("claude")
        || candidate.contains("opus")
        || candidate.contains("sonnet")
        || candidate.contains("haiku");
    if !claude_branded {
        return false;
    }
    // Refuse to strip a version segment when it would either peel a minor off
    // a still-versioned claude-4 candidate (claude-sonnet-4-5 -> claude-sonnet-4)
    // or erode the id's only version, leaving a bare brand token
    // (claude-2.1 -> claude). Both candidates would resolve to a different
    // model's price. Dated forms (claude-3-5-sonnet-20241022) keep stripping:
    // their candidate retains a version, so neither arm fires.
    contains_delimited_fragment(candidate, "4") || !candidate.bytes().any(|b| b.is_ascii_digit())
}

/// True for a bare version segment produced by splitting an id on `-`:
/// digits with at most one interior dot (`4`, `6`, `2.1`, `20241022`).
fn is_version_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() || !bytes[bytes.len() - 1].is_ascii_digit() {
        return false;
    }
    let mut seen_dot = false;
    for &byte in bytes {
        match byte {
            b'0'..=b'9' => {}
            b'.' if !seen_dot => seen_dot = true,
            _ => return false,
        }
    }
    true
}

fn has_unrecognized_claude_four_minor(model_id: &str) -> bool {
    (model_id.contains("claude")
        || model_id.contains("opus")
        || model_id.contains("sonnet")
        || model_id.contains("haiku"))
        && contains_delimited_major_minor(model_id, '4')
        && !contains_delimited_fragment(model_id, "4.5")
        && !contains_delimited_fragment(model_id, "4-5")
        && !contains_delimited_fragment(model_id, "4.6")
        && !contains_delimited_fragment(model_id, "4-6")
        && !contains_delimited_fragment(model_id, "4.7")
        && !contains_delimited_fragment(model_id, "4-7")
}

/// Attempts to find a model by progressively stripping leading segments.
/// Handles arbitrary routing prefixes (e.g., "myplugin-claude-3.5-sonnet" → "claude-3.5-sonnet").
/// This replaces the hardcoded STRIPPED_PREFIXES approach.
fn try_strip_unknown_prefix<F>(model_id: &str, do_lookup: F) -> Option<LookupResult>
where
    F: Fn(&str) -> Option<LookupResult>,
{
    let parts: Vec<&str> = model_id.split('-').collect();

    if parts.len() < 2 {
        return None;
    }

    let max_skip = std::cmp::min(parts.len() - 1, MAX_PREFIX_STRIP_SEGMENTS);

    for skip in 1..=max_skip {
        let candidate: String = parts[skip..].join("-");

        if candidate.len() >= MIN_MODEL_NAME_LEN {
            // Try candidate directly
            if let Some(result) = do_lookup(&candidate) {
                return Some(result);
            }

            // Try candidate with suffix stripping
            if let Some(result) = try_strip_unknown_suffix(&candidate, &do_lookup) {
                return Some(result);
            }
        }
    }

    None
}

/// Deterministic provider choice when multiple models.dev providers share a
/// model part: the canonical `anthropic/` namespace wins outright; otherwise
/// the shorter key is preferred (the historical winner of the insertion-order
/// race, keeping existing resolutions stable), with lexicographic order
/// breaking length ties so the result no longer depends on HashMap iteration
/// order.
// @keep: the shortest-key fallback is arbitrary and actively harmful; the
// original-provider preference in front of it is what makes this defensible.
/// Elect between two dataset keys that share a model part.
///
/// Preferring the ORIGINAL provider generalizes what used to be a hardcoded
/// `anthropic/` special case. The rule it encodes is the same one that
/// motivated that case: when several vendors publish a key ending in the same
/// model name, the vendor who made the model is the one whose rates describe
/// it — a reseller or aggregator row is at best a repackaging.
///
/// Length is the last resort and is a coin-flip, not a signal. It is what
/// elected `morph/auto` ($0.85/$1.55) over three $0.00 router rows for the
/// model part `auto` (#1062), i.e. the single worst-priced candidate purely
/// because its key was ten characters. Routing labels no longer reach here at
/// all, but the same hazard remains for any model part several vendors share,
/// so prefer adding the real vendor to ORIGINAL_PROVIDER_PREFIXES over
/// relying on the tie-break to land correctly.
fn prefers_model_part_key(candidate: &str, existing: &str) -> bool {
    let candidate_lower = candidate.to_lowercase();
    let existing_lower = existing.to_lowercase();
    match (
        is_original_provider(&candidate_lower),
        is_original_provider(&existing_lower),
    ) {
        (true, false) => true,
        (false, true) => false,
        _ => (candidate_lower.len(), candidate_lower) < (existing_lower.len(), existing_lower),
    }
}

// @keep: these look like model names and are not, which is the whole problem.
/// Model ids that name a ROUTER, not a model.
///
/// Cursor, Copilot Desktop, Copilot VS Code, Kiro and Workbuddy all emit a
/// bare `auto` when the product chose the model on the user's behalf
/// (`sessions/cursor.rs:356`, `copilot_desktop.rs:123`, `copilot_vscode.rs:110`,
/// `kiro.rs:1135`, `workbuddy.rs:127`); `agent_review` is a Cursor feature.
/// Nothing in the session log records which model actually served the
/// request, so any rate attached to these describes a different model.
///
/// Left to the normal chain, `auto` matches by model part against every
/// dataset key ending in `/auto` and — because ties break on shortest key —
/// elects `morph/auto` at $0.85/$1.55, an unrelated code-apply vendor. That
/// is real money billed from a coincidence of spelling (#1062).
///
/// BARE ids only. A qualified `morph/auto` is a genuine Morph model and still
/// resolves. `custom-pricing.json` is consulted before this, so a user who
/// knows their router's effective rate can still state it.
const ROUTING_LABELS: &[&str] = &["auto", "agent_review"];

pub(crate) fn is_routing_label(model_id: &str) -> bool {
    let lower = model_id.trim().to_lowercase();
    ROUTING_LABELS.contains(&lower.as_str())
}

fn is_original_provider(key: &str) -> bool {
    let lower = key.to_lowercase();
    ORIGINAL_PROVIDER_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

/// Whether the dataset key's leading segment *is* the hinted vendor, rather
/// than a reseller that merely nests the vendor deeper in the key.
///
/// `poe/novita/kimi-k2.6` and `novita-ai/moonshotai/kimi-k2.6` both carry the
/// tag `novita`, but only the second is Novita's own row; the first is Poe
/// reselling it at $0.96/$4.04 per MTok against Novita's $0.80/$3.40.
fn key_root_matches_hint(key: &str, hint_tags: &[String]) -> bool {
    let Some(root) = key.split('/').next() else {
        return false;
    };
    provider_identity::provider_tags(root)
        .iter()
        .any(|tag| hint_tags.iter().any(|hint| hint == tag))
}

/// Whether provider-tag folding makes the key root and hint match despite
/// naming different billing endpoints. The alias keeps fallback rows reachable,
/// but neither endpoint's root is the other endpoint's own top-level row.
fn key_root_is_cross_provider_alias(key: &str, provider_id: &str) -> bool {
    let normalize_root = |value: &str| {
        value
            .trim()
            .trim_end_matches('/')
            .split('/')
            .next()
            .unwrap_or_default()
            .to_lowercase()
            .replace('-', "_")
    };
    let root = normalize_root(key);
    let hint = normalize_root(provider_id);

    let is_claude_endpoint = |value: &str| matches!(value, "anthropic" | "vertex" | "vertex_ai");
    root != hint && is_claude_endpoint(&root) && is_claude_endpoint(&hint)
}

fn key_root_matches_provider_hint(key: &str, provider_id: &str) -> bool {
    let hint_tags = provider_identity::provider_tags(provider_id);
    key_root_matches_hint(key, &hint_tags) && !key_root_is_cross_provider_alias(key, provider_id)
}

fn is_reseller_provider(key: &str) -> bool {
    let lower = key.to_lowercase();
    RESELLER_PROVIDER_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn pricing_rows_equal(left: &ModelPricing, right: &ModelPricing) -> bool {
    left.all_rates()
        .into_iter()
        .zip(right.all_rates())
        .all(|(left, right)| match (left, right) {
            (Some(left), Some(right)) => left.to_bits() == right.to_bits(),
            (None, None) => true,
            _ => false,
        })
}

fn terminal_model_identity(model_id: &str) -> String {
    model_id
        .trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .to_lowercase()
}

fn select_best_match(
    matches: &[&String],
    dataset: &HashMap<String, ModelPricing>,
    source: &str,
    provider_id: Option<&str>,
    kind: ResolutionKind,
    requested_model_id: &str,
) -> Option<LookupResult> {
    if matches.is_empty() {
        return None;
    }

    let all_usable_matches: Vec<&String> = matches
        .iter()
        .copied()
        .filter(|key| {
            dataset
                .get(key.as_str())
                .is_some_and(has_any_usable_pricing)
        })
        .collect();
    if all_usable_matches.is_empty() {
        return None;
    }

    let candidate_count = all_usable_matches.len();
    let first_pricing = dataset.get(all_usable_matches[0].as_str())?;
    let price_consensus = all_usable_matches.iter().skip(1).all(|key| {
        dataset
            .get(key.as_str())
            .is_some_and(|pricing| pricing_rows_equal(first_pricing, pricing))
    });

    let hint_tags: Vec<String> = provider_id
        .map(provider_identity::provider_tags)
        .unwrap_or_default();

    let provider_matches: Vec<&String> = matches
        .iter()
        .copied()
        .filter(|key| provider_identity::matches_provider_hint_with_tags(key, &hint_tags))
        .collect();

    let preferred_matches = if provider_matches.is_empty() {
        matches
    } else {
        provider_matches.as_slice()
    };

    // Deprioritize entries with all-None pricing (e.g. perplexity/anthropic/...
    // which matches provider hint "anthropic" but has subscription-based pricing
    // with no per-token cost data). If provider-specific candidates are all
    // unusable, fall back to any priced candidate in the broader match set so
    // fuzzy/provider-aware lookups can still resolve a valid non-provider key.
    let preferred_with_pricing: Vec<&String> = preferred_matches
        .iter()
        .copied()
        .filter(|k| dataset.get(k.as_str()).is_some_and(has_any_usable_pricing))
        .collect();
    let effective_matches: Vec<&String> =
        if preferred_with_pricing.is_empty() && !provider_matches.is_empty() {
            matches
                .iter()
                .copied()
                .filter(|k| dataset.get(k.as_str()).is_some_and(has_any_usable_pricing))
                .collect()
        } else {
            preferred_with_pricing
        };
    if effective_matches.is_empty() {
        return None;
    }
    let effective_matches = effective_matches.as_slice();

    let hint_is_reseller = provider_id.is_some_and(is_reseller_provider);
    let pick = |candidates: &[&String], prefer_reseller: bool| -> Option<LookupResult> {
        let key = if prefer_reseller {
            candidates
                .iter()
                .find(|k| is_reseller_provider(k))
                .or_else(|| candidates.first())
        } else {
            // The vendor-spelling fold (`deepseek-ai` -> `deepseek`) widens
            // this pool: a `deepseek` hint now matches both
            // `novita/deepseek/<model>` and `cloudflare/@cf/deepseek-ai/<model>`,
            // two resellers with different price sheets for the same weights.
            // Nothing below tells them apart, so the winner falls out of key
            // ordering — which is length-descending over a HashMap's key
            // iteration, and therefore not even stable between processes for
            // equal-length keys. `deepseek-r1-distill-qwen-32b` with the
            // `deepseek` hint that `inferred_provider_from_model` synthesizes
            // moved off `novita/deepseek/...` at $0.30/$0.30 per MTok onto
            // `cloudflare/@cf/deepseek-ai/...` at $0.497/$4.881 — a 16x output
            // rate on the same weights.
            //
            // So the pool is ranked explicitly instead of leaning on key
            // order. The hinted vendor's own top-level row wins first:
            // `novita-ai/moonshotai/kimi-k2.6` at $0.80/$3.40 is Novita's own,
            // while `poe/novita/kimi-k2.6` at $0.96/$4.04 spells `novita` in a
            // nested segment only because Poe is reselling it. Ranking that row
            // rather than merely detecting it matters, because candidates are
            // ordered longest key first and the vendor's own row is usually the
            // shorter one: `vercel_ai_gateway/zai/glm-4.6` at $0.45/$1.80 would
            // otherwise be billed for a `zai` hint that Z.ai itself publishes
            // at `zai/glm-4.6`, $0.60/$2.20. A raw Vertex hint similarly keeps
            // Vertex's hosted row ahead of Anthropic's row, while an Anthropic
            // hint excludes that cross-provider root alias. A first-party row
            // is the next tier.
            //
            // Then comes a row that spells the vendor exactly as the hint does,
            // in preference to one that only matches after folding. That row is
            // taken even when it starts with a reseller prefix, because the
            // property that matters is the spelling, not the publisher: the
            // pre-fold match for a `deepseek-ai` hint on `deepseek-r1` is
            // `together_ai/deepseek-ai/DeepSeek-R1` at $3.00/$7.00, and
            // discarding it for being a reseller just hands the lookup to
            // `vercel_ai_gateway/deepseek/deepseek-r1` at $0.55/$2.19 — another
            // reseller, chosen for being spelled the other way and having a
            // longer key. Among equally spelled rows a non-reseller still wins.
            let by_root = candidates.iter().find(|k| {
                key_root_matches_hint(k, &hint_tags)
                    && !provider_id.is_some_and(|hint| key_root_is_cross_provider_alias(k, hint))
            });
            let by_spelling = provider_id.and_then(|hint| {
                let spelled: Vec<&&String> = candidates
                    .iter()
                    .filter(|k| provider_identity::matches_provider_spelling(k, hint))
                    .collect();
                spelled
                    .iter()
                    .copied()
                    .find(|k| !is_reseller_provider(k))
                    .or_else(|| spelled.first().copied())
            });
            by_root
                .or_else(|| candidates.iter().find(|k| is_original_provider(k)))
                .or(by_spelling)
                .or_else(|| candidates.iter().find(|k| !is_reseller_provider(k)))
                .or_else(|| candidates.first())
        };
        key.and_then(|k| {
            dataset.get(k.as_str()).map(|pricing| LookupResult {
                pricing: pricing.clone(),
                source: source.into(),
                matched_key: (*k).clone(),
                evidence: ResolutionEvidence {
                    kind,
                    candidate_count,
                    price_consensus,
                    exact_model_identity: terminal_model_identity(requested_model_id)
                        == terminal_model_identity(k),
                    alias_applied: false,
                    normalized: false,
                    stripped: false,
                },
            })
        })
    };

    pick(effective_matches, hint_is_reseller)
}

fn model_prefix_matches_provider(model_id: &str, provider_id: Option<&str>) -> bool {
    let Some(hint) = provider_id else {
        return true;
    };
    let Some(prefix) = model_id.split('/').next() else {
        return false;
    };
    let prefix_tag = provider_identity::canonical_provider(prefix);
    let hint_primary = provider_identity::canonical_provider(hint);
    match (prefix_tag, hint_primary) {
        (Some(p), Some(h)) => p == h,
        _ => false,
    }
}

fn scope_resolution_to_provider(mut result: LookupResult, model_id: &str) -> LookupResult {
    let Some(scoped) = parse_provider_scoped_model_path(model_id) else {
        return result;
    };

    // The scoped path asserts an endpoint, but a canonical-tag alias can still
    // make its terminal fallback land on another endpoint's root. Preserve the
    // weaker evidence in that case instead of laundering it as provider-scoped.
    result.evidence.kind = if key_root_matches_provider_hint(&result.matched_key, scoped.provider) {
        ResolutionKind::ProviderScoped
    } else {
        ResolutionKind::ModelPart
    };
    result
}

fn parse_provider_scoped_model_path(model_id: &str) -> Option<ProviderScopedModelPath<'_>> {
    let rest = model_id.strip_prefix("accounts/")?;
    let (provider, rest) = rest.split_once('/')?;
    let (scope, terminal_model_id) = rest.split_once('/')?;

    if provider.is_empty() || terminal_model_id.is_empty() {
        return None;
    }

    match scope {
        "models" | "routers" => Some(ProviderScopedModelPath {
            provider,
            terminal_model_id,
        }),
        _ => None,
    }
}

fn provider_hint_matches_scoped_provider(provider_id: Option<&str>, scoped_provider: &str) -> bool {
    let Some(provider_id) = provider_id else {
        return true;
    };

    let scoped_tags = provider_identity::provider_tags(scoped_provider);
    let hint_tags = provider_identity::provider_tags(provider_id);
    !scoped_tags.is_empty()
        && scoped_tags
            .iter()
            .any(|scoped| hint_tags.iter().any(|hint| hint == scoped))
}

fn provider_prefix_matches_scoped_provider(prefix: &str, scoped_tags: &[String]) -> bool {
    if scoped_tags.is_empty() {
        return false;
    }

    provider_identity::provider_tags(prefix.trim_end_matches('/'))
        .iter()
        .any(|prefix_tag| scoped_tags.iter().any(|scoped| scoped == prefix_tag))
}

fn normalize_provider_hint(provider_id: Option<&str>) -> Option<&str> {
    provider_id
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unknown"))
}

fn build_lookup_cache_key(model_id: &str, provider_id: Option<&str>) -> String {
    match provider_id {
        Some(provider) if !provider.trim().is_empty() => {
            format!("{}|{}", provider.to_lowercase(), model_id.to_lowercase())
        }
        _ => model_id.to_lowercase(),
    }
}

fn model_part_matches_exact(model_part: &str, model_id: &str) -> bool {
    if model_part == model_id {
        return true;
    }

    let mut suffix = model_part;
    while let Some((_, rest)) = suffix.split_once('.') {
        if rest == model_id {
            return true;
        }
        suffix = rest;
    }

    false
}

fn choose_best_source_result(
    litellm_result: Option<LookupResult>,
    openrouter_result: Option<LookupResult>,
    provider_id: Option<&str>,
) -> Option<LookupResult> {
    match (&litellm_result, &openrouter_result) {
        (Some(l), Some(o)) => {
            let l_matches_provider =
                provider_identity::matches_provider_hint(&l.matched_key, provider_id);
            let o_matches_provider =
                provider_identity::matches_provider_hint(&o.matched_key, provider_id);

            if l_matches_provider && !o_matches_provider {
                return litellm_result;
            }
            if o_matches_provider && !l_matches_provider {
                return openrouter_result;
            }

            let l_matches_root = provider_id
                .is_some_and(|hint| key_root_matches_provider_hint(&l.matched_key, hint));
            let o_matches_root = provider_id
                .is_some_and(|hint| key_root_matches_provider_hint(&o.matched_key, hint));
            if l_matches_root && !o_matches_root {
                return litellm_result;
            }
            if o_matches_root && !l_matches_root {
                return openrouter_result;
            }

            let l_is_original = is_original_provider(&l.matched_key);
            let o_is_original = is_original_provider(&o.matched_key);
            let l_is_reseller = is_reseller_provider(&l.matched_key);
            let o_is_reseller = is_reseller_provider(&o.matched_key);

            if o_is_original && !l_is_original {
                return openrouter_result;
            }
            if l_is_original && !o_is_original {
                return litellm_result;
            }
            if !l_is_reseller && o_is_reseller {
                return litellm_result;
            }
            if !o_is_reseller && l_is_reseller {
                return openrouter_result;
            }

            litellm_result
        }
        (Some(_), None) => litellm_result,
        (None, Some(_)) => openrouter_result,
        (None, None) => None,
    }
}

/// Run the normal LiteLLM/OpenRouter arbitration, but let a literal
/// provider-root match from Models.dev displace an alias-only winner. Models.dev
/// otherwise remains the long-tail fallback at its established precedence.
fn choose_best_source_result_with_models_dev(
    litellm_result: Option<LookupResult>,
    openrouter_result: Option<LookupResult>,
    models_dev_result: Option<LookupResult>,
    provider_id: Option<&str>,
) -> Option<LookupResult> {
    let primary = choose_best_source_result(litellm_result, openrouter_result, provider_id);
    let models_dev_matches_root = models_dev_result.as_ref().is_some_and(|result| {
        provider_id.is_some_and(|hint| key_root_matches_provider_hint(&result.matched_key, hint))
    });
    let primary_is_cross_provider_alias = primary.as_ref().is_some_and(|result| {
        provider_id.is_some_and(|hint| key_root_is_cross_provider_alias(&result.matched_key, hint))
    });

    if models_dev_matches_root && primary_is_cross_provider_alias {
        models_dev_result
    } else {
        primary
    }
}

/// Resolve an exact model part among keys matched by the provider hint.
/// Canonical tag aliases keep endpoint-related candidates reachable for
/// estimates, but only a raw root match is provider-scoped evidence.
fn exact_match_with_provider_prefixes(
    model_id: &str,
    provider_id: Option<&str>,
    key_parts: &[KeyModelPart],
    dataset: &HashMap<String, ModelPricing>,
    source: &str,
) -> Option<LookupResult> {
    let provider_id = provider_id?;
    let hint_tags = provider_identity::provider_tags(provider_id);

    let matches: Vec<&String> = key_parts
        .iter()
        .filter(|kp| {
            model_part_matches_exact(&kp.lower_model_part, model_id)
                && provider_identity::matches_provider_hint_with_tags(&kp.key, &hint_tags)
        })
        .map(|kp| &kp.key)
        .collect();

    if matches.is_empty() {
        return None;
    }

    let result = select_best_match(
        &matches,
        dataset,
        source,
        Some(provider_id),
        ResolutionKind::ModelPart,
        model_id,
    )?;

    // Canonical provider tags deliberately keep endpoint aliases reachable for
    // estimates (notably `anthropic` <-> `vertex_ai`). Only a candidate whose
    // top-level endpoint actually matches the hint proves provider identity.
    // Otherwise this remains the same estimate-only ModelPart evidence as an
    // unhinted cross-provider fallback.
    if key_root_matches_provider_hint(&result.matched_key, provider_id) {
        Some(result.with_kind(ResolutionKind::ProviderScoped))
    } else {
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock LiteLLM data matching real API responses for OpenCode Zen models
    fn mock_litellm() -> HashMap<String, ModelPricing> {
        let mut m = HashMap::new();

        // === GPT-4 models (baseline) ===
        m.insert(
            "gpt-4o".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000025),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(0.00000125),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-4o-mini".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000015),
                output_cost_per_token: Some(0.0000006),
                cache_read_input_token_cost: Some(0.000000075),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-4-turbo".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00003),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        // === OpenCode Zen: GPT-5 family ===
        m.insert(
            "gpt-5.2".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000175),
                output_cost_per_token: Some(0.000014),
                cache_read_input_token_cost: Some(1.75e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5.5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                input_cost_per_token_above_272k_tokens: Some(0.000010),
                output_cost_per_token: Some(0.000030),
                output_cost_per_token_above_272k_tokens: Some(0.000045),
                cache_read_input_token_cost: Some(0.0000005),
                cache_read_input_token_cost_above_272k_tokens: Some(0.000001),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5.1".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(1.25e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5.1-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(1.25e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5.1-codex-max".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(1.25e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(1.25e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(1.25e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5-nano".into(),
            ModelPricing {
                input_cost_per_token: Some(5e-8),
                output_cost_per_token: Some(4e-7),
                cache_read_input_token_cost: Some(5e-9),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        // === OpenCode Zen: Claude family (LiteLLM entries) ===
        m.insert(
            "claude-3-5-sonnet-20241022".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: Some(0.0000003),
                cache_creation_input_token_cost: Some(0.00000375),
                ..Default::default()
            },
        );
        m.insert(
            "claude-sonnet-4-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: Some(3e-7),
                cache_creation_input_token_cost: Some(0.00000375),
                ..Default::default()
            },
        );
        m.insert(
            "claude-haiku-4-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000001),
                output_cost_per_token: Some(0.000005),
                cache_read_input_token_cost: Some(1e-7),
                cache_creation_input_token_cost: Some(0.00000125),
                ..Default::default()
            },
        );
        m.insert(
            "bedrock/us.anthropic.claude-3-5-haiku-20241022-v1:0".into(),
            ModelPricing {
                input_cost_per_token: Some(8e-7),
                output_cost_per_token: Some(0.000004),
                cache_read_input_token_cost: Some(8e-8),
                cache_creation_input_token_cost: Some(0.000001),
                ..Default::default()
            },
        );
        m.insert(
            "claude-opus-4-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                cache_read_input_token_cost: Some(5e-7),
                cache_creation_input_token_cost: Some(0.00000625),
                ..Default::default()
            },
        );
        m.insert(
            "claude-opus-4-1".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),
                output_cost_per_token: Some(0.000075),
                cache_read_input_token_cost: Some(0.0000015),
                cache_creation_input_token_cost: Some(0.00001875),
                ..Default::default()
            },
        );

        // === OpenCode Zen: Gemini family (LiteLLM entries) ===
        m.insert(
            "openrouter/google/gemini-3-pro-preview".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000002),
                output_cost_per_token: Some(0.000012),
                cache_read_input_token_cost: Some(2e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "vertex_ai/gemini-3-flash-preview".into(),
            ModelPricing {
                input_cost_per_token: Some(5e-7),
                output_cost_per_token: Some(0.000003),
                cache_read_input_token_cost: Some(5e-8),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        // === OpenCode Zen: Grok (LiteLLM entry) ===
        m.insert(
            "xai/grok-code-fast-1-0825".into(),
            ModelPricing {
                input_cost_per_token: Some(2e-7),
                output_cost_per_token: Some(0.0000015),
                cache_read_input_token_cost: Some(2e-8),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        m.insert(
            "azure_ai/grok-code-fast-1".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000035),
                output_cost_per_token: Some(0.0000175),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "bedrock/anthropic.claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: Some(3e-7),
                cache_creation_input_token_cost: Some(0.00000375),
                ..Default::default()
            },
        );
        m.insert(
            "vertex_ai/gemini-2.5-pro".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.000005),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "google/gemini-2.5-pro".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.000005),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        m
    }

    /// Mock OpenRouter data matching real API responses for OpenCode Zen models
    fn mock_openrouter() -> HashMap<String, ModelPricing> {
        let mut m = HashMap::new();

        // === Baseline models ===
        m.insert(
            "openai/gpt-4o".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000025),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(0.00000125),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        // === OpenCode Zen: Claude (OpenRouter entries) ===
        m.insert(
            "anthropic/claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: Some(3e-7),
                cache_creation_input_token_cost: Some(0.00000375),
                ..Default::default()
            },
        );
        m.insert(
            "anthropic/claude-opus-4-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                cache_read_input_token_cost: Some(0.0000005),
                cache_creation_input_token_cost: Some(0.00000625),
                ..Default::default()
            },
        );
        m.insert(
            "anthropic/claude-3.5-haiku".into(),
            ModelPricing {
                input_cost_per_token: Some(8e-7),
                output_cost_per_token: Some(0.000004),
                cache_read_input_token_cost: Some(8e-8),
                cache_creation_input_token_cost: Some(0.000001),
                ..Default::default()
            },
        );

        // === OpenCode Zen: GLM family ===
        m.insert(
            "z-ai/glm-4.7".into(),
            ModelPricing {
                input_cost_per_token: Some(4e-7),
                output_cost_per_token: Some(0.0000015),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "z-ai/glm-4.6".into(),
            ModelPricing {
                input_cost_per_token: Some(3.9e-7),
                output_cost_per_token: Some(0.0000019),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        m.insert(
            "moonshotai/kimi-k2".into(),
            ModelPricing {
                input_cost_per_token: Some(4.56e-7),
                output_cost_per_token: Some(0.00000184),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "moonshotai/kimi-k2.5".into(),
            ModelPricing {
                input_cost_per_token: Some(4.5e-7),
                output_cost_per_token: Some(0.0000025),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "moonshotai/kimi-k2.6".into(),
            ModelPricing {
                input_cost_per_token: Some(9.5e-7),
                output_cost_per_token: Some(0.000004),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        m.insert(
            "moonshotai/kimi-k2-thinking".into(),
            ModelPricing {
                input_cost_per_token: Some(4e-7),
                output_cost_per_token: Some(0.00000175),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        // === OpenCode Zen: Qwen family ===
        m.insert(
            "qwen/qwen3-coder".into(),
            ModelPricing {
                input_cost_per_token: Some(2.2e-7),
                output_cost_per_token: Some(9.5e-7),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        m
    }

    fn create_lookup() -> PricingLookup {
        PricingLookup::new(mock_litellm(), mock_openrouter(), HashMap::new())
    }

    // =========================================================================
    // OPENCODE ZEN MODELS - GPT-5 FAMILY
    // All models from https://opencode.ai/docs/zen/
    // =========================================================================

    #[test]
    fn test_opencode_zen_gpt_5_2() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.2").unwrap();
        assert_eq!(result.matched_key, "gpt-5.2");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_gpt_5_1() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.1").unwrap();
        assert_eq!(result.matched_key, "gpt-5.1");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_gpt_5_1_codex() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.1-codex").unwrap();
        assert_eq!(result.matched_key, "gpt-5.1-codex");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_gpt_5_1_codex_max() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.1-codex-max").unwrap();
        assert_eq!(result.matched_key, "gpt-5.1-codex-max");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_gpt_5() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5").unwrap();
        assert_eq!(result.matched_key, "gpt-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_gpt_5_codex() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5-codex").unwrap();
        assert_eq!(result.matched_key, "gpt-5-codex");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_gpt_5_nano() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5-nano").unwrap();
        assert_eq!(result.matched_key, "gpt-5-nano");
        assert_eq!(result.source, "LiteLLM");
    }

    // =========================================================================
    // OPENCODE ZEN MODELS - CLAUDE FAMILY
    // =========================================================================

    #[test]
    fn test_opencode_zen_claude_sonnet_4_5() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-sonnet-4-5").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_claude_sonnet_4() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-sonnet-4").unwrap();
        assert_eq!(result.matched_key, "anthropic/claude-sonnet-4");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_claude_haiku_4_5() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-haiku-4-5").unwrap();
        assert_eq!(result.matched_key, "claude-haiku-4-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_claude_3_5_haiku() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-3-5-haiku").unwrap();
        assert_eq!(result.matched_key, "anthropic/claude-3.5-haiku");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_claude_3_5_haiku_with_dot() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-3.5-haiku").unwrap();
        assert_eq!(result.matched_key, "anthropic/claude-3.5-haiku");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_claude_opus_4_5() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-opus-4-5").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_claude_opus_4_1() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-opus-4-1").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-1");
        assert_eq!(result.source, "LiteLLM");
    }

    // =========================================================================
    // OPENCODE ZEN MODELS - GLM FAMILY
    // =========================================================================

    #[test]
    fn test_opencode_zen_glm_4_7_free() {
        let lookup = create_lookup();
        let result = lookup.lookup("glm-4.7-free").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.7");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_glm_4_6() {
        let lookup = create_lookup();
        let result = lookup.lookup("glm-4.6").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.6");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_glm_4_7_with_hyphen() {
        let lookup = create_lookup();
        let result = lookup.lookup("glm-4-7").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.7");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_glm_4_6_with_hyphen() {
        let lookup = create_lookup();
        let result = lookup.lookup("glm-4-6").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.6");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_big_pickle() {
        let lookup = create_lookup();
        let result = lookup.lookup("big-pickle").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.7");
        assert_eq!(result.source, "OpenRouter");
    }

    // =========================================================================
    // OPENCODE ZEN MODELS - GEMINI FAMILY
    // =========================================================================

    #[test]
    fn test_opencode_zen_gemini_3_pro() {
        let lookup = create_lookup();
        let result = lookup.lookup("gemini-3-pro").unwrap();
        assert_eq!(result.matched_key, "openrouter/google/gemini-3-pro-preview");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_opencode_zen_gemini_3_flash() {
        let lookup = create_lookup();
        let result = lookup.lookup("gemini-3-flash").unwrap();
        assert_eq!(result.matched_key, "vertex_ai/gemini-3-flash-preview");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn antigravity_model_aliases_reach_priced_catalog_entries() {
        let mut litellm = mock_litellm();
        litellm.insert(
            "gemini-3.1-pro".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000002),
                output_cost_per_token: Some(0.000012),
                ..Default::default()
            },
        );
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "google/gemini-3.5-flash".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000015),
                output_cost_per_token: Some(0.000009),
                cache_read_input_token_cost: Some(0.00000015),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            litellm,
            mock_openrouter(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let cases = [
            ("MODEL_PLACEHOLDER_M16", "gemini-3.1-pro", "LiteLLM"),
            (
                "MODEL_PLACEHOLDER_M84",
                "vertex_ai/gemini-3-flash-preview",
                "LiteLLM",
            ),
            (
                "MODEL_PLACEHOLDER_M133",
                "google/gemini-3.5-flash",
                "Models.dev",
            ),
            (
                "gemini-3-flash-agent",
                "google/gemini-3.5-flash",
                "Models.dev",
            ),
            ("gemini-3-flash-b", "google/gemini-3.5-flash", "Models.dev"),
            (
                // Legacy CLI responseModel for M132, the retired predecessor
                // of M133 — prices as the High tier, same catalog entry as
                // `gemini-3-flash-agent`/`gemini-3-flash-b` above (see
                // aliases.rs source-citation comment, models.ts@603e3ea).
                "gemini-3-flash-a",
                "google/gemini-3.5-flash",
                "Models.dev",
            ),
            (
                "MODEL_PLACEHOLDER_M187",
                "google/gemini-3.5-flash",
                "Models.dev",
            ),
            (
                "MODEL_PLACEHOLDER_M20",
                "google/gemini-3.5-flash",
                "Models.dev",
            ),
        ];

        for (raw, expected_key, expected_source) in cases {
            let result = lookup
                .lookup(raw)
                .unwrap_or_else(|| panic!("unpriced alias: {raw}"));
            assert_eq!(result.matched_key, expected_key, "raw model: {raw}");
            assert_eq!(result.source, expected_source, "raw model: {raw}");
        }

        let cost = lookup.calculate_cost("gemini-3-flash-agent", 1_000_000, 100_000, 50_000, 0, 0);
        assert!((cost - 2.4075).abs() < 1e-10);
    }

    // =========================================================================
    // OPENCODE ZEN MODELS - KIMI FAMILY
    // =========================================================================

    #[test]
    fn test_opencode_zen_kimi_k2() {
        let lookup = create_lookup();
        let result = lookup.lookup("kimi-k2").unwrap();
        assert_eq!(result.matched_key, "moonshotai/kimi-k2");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_kimi_k2_thinking() {
        let lookup = create_lookup();
        let result = lookup.lookup("kimi-k2-thinking").unwrap();
        assert_eq!(result.matched_key, "moonshotai/kimi-k2-thinking");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_kimi_k2_5() {
        let lookup = create_lookup();
        let result = lookup.lookup("kimi-k2.5").unwrap();
        assert_eq!(result.matched_key, "moonshotai/kimi-k2.5");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_kimi_k2_5_free() {
        let lookup = create_lookup();
        let result = lookup.lookup("kimi-k2.5-free").unwrap();
        assert_eq!(result.matched_key, "moonshotai/kimi-k2.5");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_kimi_k2_6_aliases() {
        let lookup = create_lookup();
        for model_id in ["k2p6", "k2-p6", "kimi-k2p6", "Kimi-K2.6"] {
            let result = lookup.lookup(model_id).unwrap();
            assert_eq!(result.matched_key, "moonshotai/kimi-k2.6");
            assert_eq!(result.source, "OpenRouter");
            assert_eq!(result.pricing.input_cost_per_token, Some(9.5e-7));
            assert_eq!(result.pricing.output_cost_per_token, Some(0.000004));
        }
    }

    #[test]
    fn test_opencode_zen_kimi_k2_6_provider_hint_from_kimi_for_coding() {
        let lookup = create_lookup();
        let result = lookup
            .lookup_with_provider("k2p6", Some("kimi-for-coding"))
            .unwrap();
        assert_eq!(result.matched_key, "moonshotai/kimi-k2.6");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_opencode_zen_kimi_k2_5_aliases_unchanged() {
        let lookup = create_lookup();

        let raw_k2p5 = lookup.lookup("k2p5").unwrap();
        assert_eq!(raw_k2p5.matched_key, "moonshotai/kimi-k2-thinking");

        let dotted = lookup.lookup("kimi-k2.5").unwrap();
        assert_eq!(dotted.matched_key, "moonshotai/kimi-k2.5");
    }

    // =========================================================================
    // OPENCODE ZEN MODELS - QWEN FAMILY
    // =========================================================================

    #[test]
    fn test_opencode_zen_qwen3_coder() {
        let lookup = create_lookup();
        let result = lookup.lookup("qwen3-coder").unwrap();
        assert_eq!(result.matched_key, "qwen/qwen3-coder");
        assert_eq!(result.source, "OpenRouter");
    }

    // =========================================================================
    // OPENCODE ZEN MODELS - GROK FAMILY
    // =========================================================================

    #[test]
    fn test_opencode_zen_grok_code() {
        let lookup = create_lookup();
        let result = lookup.lookup("grok-code").unwrap();
        assert_eq!(result.matched_key, "xai/grok-code-fast-1-0825");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_provider_hint_prefers_matching_pricing_source() {
        let lookup = create_lookup();
        let result = lookup
            .lookup_with_provider("grok-code", Some("azure"))
            .unwrap();
        assert_eq!(result.matched_key, "azure_ai/grok-code-fast-1");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_provider_hint_matches_nested_reseller_exact_key() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        litellm.insert(
            "azure/openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup_with_provider("gpt-4", Some("azure")).unwrap();
        assert_eq!(result.matched_key, "azure/openai/gpt-4");
        assert_eq!(result.source, "LiteLLM");
    }

    // Regression: a generic id whose only fuzzy-eligible remnant after suffix
    // stripping is the bare word `model` (real example seen in local data:
    // `model-zero-usage-v1`, `test-model`) must NOT fuzzy-match a real priced
    // key like `azure_ai/model_router`. The word `model` carries no model
    // identity and is on the FUZZY_BLOCKLIST.
    #[test]
    fn fuzzy_match_does_not_resolve_generic_model_token() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "azure_ai/model_router".into(),
            ModelPricing {
                input_cost_per_token: Some(1.4e-7),
                output_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        // The bare token must not resolve.
        assert!(lookup.lookup("model").is_none());
        // Ids that strip down to the bare `model` token must not misresolve.
        assert!(lookup.lookup("model-zero-usage-v1").is_none());
        assert!(lookup.lookup("model-nonzero-usage-v1").is_none());
        assert!(lookup.lookup("test-model").is_none());

        // But an EXACT key match is still honored — `model-router` is a real
        // model id, not a fuzzy remnant.
        let mut litellm2 = HashMap::new();
        litellm2.insert(
            "azure/model-router".into(),
            ModelPricing {
                input_cost_per_token: Some(1.4e-7),
                output_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        let lookup2 = PricingLookup::new(litellm2, HashMap::new(), HashMap::new());
        assert_eq!(
            lookup2.lookup("model-router").unwrap().matched_key,
            "azure/model-router"
        );
    }

    // Regression: `gemini-default` is a generic routing label — it names which
    // router served the request, never which model did — so it must stay
    // unpriced and be excluded from submission. Its fuzzy-eligible remnant
    // after prefix stripping is the bare word `default`, which substring-hits
    // LiteLLM's real `fireworks-ai-default` row.
    //
    // That row is priced 0.0/0.0, and `covers_usage` counts an explicit zero as
    // a real rate, so before `default` joined the FUZZY_BLOCKLIST the label
    // looked priced and `exclude_unpriced_submission_messages` let it
    // through — a Google routing label submitted at Fireworks AI's rates.
    // Verified against the live LiteLLM dataset: `fireworks-ai-default` is a
    // real key with input and output cost 0.0.
    #[test]
    fn fuzzy_match_does_not_resolve_generic_default_token() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "fireworks-ai-default".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        // The bare token must not resolve.
        assert!(lookup.lookup("default").is_none());
        // Nor the routing label that strips down to it, with or without the
        // provider hint the submission path passes.
        assert!(lookup.lookup("gemini-default").is_none());
        assert!(lookup
            .lookup_with_provider("gemini-default", Some("google"))
            .is_none());

        // But an EXACT key match is still honored — `fireworks-ai-default` is a
        // real id in the dataset, not a fuzzy remnant.
        assert_eq!(
            lookup.lookup("fireworks-ai-default").unwrap().matched_key,
            "fireworks-ai-default"
        );
    }

    // The blocklist is consulted with the *query* remnant, so blocking
    // `default` must not stop a query from matching INTO a dataset key that
    // merely ends in `@default`. LiteLLM ships seven of those
    // (`vertex_ai/claude-*@default`), and they are ordinary priced models.
    #[test]
    fn blocking_the_default_token_still_matches_vertex_default_suffixed_keys() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "vertex_ai/claude-opus-4-7@default".into(),
            ModelPricing {
                input_cost_per_token: Some(5e-06),
                output_cost_per_token: Some(2.5e-05),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        assert_eq!(
            lookup
                .lookup("vertex_ai/claude-opus-4-7@default")
                .unwrap()
                .matched_key,
            "vertex_ai/claude-opus-4-7@default"
        );
        assert_eq!(
            lookup
                .lookup("claude-opus-4-7@default")
                .unwrap()
                .matched_key,
            "vertex_ai/claude-opus-4-7@default"
        );
    }

    // Defense-in-depth beyond #1070: the resolver-top `is_routing_label`
    // guard refuses the router labels parsers emit today (`auto`,
    // `agent_review`), but the model-part index is a second, deeper place a
    // bare id can elect another provider's row. Any provider may publish a
    // generic `FUZZY_BLOCKLIST` token as a model part (`default`, `router`,
    // `mini`, ...) — none do today, but a bare id carrying such a token names
    // no model, so it must not land on whatever unrelated key shares the
    // spelling. This guard covers shapes the label list does not enumerate;
    // full dataset keys still resolve.
    #[test]
    fn model_part_index_does_not_resolve_bare_generic_tokens() {
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "someprovider/router".into(),
            ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        );
        models_dev.insert(
            "someprovider/default".into(),
            ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        // A bare generic token must not resolve through another provider's
        // model part.
        assert!(lookup.lookup("router").is_none());
        assert!(lookup.lookup("default").is_none());

        // The tokens' own full dataset keys are still exact matches.
        assert_eq!(
            lookup.lookup("someprovider/router").unwrap().matched_key,
            "someprovider/router"
        );
        assert_eq!(
            lookup.lookup("someprovider/default").unwrap().matched_key,
            "someprovider/default"
        );
    }

    #[test]
    fn incomplete_unhinted_result_does_not_replace_provider_pricing() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "azure/gpt-fallback-guard".into(),
            ModelPricing {
                input_cost_per_token: Some(1.0),
                ..Default::default()
            },
        );
        litellm.insert(
            "gpt-fallback-guard".into(),
            ModelPricing {
                output_cost_per_token: Some(2.0),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 1,
            output: 1,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };

        // Neither row covers both populated buckets, and they share no base
        // bucket that would show they price the same deal, so no rate is
        // borrowed. Retain the provider row rather than replacing it with an
        // unhinted row that silently prices the input bucket at zero.
        assert_eq!(
            lookup.calculate_cost_with_provider("gpt-fallback-guard", Some("azure"), &usage),
            1.0
        );
    }

    #[test]
    fn test_provider_hint_normalizes_openai_codex_alias() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-5.2-preview".into(),
            ModelPricing {
                input_cost_per_token: Some(1.0),
                ..Default::default()
            },
        );
        litellm.insert(
            "google/gpt-5.2-preview-max".into(),
            ModelPricing {
                input_cost_per_token: Some(2.0),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup
            .lookup_with_provider("gpt-5.2", Some("openai-codex"))
            .unwrap();
        assert_eq!(result.matched_key, "openai/gpt-5.2-preview");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_provider_hint_matches_nested_google_segment_during_fuzzy_lookup() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openrouter/google/gemini-3-pro-preview".into(),
            ModelPricing {
                input_cost_per_token: Some(1.0),
                ..Default::default()
            },
        );
        litellm.insert(
            "vertex_ai/gemini-3-pro-preview-max".into(),
            ModelPricing {
                input_cost_per_token: Some(2.0),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup
            .lookup_with_provider("gemini-3-pro", Some("google"))
            .unwrap();
        assert_eq!(result.matched_key, "openrouter/google/gemini-3-pro-preview");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_cross_source_fuzzy_provider_hint_wins_over_original_provider_fallback() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "fireworks_ai/deepseek-v3-0324".into(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "deepseek/deepseek-v3-0324".into(),
            ModelPricing {
                input_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let result = lookup
            .lookup_with_provider("deepseek-v3", Some("fireworks"))
            .unwrap();
        assert_eq!(result.matched_key, "fireworks_ai/deepseek-v3-0324");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_provider_scoped_path_does_not_strip_into_wrong_fireworks_model() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "fireworks_ai/accounts/fireworks/models/deepseek-r1-0528-distill-qwen3-8b".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000002),
                output_cost_per_token: Some(0.0000002),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        assert!(
            lookup
                .lookup("accounts/fireworks/models/deepseek-v4-pro")
                .is_none(),
            "provider-scoped model paths should not be shortened into unrelated fuzzy matches"
        );
    }

    #[test]
    fn test_provider_scoped_path_matches_exact_litellm_reseller_key() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "fireworks_ai/accounts/fireworks/models/deepseek-v4-pro".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000003),
                output_cost_per_token: Some(0.0000004),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup
            .lookup("accounts/fireworks/models/deepseek-v4-pro")
            .unwrap();

        assert_eq!(
            result.matched_key,
            "fireworks_ai/accounts/fireworks/models/deepseek-v4-pro"
        );
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.evidence.kind, ResolutionKind::ProviderScoped);
        assert!(result.evidence.is_submission_safe());
    }

    #[test]
    fn test_provider_scoped_path_matches_exact_terminal_provider_key() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "fireworks_ai/deepseek-v4-pro".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000003),
                output_cost_per_token: Some(0.0000004),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup
            .lookup("accounts/fireworks/models/deepseek-v4-pro")
            .unwrap();

        assert_eq!(result.matched_key, "fireworks_ai/deepseek-v4-pro");
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.evidence.kind, ResolutionKind::ProviderScoped);
        assert!(result.evidence.is_submission_safe());
    }

    #[test]
    fn test_provider_scoped_path_does_not_use_upstream_openrouter_exact() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "deepseek/deepseek-v4-pro".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000001),
                output_cost_per_token: Some(0.000002),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(HashMap::new(), openrouter, HashMap::new());

        assert!(
            lookup
                .lookup("accounts/fireworks/models/deepseek-v4-pro")
                .is_none(),
            "Fireworks-scoped usage should not be priced with upstream DeepSeek rates"
        );
    }

    // =========================================================================
    // BASELINE / LEGACY TESTS
    // =========================================================================

    #[test]
    fn test_exact_match_litellm() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-4o").unwrap();
        assert_eq!(result.matched_key, "gpt-4o");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_exact_match_gpt_5_5_litellm() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.5").unwrap();
        assert_eq!(result.matched_key, "gpt-5.5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_exact_match_openrouter() {
        let lookup = create_lookup();
        let result = lookup.lookup("z-ai/glm-4.7").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.7");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_openrouter_model_part_match() {
        let lookup = create_lookup();
        let result = lookup.lookup("glm-4.7").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.7");
        assert_eq!(result.source, "OpenRouter");
    }

    /// A bare model id only proves the terminal model spelling. Resolving it to
    /// another provider's qualified catalog row remains useful as an estimate,
    /// but must not authorize publishing that provider's price.
    #[test]
    fn cross_provider_model_part_remains_visible_but_is_not_submission_safe() {
        let openrouter = HashMap::from([(
            "vendor/atlas-chat".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        )]);
        let lookup = PricingLookup::new(HashMap::new(), openrouter, HashMap::new());

        let result = lookup
            .lookup("atlas-chat")
            .expect("reporting should retain the model-part estimate");

        assert_eq!(result.matched_key, "vendor/atlas-chat");
        assert_eq!(result.evidence.kind, ResolutionKind::ModelPart);
        assert!(result.evidence.exact_model_identity);
        assert_eq!(
            result.evidence.submission_safety_gap(),
            Some(SubmissionSafetyGap::UnverifiedProviderIdentity)
        );
        assert!(!result.evidence.is_submission_safe());
    }

    #[test]
    fn unhinted_provider_prefix_remains_visible_but_is_not_submission_safe() {
        let litellm = HashMap::from([(
            "anthropic/atlas-chat".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        )]);
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let result = lookup
            .lookup_with_provider("atlas-chat", Some("synthetic"))
            .expect("reporting should retain the provider-prefix estimate");

        assert_eq!(result.matched_key, "anthropic/atlas-chat");
        assert_eq!(result.evidence.kind, ResolutionKind::ProviderPrefix);
        assert_eq!(
            result.evidence.submission_safety_gap(),
            Some(SubmissionSafetyGap::UnverifiedProviderIdentity)
        );
        assert!(!result.evidence.is_submission_safe());
    }

    #[test]
    fn provider_hint_alias_does_not_verify_another_endpoint_root() {
        let litellm = HashMap::from([(
            "vertex_ai/atlas-chat".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        )]);
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let anthropic = lookup
            .lookup_with_provider("atlas-chat", Some("anthropic"))
            .expect("the Vertex alias remains available as an estimate");
        assert_eq!(anthropic.matched_key, "vertex_ai/atlas-chat");
        assert_eq!(anthropic.evidence.kind, ResolutionKind::ModelPart);
        assert!(!anthropic.evidence.is_submission_safe());

        let vertex = lookup
            .lookup_with_provider("atlas-chat", Some("vertex_ai"))
            .expect("the literal Vertex endpoint should resolve");
        assert_eq!(vertex.matched_key, "vertex_ai/atlas-chat");
        assert_eq!(vertex.evidence.kind, ResolutionKind::ProviderScoped);
        assert!(vertex.evidence.is_submission_safe());
    }

    #[test]
    fn scoped_provider_path_does_not_verify_another_endpoint_root() {
        let vertex_row = ModelPricing {
            input_cost_per_token: Some(1e-6),
            output_cost_per_token: Some(2e-6),
            ..Default::default()
        };
        let litellm = HashMap::from([
            ("vertex_ai/atlas-chat".to_string(), vertex_row.clone()),
            (
                "vertex_ai/accounts/anthropic/models/atlas-chat".to_string(),
                vertex_row.clone(),
            ),
        ]);
        let openrouter = HashMap::from([(
            "vertex_ai/accounts/anthropic/models/atlas-chat".to_string(),
            vertex_row,
        )]);
        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());

        for source in [None, Some("litellm"), Some("openrouter")] {
            let result = lookup
                .lookup_with_source_and_provider(
                    "accounts/anthropic/models/atlas-chat",
                    source,
                    Some("anthropic"),
                )
                .unwrap_or_else(|| {
                    panic!("the {source:?} cross-endpoint row remains available as an estimate")
                });

            assert_eq!(
                result.matched_key,
                "vertex_ai/accounts/anthropic/models/atlas-chat"
            );
            assert_eq!(result.evidence.kind, ResolutionKind::ModelPart);
            assert!(!result.evidence.is_submission_safe());
        }
    }

    #[test]
    fn test_tier_suffix_low() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.1-codex-low").unwrap();
        assert_eq!(result.matched_key, "gpt-5.1-codex");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_tier_suffix_high() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-4o-high").unwrap();
        assert_eq!(result.matched_key, "gpt-4o");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_tier_suffix_free() {
        let lookup = create_lookup();
        let result = lookup.lookup("glm-4.7-free").unwrap();
        assert_eq!(result.matched_key, "z-ai/glm-4.7");
        assert_eq!(result.source, "OpenRouter");
    }

    #[test]
    fn test_tier_suffix_xhigh() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.2-xhigh").unwrap();
        assert_eq!(result.matched_key, "gpt-5.2");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_tier_suffix_xhigh_gpt_5_5() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.5-xhigh").unwrap();
        assert_eq!(result.matched_key, "gpt-5.5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_tier_suffix_xhigh_codex_max() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.1-codex-max-xhigh").unwrap();
        assert_eq!(result.matched_key, "gpt-5.1-codex-max");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_parenthesized_reasoning_tier_gpt_levels() {
        let lookup = create_lookup();

        for tier in ["minimal", "low", "medium", "high", "xhigh", "auto", "none"] {
            let id = format!("gpt-5.2({tier})");
            let result = lookup.lookup(&id).unwrap_or_else(|| panic!("{id} miss"));
            assert_eq!(result.matched_key, "gpt-5.2", "{id}");
            assert_eq!(result.source, "LiteLLM", "{id}");
        }
    }

    #[test]
    fn test_parenthesized_reasoning_tier_claude_and_gemini() {
        let lookup = create_lookup();

        let claude = lookup.lookup("claude-sonnet-4-5(high)").unwrap();
        assert_eq!(claude.matched_key, "claude-sonnet-4-5");
        assert_eq!(claude.source, "LiteLLM");

        // Dot-form claude id (cliproxyapi accepts either) routes through
        // version-separator normalization to the dashed catalog entry.
        let claude_dot = lookup.lookup("claude-sonnet-4.5(none)").unwrap();
        assert_eq!(claude_dot.matched_key, "claude-sonnet-4-5");

        let gemini = lookup.lookup("gemini-3-pro(auto)").unwrap();
        assert_eq!(gemini.matched_key, "openrouter/google/gemini-3-pro-preview");
    }

    #[test]
    fn test_parenthesized_reasoning_tier_with_routing_prefix() {
        let lookup = create_lookup();

        let prefixed = lookup.lookup("myproxy-gpt-5.2(xhigh)").unwrap();
        assert_eq!(prefixed.matched_key, "gpt-5.2");

        let antigravity = lookup
            .lookup("antigravity-claude-sonnet-4-5(high)")
            .unwrap();
        assert_eq!(antigravity.matched_key, "claude-sonnet-4-5");
    }

    #[test]
    fn test_parenthesized_reasoning_tier_unknown_value_does_not_strip() {
        let lookup = create_lookup();

        // Values outside the cliproxyapi level set must not silently
        // misresolve via `try_strip_unknown_suffix`: without an early
        // return, splitting on `-` would peel the parenthesized fragment
        // off and match a shorter, unrelated model id (e.g.
        // `gpt-5.2-codex(invalid)` collapsing to `gpt-5.2`).
        assert!(lookup.lookup("gpt-5.2(weirdgarbage)").is_none());
        assert!(lookup.lookup("gpt-5.2(1024)").is_none());
        assert!(lookup.lookup("gpt-5.2()").is_none());
        assert!(lookup.lookup("gpt-5.2-codex(invalid)").is_none());
        assert!(lookup.lookup("myproxy-gpt-5.2(invalid)").is_none());

        // The same guard must hold across model families so that the
        // generalized stripper never misresolves a non-GPT id by peeling
        // a parenthesized fragment off through the dash-suffix path.
        assert!(lookup
            .lookup("antigravity-claude-sonnet-4-5(invalid)")
            .is_none());
        assert!(lookup.lookup("claude-sonnet-4-5(garbage)").is_none());
        assert!(lookup.lookup("gemini-3-pro(weird)").is_none());
    }

    #[test]
    fn test_parenthesized_reasoning_tier_cost_matches_base_model() {
        let lookup = create_lookup();
        let base = lookup.calculate_cost("gpt-5.2", 1_000_000, 500_000, 0, 0, 0);
        let tiered = lookup.calculate_cost("gpt-5.2(xhigh)", 1_000_000, 500_000, 0, 0, 0);

        assert!((tiered - base).abs() < f64::EPSILON);
        assert!((tiered - 8.75).abs() < 0.001);
    }

    #[test]
    fn test_normalize_opus_4_5() {
        let lookup = create_lookup();
        let result = lookup.lookup("opus-4-5").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_free_variant_normalizes_to_market_priced_claude_model() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-sonnet-4-5-free").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_free_variant_with_extra_suffix_falls_back_to_market_priced_model() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-sonnet-4-5-free-high").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_normalize_opus_4_6_prefers_4_6_over_4() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00002),
                output_cost_per_token: Some(0.0001),
                ..Default::default()
            },
        );
        litellm.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00005),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup("opus-4-6").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-6");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_normalize_opus_4_6_dot_prefers_4_6_over_4() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00002),
                output_cost_per_token: Some(0.0001),
                ..Default::default()
            },
        );
        litellm.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00005),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup("opus-4.6").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-6");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_normalize_opus_4_60_does_not_degrade_to_opus_4() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00002),
                output_cost_per_token: Some(0.0001),
                ..Default::default()
            },
        );
        litellm.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00005),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        assert!(lookup.lookup("opus-4-60").is_none());
    }

    #[test]
    fn test_normalize_opus_4_7_prefers_4_7_over_4() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),
                output_cost_per_token: Some(0.000075),
                ..Default::default()
            },
        );
        litellm.insert(
            "claude-opus-4-7".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup("opus-4-7").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-7");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_normalize_opus_4_7_dot_prefers_4_7_over_4() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),
                output_cost_per_token: Some(0.000075),
                ..Default::default()
            },
        );
        litellm.insert(
            "claude-opus-4-7".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup("opus-4.7").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-7");
        assert_eq!(result.source, "LiteLLM");
    }

    /// Regression: `aws.claude-opus-4-7` (Bedrock-style id) used to degrade
    /// to OpenRouter's `anthropic/claude-opus-4` ($15/$75/$1.50/$18.75 per M)
    /// because `normalize_model_name` only knew 4.5/4.6 and fell through to
    /// the bare `claude-opus-4` branch — which OpenRouter then resolved via
    /// `model_part` index to the legacy opus 4 entry. Result was ~3x overcharge.
    #[test]
    fn test_aws_opus_4_7_does_not_degrade_to_opus_4() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4-7".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                cache_read_input_token_cost: Some(5e-7),
                cache_creation_input_token_cost: Some(0.00000625),
                ..Default::default()
            },
        );
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),
                output_cost_per_token: Some(0.000075),
                cache_read_input_token_cost: Some(0.0000015),
                cache_creation_input_token_cost: Some(0.00001875),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let result = lookup.lookup("aws.claude-opus-4-7").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-7");
        assert_ne!(result.matched_key, "anthropic/claude-opus-4");

        // 8.4M input + 873K output + 41.3M cache_read + 12.1M cache_write
        // at opus-4-7 rates should be ~$160, not ~$480 (legacy opus 4).
        let cost = lookup.calculate_cost(
            "aws.claude-opus-4-7",
            8_400_000,
            873_000,
            41_300_000,
            12_100_000,
            0,
        );
        assert!(
            (140.0..=180.0).contains(&cost),
            "expected opus-4-7 priced cost around $160, got ${cost:.2}"
        );
    }

    #[test]
    fn test_unknown_future_opus_minor_does_not_degrade_to_opus_4() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),
                output_cost_per_token: Some(0.000075),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(HashMap::new(), openrouter, HashMap::new());

        assert!(lookup.lookup("claude-opus-4-8").is_none());
        assert!(lookup.lookup("aws.claude-opus-4-8").is_none());
    }

    #[test]
    fn test_normalize_opus_14_6_does_not_map_to_4_6() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00005),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        assert!(lookup.lookup("opus-14-6").is_none());
    }

    #[test]
    fn test_normalize_sonnet_14_5_does_not_map_to_4_5() {
        assert_eq!(normalize_model_name("sonnet-14-5"), None);
    }

    #[test]
    fn test_normalize_haiku_14_5_does_not_map_to_4_5() {
        assert_eq!(normalize_model_name("haiku-14-5"), None);
    }

    // =========================================================================
    // Generalized Claude family/major/minor normalization (PR #634 rework)
    // =========================================================================

    /// Synthetic dataset mirroring real LiteLLM/OpenRouter key shapes, with
    /// deliberately adversarial gaps: bedrock-style `us.anthropic.` keys exist
    /// for opus but not sonnet, and OpenRouter carries a pricier opus `-fast`
    /// variant that the old fallbacks degraded other families onto.
    fn claude_family_fixture() -> PricingLookup {
        fn p(input: f64, output: f64) -> ModelPricing {
            ModelPricing {
                input_cost_per_token: Some(input),
                output_cost_per_token: Some(output),
                ..Default::default()
            }
        }

        let mut litellm = HashMap::new();
        litellm.insert("claude-opus-4".to_string(), p(15e-6, 75e-6));
        litellm.insert("claude-opus-4-1".to_string(), p(15e-6, 75e-6));
        litellm.insert("claude-opus-4-5".to_string(), p(5e-6, 25e-6));
        litellm.insert("claude-opus-4-6".to_string(), p(5e-6, 25e-6));
        litellm.insert("claude-opus-4-7".to_string(), p(5e-6, 25e-6));
        litellm.insert("claude-opus-4-8".to_string(), p(5e-6, 25e-6));
        litellm.insert("claude-sonnet-4".to_string(), p(3e-6, 15e-6));
        litellm.insert("claude-sonnet-4-5".to_string(), p(3e-6, 15e-6));
        litellm.insert("claude-sonnet-4-6".to_string(), p(3e-6, 15e-6));
        litellm.insert("claude-haiku-4-5".to_string(), p(1e-6, 5e-6));
        litellm.insert("us.anthropic.claude-opus-4-8".to_string(), p(5e-6, 25e-6));
        litellm.insert("vertex_ai/claude-sonnet-4-6".to_string(), p(3e-6, 15e-6));

        let mut openrouter = HashMap::new();
        openrouter.insert("anthropic/claude-opus-4".to_string(), p(15e-6, 75e-6));
        openrouter.insert("anthropic/claude-opus-4.8".to_string(), p(5e-6, 25e-6));
        openrouter.insert("anthropic/claude-opus-4.8-fast".to_string(), p(7e-6, 30e-6));
        openrouter.insert("anthropic/claude-sonnet-4.6".to_string(), p(3e-6, 15e-6));
        openrouter.insert("anthropic/claude-haiku-4.5".to_string(), p(1e-6, 5e-6));
        openrouter.insert("anthropic/claude-fable-5".to_string(), p(5e-6, 25e-6));

        PricingLookup::new(litellm, openrouter, HashMap::new())
    }

    #[test]
    fn test_normalize_minor_generalizes_across_families() {
        assert_eq!(
            normalize_model_name("claude-sonnet-4-7"),
            Some("claude-sonnet-4-7".into())
        );
        assert_eq!(
            normalize_model_name("sonnet-4.7"),
            Some("claude-sonnet-4-7".into())
        );
        assert_eq!(
            normalize_model_name("claude-haiku-4-6"),
            Some("claude-haiku-4-6".into())
        );
        assert_eq!(
            normalize_model_name("haiku-4.6"),
            Some("claude-haiku-4-6".into())
        );
        assert_eq!(
            normalize_model_name("claude-opus-4-9"),
            Some("claude-opus-4-9".into())
        );
        assert_eq!(
            normalize_model_name("opus-4.9"),
            Some("claude-opus-4-9".into())
        );
        assert_eq!(
            normalize_model_name("opus-5-2"),
            Some("claude-opus-5-2".into())
        );
    }

    #[test]
    fn test_normalize_reversed_order_all_families() {
        assert_eq!(
            normalize_model_name("claude-4-8-opus"),
            Some("claude-opus-4-8".into())
        );
        assert_eq!(
            normalize_model_name("4-8-opus"),
            Some("claude-opus-4-8".into())
        );
        assert_eq!(
            normalize_model_name("claude-4-6-sonnet"),
            Some("claude-sonnet-4-6".into())
        );
        assert_eq!(
            normalize_model_name("claude-4-5-haiku"),
            Some("claude-haiku-4-5".into())
        );
    }

    #[test]
    fn test_normalize_bare_modern_major() {
        assert_eq!(
            normalize_model_name("claude-sonnet-5"),
            Some("claude-sonnet-5".into())
        );
        assert_eq!(
            normalize_model_name("claude-opus-5"),
            Some("claude-opus-5".into())
        );
        assert_eq!(
            normalize_model_name("fable-5"),
            Some("claude-fable-5".into())
        );
        assert_eq!(
            normalize_model_name("claude-fable-5[1m]"),
            Some("claude-fable-5".into())
        );
    }

    /// Boundary contract preserved from main's hardcoded matcher: two-digit
    /// minors and majors, zero minors, undelimited versions, and dated forms
    /// must not normalize to a coarser key. (PR #634's original parser
    /// degraded `opus-4-60` to `claude-opus-4`; main's contract is None.)
    #[test]
    fn test_normalize_modern_claude_boundaries() {
        assert_eq!(normalize_model_name("opus-4-60"), None);
        assert_eq!(normalize_model_name("sonnet-4-60"), None);
        assert_eq!(normalize_model_name("opus-14-6"), None);
        assert_eq!(normalize_model_name("opus4"), None);
        assert_eq!(normalize_model_name("opus-4x"), None);
        assert_eq!(normalize_model_name("opus-3"), None);
        assert_eq!(normalize_model_name("claude-sonnet-5-0"), None);
        assert_eq!(normalize_model_name("claude-opus-4-20250514"), None);
    }

    /// Legacy 3.x ids keep their irregular canonical keys; the reversed-order
    /// and bare-major parsing must not hijack the digit pairs in them.
    #[test]
    fn test_normalize_legacy_line_not_hijacked_by_modern_parser() {
        assert_eq!(
            normalize_model_name("claude-3-5-sonnet"),
            Some("claude-3.5-sonnet".into())
        );
        assert_eq!(
            normalize_model_name("claude-3-7-sonnet-20250219"),
            Some("claude-3-7-sonnet".into())
        );
        assert_eq!(
            normalize_model_name("claude-3-5-haiku-20241022"),
            Some("claude-3.5-haiku".into())
        );
    }

    /// Regression (B1): a bedrock-style sonnet id must never be billed at an
    /// opus key. Before the family guard, `us.anthropic.claude-sonnet-4-6-v1:0`
    /// suffix-stripped down to `us.anthropic.claude` and fuzzy-matched the
    /// dataset's `us.anthropic.claude-opus-4-8` entry ($5/M instead of $3/M).
    #[test]
    fn test_bedrock_sonnet_never_billed_as_opus() {
        let lookup = claude_family_fixture();
        let result = lookup
            .lookup("us.anthropic.claude-sonnet-4-6-v1:0")
            .unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-6");
        assert_eq!(result.pricing.input_cost_per_token, Some(3e-6));
    }

    /// Regression (B2): reversed-order sonnet ids must resolve to the sonnet
    /// key, not cross-family. Before reversed-order parsing was generalized
    /// beyond opus, `claude-4-6-sonnet` stripped down to `claude` and
    /// fuzzy-matched `anthropic/claude-opus-4.8-fast`.
    #[test]
    fn test_reversed_sonnet_resolves_canonical_not_cross_family() {
        let lookup = claude_family_fixture();
        for id in ["claude-4-6-sonnet", "4-6-sonnet"] {
            let result = lookup.lookup(id).unwrap();
            assert_eq!(result.matched_key, "claude-sonnet-4-6", "id: {id}");
        }
        let result = lookup.lookup("claude-4-5-haiku").unwrap();
        assert_eq!(result.matched_key, "claude-haiku-4-5");
    }

    /// Regression (B3): the never-degrade contract that
    /// `test_unknown_future_opus_minor_does_not_degrade_to_opus_4` pins for
    /// opus now holds for sonnet and haiku too. Unknown minors previously
    /// degraded: `sonnet-4-7` -> claude-sonnet-4.6, `haiku-4-6` ->
    /// claude-haiku-4.5 (and with real data even claude-3.5-haiku).
    #[test]
    fn test_unknown_sonnet_haiku_minor_does_not_degrade() {
        let lookup = claude_family_fixture();
        for id in [
            "sonnet-4-7",
            "claude-sonnet-4-7",
            "sonnet-4-60",
            "haiku-4-6",
            "claude-haiku-4-6",
        ] {
            assert!(lookup.lookup(id).is_none(), "id {id} must not degrade");
        }
    }

    /// Regression (B4): major >= 5 ids resolve to a dataset-known exact id
    /// when one exists, else None — never to a different major. Previously
    /// `claude-opus-5` resolved to `anthropic/claude-opus-4.8-fast` and
    /// `sonnet-5`/`claude-sonnet-5-0` to sonnet 4.6, while bare `opus-5`
    /// happened to return None only because of a fuzzy length cutoff.
    #[test]
    fn test_major_five_never_resolves_to_different_major() {
        let lookup = claude_family_fixture();
        for id in [
            "claude-opus-5",
            "opus-5",
            "opus-5-2",
            "sonnet-5",
            "claude-sonnet-5-0",
        ] {
            assert!(
                lookup.lookup(id).is_none(),
                "id {id} must not resolve to a 4.x key"
            );
        }

        // fable-5 is dataset-known (OpenRouter) and resolves in all forms.
        for id in [
            "claude-fable-5",
            "fable-5",
            "claude-fable-5[1m]",
            "anthropic/claude-fable-5",
        ] {
            let result = lookup.lookup(id).unwrap();
            assert_eq!(result.matched_key, "anthropic/claude-fable-5", "id: {id}");
        }
    }

    /// Regression (#831): router/proxy-assigned ids like `cx/gpt-5.5` (seen
    /// from OpenCode's `omniroute` provider) carry a prefix outside the
    /// curated `PROVIDER_PREFIXES` list, so the pricing lookup used to return
    /// `None` (and thus bill $0) instead of stripping the prefix and pricing
    /// the underlying `gpt-5.5` model.
    #[test]
    fn test_unknown_prefixed_model_id_strips_to_underlying_model() {
        let lookup = create_lookup();
        let direct = lookup.lookup("gpt-5.5").unwrap();
        let prefixed = lookup.lookup("cx/gpt-5.5").unwrap();
        assert_eq!(prefixed.matched_key, direct.matched_key);
        assert_eq!(prefixed.source, direct.source);
        assert_eq!(
            prefixed.pricing.input_cost_per_token,
            direct.pricing.input_cost_per_token
        );
        assert_eq!(
            prefixed.pricing.output_cost_per_token,
            direct.pricing.output_cost_per_token
        );
    }

    /// Regression (#846): an id carrying both a routing prefix and a tier
    /// suffix resolved to nothing, so real usage billed $0. Each id below
    /// resolves once one transformation is applied, but the two were never
    /// applied together: prefix stripping only retried the terminal segment
    /// as-is, and suffix stripping splits on `-`, so it never shed the `cx/`.
    #[test]
    fn test_routing_prefix_and_tier_suffix_strip_together() {
        let lookup = create_lookup();
        let expected = lookup.lookup("gpt-5.5").unwrap();

        for id in [
            "cx/gpt-5.5-xhigh",
            "cx/gpt-5.5-high",
            "cx/gpt-5.5-medium",
            "cx/gpt-5.5-low",
        ] {
            let result = lookup
                .lookup(id)
                .unwrap_or_else(|| panic!("{id} must resolve"));
            assert_eq!(result.matched_key, expected.matched_key, "id: {id}");
            assert_eq!(
                result.pricing.input_cost_per_token, expected.pricing.input_cost_per_token,
                "id: {id}"
            );
        }
    }

    /// Regression (#831): a dataset key that legitimately keeps its own
    /// provider prefix (e.g. `anthropic/claude-fable-5`, which exists as its
    /// own OpenRouter key) must still resolve via the exact/direct lookup —
    /// the new generic prefix-stripping fallback must not preempt it.
    #[test]
    fn test_known_prefixed_dataset_key_still_resolves_exactly() {
        let lookup = claude_family_fixture();
        let result = lookup.lookup("anthropic/claude-fable-5").unwrap();
        assert_eq!(result.matched_key, "anthropic/claude-fable-5");
    }

    /// Regression (#831): an id with an unrecognized provider prefix AND an
    /// unrecognized underlying model must still return `None` rather than
    /// fuzzy-matching something unrelated.
    #[test]
    fn test_unknown_prefixed_unknown_model_stays_none() {
        let lookup = create_lookup();
        assert!(lookup.lookup("unknown/nonexistent").is_none());
    }

    /// When the dataset later gains a major-5 key, the same ids resolve to it
    /// with no code change — the "known version" decision is dataset-driven.
    #[test]
    fn test_major_five_resolves_once_dataset_knows_it() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00005),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        for id in ["claude-opus-5", "opus-5", "aws.claude-opus-5-thinking"] {
            let result = lookup.lookup(id).unwrap();
            assert_eq!(result.matched_key, "claude-opus-5", "id: {id}");
        }
    }

    /// Known minors keep resolving across the id shapes seen in the wild:
    /// dotted versions, vendor prefixes, tier/feature suffixes.
    #[test]
    fn test_known_minor_shapes_resolve_per_family() {
        let lookup = claude_family_fixture();
        let cases = [
            ("opus-4-8", "claude-opus-4-8"),
            ("opus-4.8", "claude-opus-4-8"),
            ("aws.claude-opus-4-8", "claude-opus-4-8"),
            ("claude-opus-4-8-thinking", "claude-opus-4-8"),
            ("claude-sonnet-4-6", "claude-sonnet-4-6"),
            ("claude-sonnet-4.6", "claude-sonnet-4-6"),
            ("sonnet-4-6", "claude-sonnet-4-6"),
            ("sonnet-4.6", "claude-sonnet-4-6"),
            ("aws.claude-sonnet-4-6-v1", "claude-sonnet-4-6"),
            ("claude-sonnet-4-6-thinking", "claude-sonnet-4-6"),
            ("haiku-4-5", "claude-haiku-4-5"),
            ("haiku-4.5", "claude-haiku-4-5"),
            ("vertex_ai/claude-sonnet-4-6", "vertex_ai/claude-sonnet-4-6"),
        ];
        for (id, expected) in cases {
            let result = lookup.lookup(id).unwrap();
            assert_eq!(result.matched_key, expected, "id: {id}");
        }
    }

    /// Ported from PR #634: the next opus minor must prefer its own key over
    /// the bare `claude-opus-4` catch-all, in dashed and dotted forms.
    #[test]
    fn test_normalize_opus_4_8_prefers_4_8_over_4() {
        let lookup = claude_family_fixture();
        for id in ["opus-4-8", "opus-4.8"] {
            let result = lookup.lookup(id).unwrap();
            assert_eq!(result.matched_key, "claude-opus-4-8", "id: {id}");
            assert_eq!(result.source, "LiteLLM");
        }
    }

    /// Ported from PR #634: `aws.claude-opus-4-8` must not degrade to
    /// OpenRouter's legacy `anthropic/claude-opus-4` (~3x overcharge).
    #[test]
    fn test_aws_opus_4_8_does_not_degrade_to_opus_4() {
        let lookup = claude_family_fixture();
        let result = lookup.lookup("aws.claude-opus-4-8").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-8");

        // 8.4M input + 873K output at opus-4-8 rates is ~$64, not ~$191
        // (legacy opus 4 at $15/$75 per M).
        let cost = lookup.calculate_cost("aws.claude-opus-4-8", 8_400_000, 873_000, 0, 0, 0);
        assert!(
            (60.0..=70.0).contains(&cost),
            "expected opus-4-8 priced cost around $64, got ${cost:.2}"
        );
    }

    /// Regression (post-#634 catalog audit, bug 1): retired `claude-2.x` ids
    /// (present in historical usage logs, absent from every pricing dataset)
    /// must resolve to None, not to a modern model's price. Previously
    /// `try_strip_unknown_suffix` eroded `claude-2.1` to bare `claude`
    /// (the "2.1" segment failed the all-digits version check), which then
    /// fuzzy-matched `anthropic/claude-opus-4.7-fast` at $30/$150. The #634
    /// family veto was bypassed because `claude-2.1` carries no
    /// opus/sonnet/haiku/fable token.
    #[test]
    fn claude_2x_never_fuzzy_matches_modern_models() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4.7-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(30e-6),
                output_cost_per_token: Some(150e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(HashMap::new(), openrouter, HashMap::new());

        for id in ["claude-2.1", "claude-2.0", "claude", "anthropic"] {
            assert!(
                lookup.lookup(id).is_none(),
                "id {id} must resolve unpriced, never to another model's price"
            );
        }
    }

    /// Positive control for the claude-2.x guards: when a dataset actually
    /// prices `claude-2.1`, it still resolves — the guards only block the
    /// erosion-to-bare-brand path, not legitimate dataset hits.
    #[test]
    fn claude_2x_still_resolves_when_dataset_prices_it() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-2.1".to_string(),
            ModelPricing {
                input_cost_per_token: Some(8e-6),
                output_cost_per_token: Some(24e-6),
                ..Default::default()
            },
        );
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4.7-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(30e-6),
                output_cost_per_token: Some(150e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());

        let result = lookup.lookup("claude-2.1").unwrap();
        assert_eq!(result.matched_key, "claude-2.1");
        assert_eq!(result.pricing.input_cost_per_token, Some(8e-6));
    }

    /// Regression (post-#634 catalog audit, bug 2): `claude-opus-4-6-fast`
    /// must hit the canonical OpenRouter `anthropic/claude-opus-4.6-fast`
    /// key ($30/$150) via separator normalization, not Models.dev's reseller
    /// `venice/claude-opus-4-6-fast` markup ($36/$180). Previously the
    /// models.dev model-part pass ran before the version-normalized
    /// OpenRouter exact pass in `lookup_auto`.
    #[test]
    fn canonical_fast_price_beats_reseller_markup() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4.6-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(30e-6),
                output_cost_per_token: Some(150e-6),
                ..Default::default()
            },
        );
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "venice/claude-opus-4-6-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(36e-6),
                output_cost_per_token: Some(180e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            openrouter,
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let result = lookup.lookup("claude-opus-4-6-fast").unwrap();
        assert_eq!(result.matched_key, "anthropic/claude-opus-4.6-fast");
        assert_eq!(result.pricing.input_cost_per_token, Some(30e-6));
    }

    /// Regression (#707 review): a provider hint pins the lookup to that
    /// provider's catalog. The canonical-source reorder asserted by
    /// `canonical_fast_price_beats_reseller_markup` only applies to unhinted
    /// lookups; with `provider_id = Some("venice")` the provider-scoped
    /// models.dev pass must win over OpenRouter's unscoped `anthropic/...`
    /// row, so provider-aware callers get the hinted provider's price.
    #[test]
    fn provider_hint_keeps_models_dev_provider_key_over_unscoped_canonical() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4.6-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(30e-6),
                output_cost_per_token: Some(150e-6),
                ..Default::default()
            },
        );
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "venice/claude-opus-4-6-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(36e-6),
                output_cost_per_token: Some(180e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            openrouter,
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let hinted = lookup
            .lookup_with_provider("claude-opus-4-6-fast", Some("venice"))
            .unwrap();
        assert_eq!(hinted.matched_key, "venice/claude-opus-4-6-fast");
        assert_eq!(hinted.pricing.input_cost_per_token, Some(36e-6));

        // Unhinted lookups keep the canonical resolution.
        let unhinted = lookup.lookup("claude-opus-4-6-fast").unwrap();
        assert_eq!(unhinted.matched_key, "anthropic/claude-opus-4.6-fast");
        assert_eq!(unhinted.pricing.input_cost_per_token, Some(30e-6));
    }

    /// Regression (#1004 follow-up): a reseller provider hint must select the
    /// reseller-scoped models.dev row instead of a direct upstream catalog row
    /// with the same terminal model id.
    #[test]
    fn orcarouter_hint_selects_orcarouter_models_dev_row() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "openai/gpt-5.5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(5e-6),
                output_cost_per_token: Some(30e-6),
                ..Default::default()
            },
        );
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "orcarouter/openai/gpt-5.5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(8e-6),
                output_cost_per_token: Some(48e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            openrouter,
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let result = lookup
            .lookup_with_provider("gpt-5.5", Some("orcarouter"))
            .unwrap();
        assert_eq!(result.source, "Models.dev");
        assert_eq!(result.matched_key, "orcarouter/openai/gpt-5.5");
        assert_eq!(result.pricing.input_cost_per_token, Some(8e-6));
    }

    /// Regression (#707 review, cubic follow-up): the provider-hint pin must
    /// also beat the unscoped OpenRouter MODEL-PART fallback, not just the
    /// separator-normalized passes. When the hinted provider's models.dev key
    /// shares the dotted model-part spelling that OpenRouter already indexes
    /// (here both `claude-opus-4.6-fast`), an unscoped model-part match would
    /// otherwise return `anthropic/...` before the provider-scoped pass ran.
    #[test]
    fn provider_hint_beats_unscoped_openrouter_model_part_for_dotted_id() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4.6-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(30e-6),
                output_cost_per_token: Some(150e-6),
                ..Default::default()
            },
        );
        let mut models_dev = HashMap::new();
        // Hinted provider's key uses the SAME dotted spelling OpenRouter
        // indexes as a model-part — this is what makes the unscoped model-part
        // pass fire first without the fix.
        models_dev.insert(
            "venice/claude-opus-4.6-fast".to_string(),
            ModelPricing {
                input_cost_per_token: Some(36e-6),
                output_cost_per_token: Some(180e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            openrouter,
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        // Hinted dotted lookup must pin to venice, not the canonical OpenRouter
        // model-part it also matches.
        let hinted = lookup
            .lookup_with_provider("claude-opus-4.6-fast", Some("venice"))
            .unwrap();
        assert_eq!(hinted.matched_key, "venice/claude-opus-4.6-fast");
        assert_eq!(hinted.pricing.input_cost_per_token, Some(36e-6));
        assert_eq!(hinted.evidence.kind, ResolutionKind::ProviderScoped);
        assert!(hinted.evidence.is_submission_safe());

        // Unhinted dotted lookup keeps the canonical OpenRouter resolution.
        let unhinted = lookup.lookup("claude-opus-4.6-fast").unwrap();
        assert_eq!(unhinted.matched_key, "anthropic/claude-opus-4.6-fast");
        assert_eq!(unhinted.pricing.input_cost_per_token, Some(30e-6));

        // A hint for a provider with no matching key must still fall through to
        // the canonical resolution rather than returning None.
        let no_match = lookup
            .lookup_with_provider("claude-opus-4.6-fast", Some("groq"))
            .unwrap();
        assert_eq!(no_match.matched_key, "anthropic/claude-opus-4.6-fast");
        assert_eq!(no_match.pricing.input_cost_per_token, Some(30e-6));
    }

    /// Regression (#707 review): the anthropic-first preference in the
    /// models.dev model-part index must only choose among priced keys. An
    /// unpriced (all-None) `anthropic/<model>` row must not shadow a priced
    /// reseller row, which would bill the model at zero cost.
    #[test]
    fn unpriced_anthropic_models_dev_key_does_not_shadow_priced_reseller() {
        let mut models_dev = HashMap::new();
        models_dev.insert("anthropic/model-x".to_string(), ModelPricing::default());
        models_dev.insert(
            "reseller/model-x".to_string(),
            ModelPricing {
                input_cost_per_token: Some(36e-6),
                output_cost_per_token: Some(180e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let result = lookup.lookup("model-x").unwrap();
        assert_eq!(result.matched_key, "reseller/model-x");
        assert_eq!(result.pricing.input_cost_per_token, Some(36e-6));
    }

    /// After the lookup_auto reorder, models.dev must remain the long-tail
    /// fallback for ids no canonical source knows.
    #[test]
    fn models_dev_still_covers_long_tail_after_reorder() {
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "someprovider/exotic-model-9".to_string(),
            ModelPricing {
                input_cost_per_token: Some(2e-6),
                output_cost_per_token: Some(6e-6),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let result = lookup.lookup("exotic-model-9").unwrap();
        assert_eq!(result.matched_key, "someprovider/exotic-model-9");
        assert_eq!(result.pricing.input_cost_per_token, Some(2e-6));
    }

    /// Regression (post-#634 catalog audit, bug 2b): when multiple models.dev
    /// providers share a model part, the winner must be deterministic and
    /// prefer the canonical `anthropic/` namespace. Previously the winner
    /// depended on HashMap iteration order (with real data `302ai/` beat
    /// `anthropic/` for claude-3-5-haiku-20241022 because shorter keys were
    /// inserted last).
    #[test]
    fn models_dev_provider_choice_is_deterministic_and_prefers_anthropic() {
        let price = ModelPricing {
            input_cost_per_token: Some(0.8e-6),
            output_cost_per_token: Some(4e-6),
            ..Default::default()
        };
        // Adversarial insertion order: the non-canonical provider first.
        let mut models_dev = HashMap::new();
        models_dev.insert("302ai/claude-3-5-haiku-20241022".to_string(), price.clone());
        models_dev.insert(
            "anthropic/claude-3-5-haiku-20241022".to_string(),
            price.clone(),
        );
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let result = lookup.lookup("claude-3-5-haiku-20241022").unwrap();
        assert_eq!(result.matched_key, "anthropic/claude-3-5-haiku-20241022");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.8e-6));
    }

    #[test]
    fn test_blocklist_auto() {
        let lookup = create_lookup();
        assert!(lookup.lookup("auto").is_none());
    }

    #[test]
    fn test_blocklist_mini() {
        let lookup = create_lookup();
        assert!(lookup.lookup("mini").is_none());
    }

    #[test]
    fn test_force_source_litellm() {
        let lookup = create_lookup();
        let result = lookup
            .lookup_with_source("gpt-4o", Some("litellm"))
            .unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.matched_key, "gpt-4o");
    }

    #[test]
    fn test_force_source_openrouter() {
        let lookup = create_lookup();
        let result = lookup
            .lookup_with_source("gpt-4o", Some("openrouter"))
            .unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.matched_key, "openai/gpt-4o");
    }

    #[test]
    fn test_case_insensitive() {
        let lookup = create_lookup();
        let result = lookup.lookup("GPT-4O").unwrap();
        assert_eq!(result.matched_key, "gpt-4o");
    }

    #[test]
    fn test_fuzzy_match_gemini() {
        let lookup = create_lookup();
        let result = lookup.lookup("gemini-3-pro").unwrap();
        assert_eq!(result.matched_key, "openrouter/google/gemini-3-pro-preview");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_tier_suffix_with_fuzzy() {
        let lookup = create_lookup();
        let result = lookup.lookup("gemini-3-pro-high").unwrap();
        assert_eq!(result.matched_key, "openrouter/google/gemini-3-pro-preview");
    }

    #[test]
    fn test_nonexistent_model() {
        let lookup = create_lookup();
        assert!(lookup.lookup("nonexistent-model-xyz").is_none());
    }

    #[test]
    fn test_fallback_suffix_lookup() {
        // Create a lookup with only the base model (no -codex variant)
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(1.25e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        // Note: gpt-5-codex is NOT in the pricing data

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        // Looking up gpt-5-codex should fall back to gpt-5
        let result = lookup.lookup("gpt-5-codex").unwrap();
        assert_eq!(result.matched_key, "gpt-5");
        assert_eq!(result.source, "LiteLLM");

        // Looking up gpt-5-codex-max should also fall back to gpt-5
        let result = lookup.lookup("gpt-5-codex-max").unwrap();
        assert_eq!(result.matched_key, "gpt-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_fallback_suffix_with_tier_suffix() {
        // Test that tier suffix + fallback suffix both work together
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: Some(1.25e-7),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        // gpt-5-codex-high should strip -high first, then fall back from gpt-5-codex to gpt-5
        let result = lookup.lookup("gpt-5-codex-high").unwrap();
        assert_eq!(result.matched_key, "gpt-5");
        assert_eq!(result.source, "LiteLLM");

        // gpt-5-codex-max-xhigh should strip -xhigh first, then fall back from gpt-5-codex-max to gpt-5
        let result = lookup.lookup("gpt-5-codex-max-xhigh").unwrap();
        assert_eq!(result.matched_key, "gpt-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn fuzzy_resolution_records_conflicting_candidates_as_submission_unsafe() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "vendor-a/atlas-chat-preview".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000001),
                output_cost_per_token: Some(0.000002),
                ..Default::default()
            },
        );
        litellm.insert(
            "vendor-b/atlas-chat-beta".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000006),
                ..Default::default()
            },
        );

        let result = PricingLookup::new(litellm, HashMap::new(), HashMap::new())
            .lookup("atlas-chat")
            .expect("reporting lookup should still expose its estimate");

        assert_eq!(result.evidence.kind, ResolutionKind::Fuzzy);
        assert_eq!(result.evidence.candidate_count, 2);
        assert!(!result.evidence.price_consensus);
        assert!(!result.evidence.exact_model_identity);
        assert!(!result.evidence.is_submission_safe());
    }

    #[test]
    fn fuzzy_resolution_accepts_exact_terminal_identity_with_price_consensus() {
        let same_price = ModelPricing {
            input_cost_per_token: Some(0.000001),
            output_cost_per_token: Some(0.000002),
            ..Default::default()
        };
        let litellm = HashMap::from([
            ("gateway-a/atlas-chat".into(), same_price.clone()),
            ("gateway-b/atlas-chat".into(), same_price),
        ]);

        let result = PricingLookup::new(litellm, HashMap::new(), HashMap::new())
            .lookup("unknown-router/atlas-chat")
            .expect("the stripped terminal identity should resolve");

        assert_eq!(result.evidence.kind, ResolutionKind::Fuzzy);
        assert_eq!(result.evidence.candidate_count, 2);
        assert!(result.evidence.price_consensus);
        assert!(result.evidence.exact_model_identity);
        assert!(result.evidence.stripped);
        assert!(result.evidence.is_submission_safe());
    }

    /// A provider-hinted row that resolves deterministically is publishable on
    /// its own evidence, but the rates it borrows to cover a bucket it does not
    /// price are only as trustworthy as the row they came from. Filling from an
    /// ambiguous fuzzy canonical row must not turn that row's price into a
    /// submitted one.
    #[test]
    fn borrowed_rates_from_an_ambiguous_canonical_row_are_not_submission_safe() {
        let disputed_cache_row = |cache_read: f64| ModelPricing {
            input_cost_per_token: Some(1e-6),
            output_cost_per_token: Some(2e-6),
            cache_read_input_token_cost: Some(cache_read),
            ..Default::default()
        };
        let litellm = HashMap::from([
            (
                "azure_ai/atlas-chat".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(1e-6),
                    output_cost_per_token: Some(2e-6),
                    ..Default::default()
                },
            ),
            (
                "vendor-a/atlas-chat-preview".to_string(),
                disputed_cache_row(5e-7),
            ),
            (
                "vendor-b/atlas-chat-beta".to_string(),
                disputed_cache_row(9e-7),
            ),
        ]);
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 20,
            cache_write: 0,
            reasoning: 0,
        };

        let hinted = lookup
            .lookup_with_provider("atlas-chat", Some("azure"))
            .expect("the provider-hinted row resolves deterministically");
        assert_eq!(hinted.matched_key, "azure_ai/atlas-chat");
        assert!(hinted.evidence.is_submission_safe());

        let canonical = lookup
            .lookup_with_provider("atlas-chat", None)
            .expect("the unhinted lookup falls back to a fuzzy estimate");
        assert_eq!(canonical.evidence.kind, ResolutionKind::Fuzzy);
        assert!(!canonical.evidence.is_submission_safe());

        // The cache-read rate is borrowed from a row the resolver already
        // refused to publish, and the candidates it was chosen from disagree
        // about it (5e-7 against 9e-7).
        let resolved = lookup
            .resolve_for_usage("atlas-chat", Some("azure"), &usage)
            .expect("the hinted row still resolves");
        assert_eq!(resolved.matched_key, "azure_ai/atlas-chat");
        assert_eq!(resolved.pricing.cache_read_input_token_cost, Some(5e-7));
        assert!(resolved.pricing.covers_usage(&usage));
        assert_eq!(
            resolved.evidence.submission_safety_gap(),
            Some(SubmissionSafetyGap::PriceDisagreement)
        );
        assert!(!resolved.evidence.is_submission_safe());

        // The estimate itself stays visible: this separates estimates from
        // submissions, it does not stop reporting the cache-read cost.
        let cost = lookup.calculate_cost_with_provider("atlas-chat", Some("azure"), &usage);
        assert!((cost - (100.0 * 1e-6 + 50.0 * 2e-6 + 20.0 * 5e-7)).abs() < 1e-12);
    }

    /// The counterpart to the guard above: when the canonical row is itself
    /// publishable, borrowing its rate must still produce a submittable price.
    /// This is the #1013 behaviour the borrow exists for.
    #[test]
    fn borrowed_rates_from_a_submission_safe_canonical_row_still_submit() {
        let litellm = HashMap::from([
            (
                "azure_ai/atlas-chat".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(1e-6),
                    output_cost_per_token: Some(2e-6),
                    ..Default::default()
                },
            ),
            (
                "atlas-chat".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(1e-6),
                    output_cost_per_token: Some(2e-6),
                    cache_read_input_token_cost: Some(5e-7),
                    ..Default::default()
                },
            ),
        ]);
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let usage = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 20,
            cache_write: 0,
            reasoning: 0,
        };

        let resolved = lookup
            .resolve_for_usage("atlas-chat", Some("azure"), &usage)
            .expect("the hinted row resolves");
        assert_eq!(resolved.matched_key, "azure_ai/atlas-chat");
        assert_eq!(resolved.pricing.cache_read_input_token_cost, Some(5e-7));
        assert!(resolved.evidence.is_submission_safe());
        assert!(resolved.pricing.covers_usage(&usage));
    }

    #[test]
    fn test_fallback_suffix_prefers_exact_match() {
        // If the exact model exists, it should be used (no fallback)
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000125),
                output_cost_per_token: Some(0.00001),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );
        litellm.insert(
            "gpt-5-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000002), // Different price to verify which one is used
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        // Should use the exact match, not fall back
        let result = lookup.lookup("gpt-5-codex").unwrap();
        assert_eq!(result.matched_key, "gpt-5-codex");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn test_normalize_version_separator() {
        assert_eq!(
            normalize_version_separator("glm-4-7"),
            Some("glm-4.7".into())
        );
        assert_eq!(
            normalize_version_separator("glm-4-6"),
            Some("glm-4.6".into())
        );
        assert_eq!(
            normalize_version_separator("claude-3-5-haiku"),
            Some("claude-3.5-haiku".into())
        );
        assert_eq!(
            normalize_version_separator("gpt-5-1-codex"),
            Some("gpt-5.1-codex".into())
        );
        assert_eq!(normalize_version_separator("gpt-4o"), None);
        assert_eq!(normalize_version_separator("claude-sonnet"), None);
        assert_eq!(normalize_version_separator("big-pickle"), None);
    }

    #[test]
    fn test_normalize_version_separator_preserves_dates() {
        assert_eq!(normalize_version_separator("2024-11-20"), None);
        assert_eq!(normalize_version_separator("model-2024-11-20"), None);
        assert_eq!(
            normalize_version_separator("claude-3-5-sonnet-20241022"),
            Some("claude-3.5-sonnet-20241022".into())
        );
        assert_eq!(normalize_version_separator("sonnet-20241022"), None);
        assert_eq!(normalize_version_separator("model-20241022-v1"), None);
    }

    #[test]
    fn test_is_fuzzy_eligible() {
        assert!(!is_fuzzy_eligible("auto"));
        assert!(!is_fuzzy_eligible("mini"));
        assert!(!is_fuzzy_eligible("chat"));
        assert!(!is_fuzzy_eligible("base"));
        assert!(!is_fuzzy_eligible("abc"));
        assert!(is_fuzzy_eligible("gpt-4o"));
        // Bare brand tokens carry no model information: a fuzzy hit from them
        // can land on any model of the brand, so they are blocklisted.
        assert!(!is_fuzzy_eligible("claude"));
        assert!(!is_fuzzy_eligible("anthropic"));
    }

    // =========================================================================
    // PROVIDER PREFERENCE TESTS
    // =========================================================================

    #[test]
    fn test_provider_preference_grok_prefers_xai_over_azure() {
        let lookup = create_lookup();
        let result = lookup.lookup("grok-code").unwrap();
        assert_eq!(result.matched_key, "xai/grok-code-fast-1-0825");
        assert_eq!(result.source, "LiteLLM");
        assert!(!result.matched_key.starts_with("azure"));
    }

    /// Test that documents the exact before/after behavior for grok-code provider preference.
    /// This test explicitly verifies that the original provider (xai/) is preferred over resellers (azure_ai/).
    #[test]
    fn test_grok_code_prefers_xai_over_azure() {
        // =========================================================================
        // BEFORE FIX: grok-code → azure_ai/grok-code-fast-1 ($3.50/$17.50) ❌ reseller
        // AFTER FIX:  grok-code → xai/grok-code-fast-1-0825 ($0.20/$1.50) ✅ original provider
        //
        // The azure_ai/ prefix indicates a reseller (Azure AI marketplace), which typically
        // has higher prices. The xai/ prefix indicates the original provider (X.AI/Grok),
        // which offers lower direct pricing. Our lookup should prefer the original provider.
        // =========================================================================

        let mut litellm = HashMap::new();

        // Reseller entry: azure_ai/ prefix with higher prices ($3.50/$17.50 per 1M tokens)
        litellm.insert(
            "azure_ai/grok-code-fast-1".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0000035),  // $3.50/1M tokens
                output_cost_per_token: Some(0.0000175), // $17.50/1M tokens
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        // Original provider entry: xai/ prefix with lower prices ($0.20/$1.50 per 1M tokens)
        litellm.insert(
            "xai/grok-code-fast-1-0825".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0000002),  // $0.20/1M tokens
                output_cost_per_token: Some(0.0000015), // $1.50/1M tokens
                cache_read_input_token_cost: Some(0.00000002),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup("grok-code").unwrap();

        // Must prefer xai (original provider) over azure_ai (reseller)
        assert!(
            result.matched_key.starts_with("xai/"),
            "Expected xai/ prefix (original provider) but got: {}. \
             The lookup should prefer original providers over resellers.",
            result.matched_key
        );
        assert_eq!(
            result.matched_key, "xai/grok-code-fast-1-0825",
            "Should match the xai/grok-code-fast-1-0825 entry, not azure_ai/grok-code-fast-1"
        );

        // Verify we got the lower price (original provider)
        let pricing = &result.pricing;
        assert!(
            pricing.input_cost_per_token.unwrap() < 0.000001,
            "Input cost should be ~$0.20/1M (0.0000002), not ~$3.50/1M (reseller price)"
        );
        assert!(
            pricing.output_cost_per_token.unwrap() < 0.000005,
            "Output cost should be ~$1.50/1M (0.0000015), not ~$17.50/1M (reseller price)"
        );
    }

    #[test]
    fn test_provider_preference_gemini_prefers_google_over_vertex() {
        let lookup = create_lookup();
        let result = lookup.lookup("gemini-2.5-pro").unwrap();
        assert_eq!(result.matched_key, "google/gemini-2.5-pro");
        assert_eq!(result.source, "LiteLLM");
        assert!(!result.matched_key.starts_with("vertex_ai"));
    }

    #[test]
    fn test_is_original_provider() {
        assert!(is_original_provider("xai/grok-code"));
        assert!(is_original_provider("anthropic/claude-3"));
        assert!(is_original_provider("openai/gpt-4"));
        assert!(is_original_provider("google/gemini"));
        assert!(is_original_provider("x-ai/grok"));
        assert!(!is_original_provider("azure_ai/grok"));
        assert!(!is_original_provider("bedrock/anthropic"));
        assert!(!is_original_provider("vertex_ai/gemini"));
        assert!(!is_original_provider("unknown-provider/model"));
    }

    #[test]
    fn test_is_reseller_provider() {
        assert!(is_reseller_provider("azure_ai/grok-code"));
        assert!(is_reseller_provider("azure/openai/gpt-4"));
        assert!(is_reseller_provider("bedrock/anthropic.claude"));
        assert!(is_reseller_provider("vertex_ai/gemini"));
        assert!(is_reseller_provider("together_ai/llama"));
        assert!(is_reseller_provider("groq/llama"));
        assert!(is_reseller_provider("orcarouter/openai/gpt-4"));
        assert!(!is_reseller_provider("xai/grok"));
        assert!(!is_reseller_provider("anthropic/claude"));
        assert!(!is_reseller_provider("openai/gpt-4"));
    }

    // =========================================================================
    // COST CALCULATION TESTS
    // =========================================================================

    #[test]
    fn test_calculate_cost_gpt_5_2() {
        let lookup = create_lookup();
        // 1M input, 500K output tokens
        let cost = lookup.calculate_cost("gpt-5.2", 1_000_000, 500_000, 0, 0, 0);
        // input: 1M * 0.00000175 = 1.75, output: 500K * 0.000014 = 7.0
        assert!((cost - 8.75).abs() < 0.001);
    }

    #[test]
    fn test_calculate_cost_claude_sonnet_4_5() {
        let lookup = create_lookup();
        // 100K input, 50K output, 200K cache read
        let cost = lookup.calculate_cost("claude-sonnet-4-5", 100_000, 50_000, 200_000, 0, 0);
        // input: 100K * 0.000003 = 0.30, output: 50K * 0.000015 = 0.75, cache: 200K * 3e-7 = 0.06
        assert!((cost - 1.11).abs() < 0.001);
    }

    #[test]
    fn test_compute_cost_tiered_boundary_at_200k_uses_base_rates() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token": 0.000001,
                "input_cost_per_token_above_200k_tokens": 0.000002,
                "output_cost_per_token": 0.000003,
                "output_cost_per_token_above_200k_tokens": 0.000004
            }"#,
        )
        .unwrap();

        let cost = compute_cost(&pricing, 200_000, 200_000, 0, 0, 0);
        let expected = 200_000.0 * 0.000001 + 200_000.0 * 0.000003;

        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_above_200k_splits_input_and_output() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token": 0.000001,
                "input_cost_per_token_above_200k_tokens": 0.000002,
                "output_cost_per_token": 0.000003,
                "output_cost_per_token_above_200k_tokens": 0.000004
            }"#,
        )
        .unwrap();

        let cost = compute_cost(&pricing, 200_001, 200_001, 0, 0, 0);
        let expected =
            (200_000.0 * 0.000001 + 1.0 * 0.000002) + (200_000.0 * 0.000003 + 1.0 * 0.000004);

        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_above_272k_splits_gpt_5_5_tokens() {
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

        let cost = compute_cost(&pricing, 272_001, 272_001, 272_001, 0, 0);
        let expected = (272_000.0 * 0.000005 + 1.0 * 0.000010)
            + (272_000.0 * 0.000030 + 1.0 * 0.000045)
            + (272_000.0 * 0.0000005 + 1.0 * 0.000001);

        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_uses_multiple_thresholds_in_order() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token": 0.000001,
                "input_cost_per_token_above_128k_tokens": 0.000002,
                "input_cost_per_token_above_256k_tokens": 0.000003,
                "input_cost_per_token_above_272k_tokens": 0.000004
            }"#,
        )
        .unwrap();

        let cost = compute_cost(&pricing, 300_000, 0, 0, 0, 0);
        let expected = (128_000.0 * 0.000001)
            + (128_000.0 * 0.000002)
            + (16_000.0 * 0.000003)
            + (28_000.0 * 0.000004);

        assert!((cost - expected).abs() < 1e-12);
    }

    fn openai_272k_result(key: &str, source: &str) -> LookupResult {
        LookupResult {
            matched_key: key.into(),
            source: source.into(),
            evidence: ResolutionEvidence::deterministic(ResolutionKind::Exact),
            pricing: ModelPricing {
                input_cost_per_token: Some(0.000005),
                input_cost_per_token_above_272k_tokens: Some(0.000010),
                output_cost_per_token: Some(0.000030),
                output_cost_per_token_above_272k_tokens: Some(0.000045),
                cache_read_input_token_cost: Some(0.0000005),
                cache_read_input_token_cost_above_272k_tokens: Some(0.000001),
                cache_creation_input_token_cost: Some(0.00000625),
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_openai_272k_full_request_pricing_uses_combined_input() {
        let result = openai_272k_result("openai/gpt-5.5", "LiteLLM");
        let usage = |input, output, cache_read, cache_write| TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            reasoning: 0,
        };
        let cost =
            compute_cost_for_lookup(&result, Some("openai"), &usage(200_000, 10_000, 72_000, 1));
        let expected = 200_000.0 * 0.000010 + 10_000.0 * 0.000045 + 72_000.0 * 0.000001 + 0.0000125;
        assert!((cost - expected).abs() < 1e-12);

        let boundary = compute_cost_for_lookup(&result, None, &usage(200_000, 10_000, 72_000, 0));
        let boundary_expected = 200_000.0 * 0.000005 + 10_000.0 * 0.000030 + 72_000.0 * 0.0000005;
        assert!((boundary - boundary_expected).abs() < 1e-12);

        let output_only = compute_cost_for_lookup(&result, None, &usage(1, 300_000, 0, 0));
        assert!((output_only - (0.000005 + 300_000.0 * 0.000030)).abs() < 1e-12);
    }

    #[test]
    fn test_provider_aware_openai_prefers_complete_litellm_tiers() {
        let litellm_pricing = openai_272k_result("gpt-5.6-sol", "LiteLLM").pricing;
        let openrouter_pricing = ModelPricing {
            input_cost_per_token: litellm_pricing.input_cost_per_token,
            output_cost_per_token: litellm_pricing.output_cost_per_token,
            cache_read_input_token_cost: litellm_pricing.cache_read_input_token_cost,
            ..Default::default()
        };
        let lookup = PricingLookup::new(
            HashMap::from([("gpt-5.6-sol".into(), litellm_pricing.clone())]),
            HashMap::from([("openai/gpt-5.6-sol".into(), openrouter_pricing)]),
            HashMap::new(),
        );

        let result = lookup
            .lookup_with_provider("gpt-5.6-sol", Some("openai"))
            .unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.matched_key, "gpt-5.6-sol");

        let usage = TokenBreakdown {
            input: 200_000,
            output: 10_000,
            cache_read: 72_001,
            ..Default::default()
        };
        let expected = 200_000.0 * 0.000010 + 10_000.0 * 0.000045 + 72_001.0 * 0.000001;
        for provider in [Some("openai"), Some("unknown"), Some(""), None] {
            let cost = lookup.calculate_cost_with_provider("gpt-5.6-sol", provider, &usage);
            assert!((cost - expected).abs() < 1e-12);
        }

        let lookup = PricingLookup::new(
            HashMap::from([("gpt-5.6-sol".into(), litellm_pricing.clone())]),
            HashMap::from([("openai/gpt-5.6-sol".into(), litellm_pricing)]),
            HashMap::new(),
        );
        let result = lookup
            .lookup_with_provider("gpt-5.6-sol", Some("openai"))
            .unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert!(!should_prefer_openai_tiered_litellm(
            "gpt-5.6-sol",
            Some("openrouter"),
            Some(&result)
        ));
    }

    #[test]
    fn test_openai_tiered_litellm_preference_requires_complete_272k_pricing() {
        let pricing = openai_272k_result("gpt-5.6-sol", "LiteLLM").pricing;
        assert!(has_complete_openai_272k_pricing(&pricing));

        let clear_required: [fn(&mut ModelPricing); 5] = [
            |pricing| pricing.input_cost_per_token = None,
            |pricing| pricing.input_cost_per_token_above_272k_tokens = None,
            |pricing| pricing.output_cost_per_token = None,
            |pricing| pricing.output_cost_per_token_above_272k_tokens = None,
            |pricing| pricing.cache_read_input_token_cost_above_272k_tokens = None,
        ];
        for clear in clear_required {
            let mut incomplete = pricing.clone();
            clear(&mut incomplete);
            assert!(!has_complete_openai_272k_pricing(&incomplete));
        }

        // A fully-absent cache_read pair is now incomplete too: this used to
        // pass leniently, letting the 272k preference silently drop an
        // OpenRouter entry's cache-read pricing (see
        // openai_272k_preference_prefers_openrouter_cache_read_pricing_over_incomplete_litellm).
        let mut without_cache_read = pricing;
        without_cache_read.cache_read_input_token_cost = None;
        without_cache_read.cache_read_input_token_cost_above_272k_tokens = None;
        assert!(!has_complete_openai_272k_pricing(&without_cache_read));
    }

    #[test]
    fn openai_272k_preference_prefers_openrouter_cache_read_pricing_over_incomplete_litellm() {
        let mut litellm_pricing = openai_272k_result("gpt-5.6-sol", "LiteLLM").pricing;
        litellm_pricing.cache_read_input_token_cost = None;
        litellm_pricing.cache_read_input_token_cost_above_272k_tokens = None;

        let openrouter_pricing = openai_272k_result("openai/gpt-5.6-sol", "OpenRouter").pricing;

        let lookup = PricingLookup::new(
            HashMap::from([("gpt-5.6-sol".into(), litellm_pricing)]),
            HashMap::from([("openai/gpt-5.6-sol".into(), openrouter_pricing)]),
            HashMap::new(),
        );

        let result = lookup
            .lookup_with_provider("gpt-5.6-sol", Some("openai"))
            .unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.matched_key, "openai/gpt-5.6-sol");
        assert!(result.pricing.cache_read_input_token_cost.is_some());
    }

    #[test]
    fn openai_272k_preference_still_prefers_complete_litellm_pricing() {
        let litellm_pricing = openai_272k_result("gpt-5.6-sol", "LiteLLM").pricing;
        let openrouter_pricing = openai_272k_result("openai/gpt-5.6-sol", "OpenRouter").pricing;

        let lookup = PricingLookup::new(
            HashMap::from([("gpt-5.6-sol".into(), litellm_pricing)]),
            HashMap::from([("openai/gpt-5.6-sol".into(), openrouter_pricing)]),
            HashMap::new(),
        );

        let result = lookup
            .lookup_with_provider("gpt-5.6-sol", Some("openai"))
            .unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.matched_key, "gpt-5.6-sol");
    }

    #[test]
    fn test_openai_272k_full_request_pricing_scope() {
        for key in [
            "gpt-5.4",
            "openai/gpt-5.4-pro-2026-03-05",
            "gpt-5.5-2026-04-23",
            "gpt-5.5-pro",
            "gpt-5.5-pro-2026-04-23",
            "gpt-5.6",
            "gpt-5.6-sol",
            "gpt-5.6-terra-2026-07-01",
            "gpt-5.6-luna",
        ] {
            assert!(
                uses_openai_full_request_272k_pricing(
                    &openai_272k_result(key, "LiteLLM"),
                    Some("openai")
                ),
                "expected full-request pricing for {key}"
            );
        }

        for key in [
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            "gpt-5.5-promax",
            "gpt-5.2",
            "fugu-ultra",
            "custom/gpt-5.5-pro",
        ] {
            assert!(
                !uses_openai_full_request_272k_pricing(
                    &openai_272k_result(key, "LiteLLM"),
                    Some("openai")
                ),
                "expected progressive pricing for {key}"
            );
        }

        for (result, provider) in [
            (openai_272k_result("fugu-ultra", "LiteLLM"), None),
            (openai_272k_result("openai/gpt-5.5", "OpenRouter"), None),
            (
                openai_272k_result("azure/openai/gpt-5.5", "LiteLLM"),
                Some("azure"),
            ),
        ] {
            assert!(!uses_openai_full_request_272k_pricing(&result, provider));
        }
    }

    #[test]
    fn orcarouter_hint_keeps_litellm_fallback_on_progressive_long_context_pricing() {
        // OrcaRouter can fall back to LiteLLM's unscoped OpenAI row when its
        // provider-specific catalog has no match. The provider hint, not an
        // invented OrcaRouter LiteLLM key, must keep that fallback on normal
        // progressive tiers instead of applying direct-OpenAI full-request
        // 272K semantics.
        let result = openai_272k_result("gpt-5.5", "LiteLLM");
        let usage = TokenBreakdown {
            input: 200_000,
            output: 10_000,
            cache_read: 72_001,
            ..Default::default()
        };

        assert!(uses_openai_full_request_272k_pricing(
            &result,
            Some("openai")
        ));
        assert!(!uses_openai_full_request_272k_pricing(
            &result,
            Some("orcarouter")
        ));

        let direct_openai_cost = compute_cost_for_lookup(&result, Some("openai"), &usage);
        let direct_openai_expected =
            (200_000.0 * 0.000010) + (10_000.0 * 0.000045) + (72_001.0 * 0.000001);
        assert!((direct_openai_cost - direct_openai_expected).abs() < 1e-12);

        let orcarouter_cost = compute_cost_for_lookup(&result, Some("orcarouter"), &usage);
        let orcarouter_expected =
            (200_000.0 * 0.000005) + (10_000.0 * 0.000030) + (72_001.0 * 0.0000005);
        assert!((orcarouter_cost - orcarouter_expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_is_applied_per_bucket() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token": 0.000001,
                "input_cost_per_token_above_200k_tokens": 0.000002,
                "output_cost_per_token": 0.000003,
                "output_cost_per_token_above_200k_tokens": 0.000004
            }"#,
        )
        .unwrap();

        let cost = compute_cost(&pricing, 200_001, 200_000, 0, 0, 0);
        let expected = (200_000.0 * 0.000001 + 1.0 * 0.000002) + (200_000.0 * 0.000003);

        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_missing_base_input_only_charges_above_threshold() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "input_cost_per_token_above_200k_tokens": 0.000002
            }"#,
        )
        .unwrap();

        let at_threshold = compute_cost(&pricing, 200_000, 0, 0, 0, 0);
        let above_threshold = compute_cost(&pricing, 200_001, 0, 0, 0, 0);

        assert_eq!(at_threshold, 0.0);
        assert!((above_threshold - 0.000002).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_cache_read_applies_split() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "cache_read_input_token_cost": 0.0000001,
                "cache_read_input_token_cost_above_200k_tokens": 0.0000002
            }"#,
        )
        .unwrap();

        let at_threshold = compute_cost(&pricing, 0, 0, 200_000, 0, 0);
        let above_threshold = compute_cost(&pricing, 0, 0, 200_001, 0, 0);

        assert!((at_threshold - (200_000.0 * 0.0000001)).abs() < 1e-12);
        assert!((above_threshold - (200_000.0 * 0.0000001 + 0.0000002)).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_cache_write_applies_split() {
        let pricing: ModelPricing = serde_json::from_str(
            r#"{
                "cache_creation_input_token_cost": 0.0000003,
                "cache_creation_input_token_cost_above_200k_tokens": 0.0000004
            }"#,
        )
        .unwrap();

        let at_threshold = compute_cost(&pricing, 0, 0, 0, 200_000, 0);
        let above_threshold = compute_cost(&pricing, 0, 0, 0, 200_001, 0);

        assert!((at_threshold - (200_000.0 * 0.0000003)).abs() < 1e-12);
        assert!((above_threshold - (200_000.0 * 0.0000003 + 0.0000004)).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_without_above_rate_uses_base_for_all_tokens() {
        let pricing = ModelPricing {
            input_cost_per_token: Some(0.000001),
            ..Default::default()
        };

        let cost = compute_cost(&pricing, 250_000, 0, 0, 0, 0);

        assert!((cost - (250_000.0 * 0.000001)).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_invalid_above_rate_falls_back_to_base() {
        let pricing_negative = ModelPricing {
            input_cost_per_token: Some(0.000001),
            input_cost_per_token_above_200k_tokens: Some(-0.000002),
            ..Default::default()
        };
        let pricing_infinite = ModelPricing {
            input_cost_per_token: Some(0.000001),
            input_cost_per_token_above_200k_tokens: Some(f64::INFINITY),
            ..Default::default()
        };
        let pricing_nan = ModelPricing {
            input_cost_per_token: Some(0.000001),
            input_cost_per_token_above_200k_tokens: Some(f64::NAN),
            ..Default::default()
        };

        let expected = 200_001.0 * 0.000001;
        assert!((compute_cost(&pricing_negative, 200_001, 0, 0, 0, 0) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_infinite, 200_001, 0, 0, 0, 0) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_nan, 200_001, 0, 0, 0, 0) - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_reasoning_boundary_at_200k_uses_base_output_rate() {
        let pricing = ModelPricing {
            output_cost_per_token: Some(0.000003),
            output_cost_per_token_above_200k_tokens: Some(0.000004),
            ..Default::default()
        };

        let cost = compute_cost(&pricing, 0, 199_999, 0, 0, 1);
        let expected = 200_000.0 * 0.000003;

        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_invalid_above_rate_falls_back_to_base_output_reasoning() {
        let pricing_negative = ModelPricing {
            output_cost_per_token: Some(0.000003),
            output_cost_per_token_above_200k_tokens: Some(-0.000004),
            ..Default::default()
        };
        let pricing_infinite = ModelPricing {
            output_cost_per_token: Some(0.000003),
            output_cost_per_token_above_200k_tokens: Some(f64::INFINITY),
            ..Default::default()
        };
        let pricing_nan = ModelPricing {
            output_cost_per_token: Some(0.000003),
            output_cost_per_token_above_200k_tokens: Some(f64::NAN),
            ..Default::default()
        };

        let expected = 200_001.0 * 0.000003;
        assert!((compute_cost(&pricing_negative, 0, 199_999, 0, 0, 2) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_infinite, 0, 199_999, 0, 0, 2) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_nan, 0, 199_999, 0, 0, 2) - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_invalid_above_rate_falls_back_to_base_cache_read() {
        let pricing_negative = ModelPricing {
            cache_read_input_token_cost: Some(0.0000001),
            cache_read_input_token_cost_above_200k_tokens: Some(-0.0000002),
            ..Default::default()
        };
        let pricing_infinite = ModelPricing {
            cache_read_input_token_cost: Some(0.0000001),
            cache_read_input_token_cost_above_200k_tokens: Some(f64::INFINITY),
            ..Default::default()
        };
        let pricing_nan = ModelPricing {
            cache_read_input_token_cost: Some(0.0000001),
            cache_read_input_token_cost_above_200k_tokens: Some(f64::NAN),
            ..Default::default()
        };

        let expected = 200_001.0 * 0.0000001;
        assert!((compute_cost(&pricing_negative, 0, 0, 200_001, 0, 0) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_infinite, 0, 0, 200_001, 0, 0) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_nan, 0, 0, 200_001, 0, 0) - expected).abs() < 1e-12);
    }

    #[test]
    fn test_compute_cost_tiered_invalid_above_rate_falls_back_to_base_cache_write() {
        let pricing_negative = ModelPricing {
            cache_creation_input_token_cost: Some(0.0000003),
            cache_creation_input_token_cost_above_200k_tokens: Some(-0.0000004),
            ..Default::default()
        };
        let pricing_infinite = ModelPricing {
            cache_creation_input_token_cost: Some(0.0000003),
            cache_creation_input_token_cost_above_200k_tokens: Some(f64::INFINITY),
            ..Default::default()
        };
        let pricing_nan = ModelPricing {
            cache_creation_input_token_cost: Some(0.0000003),
            cache_creation_input_token_cost_above_200k_tokens: Some(f64::NAN),
            ..Default::default()
        };

        let expected = 200_001.0 * 0.0000003;
        assert!((compute_cost(&pricing_negative, 0, 0, 0, 200_001, 0) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_infinite, 0, 0, 0, 200_001, 0) - expected).abs() < 1e-12);
        assert!((compute_cost(&pricing_nan, 0, 0, 0, 200_001, 0) - expected).abs() < 1e-12);
    }

    #[test]
    fn test_provider_prefixed_non_opus_prefers_exact_openrouter_without_tier_advantage() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000123),
                output_cost_per_token: Some(0.0000456),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-sonnet-4").unwrap();
        assert_eq!(resolved.source, "OpenRouter");
        assert_eq!(resolved.matched_key, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_provider_prefixed_exact_litellm_beats_stripped_generic_match() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                ..Default::default()
            },
        );
        litellm.insert(
            "openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let resolved = lookup.lookup("openai/gpt-4").unwrap();
        assert_eq!(resolved.source, "LiteLLM");
        assert_eq!(resolved.matched_key, "openai/gpt-4");
    }

    #[test]
    fn test_provider_prefixed_override_requires_valid_base_and_above_pair() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4".into(),
            ModelPricing {
                // Above tier exists, but corresponding base is missing.
                // This must not qualify for provider-prefixed override.
                input_cost_per_token: None,
                input_cost_per_token_above_200k_tokens: Some(0.00002),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000123),
                output_cost_per_token: Some(0.0000456),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-sonnet-4").unwrap();
        assert_eq!(resolved.source, "OpenRouter");
        assert_eq!(resolved.matched_key, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_provider_prefixed_override_rejects_invalid_base_even_with_above() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(f64::NAN),
                input_cost_per_token_above_200k_tokens: Some(0.00002),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000123),
                output_cost_per_token: Some(0.0000456),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-sonnet-4").unwrap();
        assert_eq!(resolved.source, "OpenRouter");
        assert_eq!(resolved.matched_key, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_provider_prefixed_override_allows_zero_base_with_valid_above() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4".into(),
            ModelPricing {
                // Policy: base=0 with valid above is a valid tier pair.
                input_cost_per_token: Some(0.0),
                input_cost_per_token_above_200k_tokens: Some(0.00002),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000123),
                output_cost_per_token: Some(0.0000456),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-sonnet-4").unwrap();
        assert_eq!(resolved.source, "LiteLLM");
        assert_eq!(resolved.matched_key, "claude-sonnet-4");
    }

    #[test]
    fn test_provider_prefixed_cache_only_tier_keeps_exact_openrouter() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4".into(),
            ModelPricing {
                cache_read_input_token_cost: Some(0.0000001),
                cache_read_input_token_cost_above_200k_tokens: Some(0.0000002),
                cache_creation_input_token_cost: Some(0.0000003),
                cache_creation_input_token_cost_above_200k_tokens: Some(0.0000004),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000123),
                output_cost_per_token: Some(0.0000456),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-sonnet-4").unwrap();
        assert_eq!(resolved.source, "OpenRouter");
        assert_eq!(resolved.matched_key, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_provider_prefixed_opus_4_6_prefers_litellm_tiered_pricing() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                input_cost_per_token_above_200k_tokens: Some(0.00002),
                output_cost_per_token: Some(0.00005),
                output_cost_per_token_above_200k_tokens: Some(0.00006),
                cache_read_input_token_cost: Some(0.000001),
                cache_read_input_token_cost_above_200k_tokens: Some(0.000002),
                cache_creation_input_token_cost: Some(0.000003),
                cache_creation_input_token_cost_above_200k_tokens: Some(0.000004),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.123),
                output_cost_per_token: Some(0.456),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-opus-4-6").unwrap();
        assert_eq!(resolved.source, "LiteLLM");
        assert_eq!(resolved.matched_key, "claude-opus-4-6");

        let cost = lookup.calculate_cost("anthropic/claude-opus-4-6", 200_001, 0, 0, 0, 0);
        let expected = 200_000.0 * 0.00001 + 0.00002;
        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_anthropic_prefixed_sonnet_variant_uses_canonical_pricing() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: Some(0.0000003),
                cache_creation_input_token_cost: Some(0.00000375),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-4-6-sonnet").unwrap();
        assert_eq!(resolved.source, "LiteLLM");
        assert_eq!(resolved.matched_key, "claude-sonnet-4-6");

        let cost = lookup.calculate_cost("anthropic/claude-4-6-sonnet", 100, 20, 10, 5, 0);
        let expected = 100.0 * 0.000003 + 20.0 * 0.000015 + 10.0 * 0.0000003 + 5.0 * 0.00000375;
        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_anthropic_prefixed_haiku_variant_uses_canonical_pricing() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-haiku-4-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0000008),
                output_cost_per_token: Some(0.000004),
                cache_read_input_token_cost: Some(0.00000008),
                cache_creation_input_token_cost: Some(0.000001),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let resolved = lookup.lookup("anthropic/claude-4-5-haiku").unwrap();
        assert_eq!(resolved.source, "LiteLLM");
        assert_eq!(resolved.matched_key, "claude-haiku-4-5");

        let cost = lookup.calculate_cost("anthropic/claude-4-5-haiku", 100, 20, 10, 5, 0);
        let expected = 100.0 * 0.0000008 + 20.0 * 0.000004 + 10.0 * 0.00000008 + 5.0 * 0.000001;
        assert!((cost - expected).abs() < 1e-12);
    }

    /// Regression test for #336: subscription-based resellers (e.g. Perplexity) with
    /// all-None pricing should not shadow valid entries during provider-aware lookup.
    /// `perplexity/anthropic/claude-opus-4-6` matches provider hint "anthropic" via
    /// its path segments, but has no per-token pricing. The lookup must fall through
    /// to the exact `claude-opus-4-6` entry that has real pricing data.
    #[test]
    fn test_none_pricing_reseller_does_not_shadow_real_entry() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                cache_read_input_token_cost: Some(0.0000005),
                cache_creation_input_token_cost: Some(0.00000625),
                ..Default::default()
            },
        );
        // Perplexity entry: matches "anthropic" hint but has no pricing
        litellm.insert(
            "perplexity/anthropic/claude-opus-4-6".into(),
            ModelPricing::default(),
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        // With provider hint "anthropic", should find the real entry, not perplexity
        let result = lookup.lookup_with_provider("claude-opus-4-6", Some("anthropic"));
        assert!(result.is_some(), "lookup should succeed");
        let result = result.unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-6");
        assert!(result.pricing.input_cost_per_token.is_some());

        // Cost should be non-zero
        let cost = lookup.calculate_cost("claude-opus-4-6", 100_000, 50_000, 0, 0, 0);
        assert!(cost > 0.0, "cost should be positive, got {}", cost);
    }

    #[test]
    fn test_none_pricing_provider_match_falls_back_to_priced_fuzzy_candidate() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4-6-20250301".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                ..Default::default()
            },
        );
        litellm.insert(
            "perplexity/anthropic/claude-opus-4-6-20250301".into(),
            ModelPricing::default(),
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let result = lookup.lookup_with_provider("claude-opus-4-6-latest", Some("anthropic"));
        assert!(result.is_some(), "lookup should succeed via fuzzy fallback");
        let result = result.unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-6-20250301");
        assert_eq!(result.source, "LiteLLM");
        assert!(result.pricing.input_cost_per_token.is_some());
    }

    #[test]
    fn test_none_pricing_exact_litellm_does_not_shadow_openrouter_model_part() {
        let mut litellm = HashMap::new();
        litellm.insert("claude-opus-4-6".into(), ModelPricing::default());

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000025),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());
        let result = lookup.lookup("claude-opus-4-6").unwrap();

        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.matched_key, "anthropic/claude-opus-4-6");

        let cost = lookup.calculate_cost("claude-opus-4-6", 100, 20, 0, 0, 0);
        assert!(cost > 0.0, "cost should use priced fallback, got {cost}");
    }

    #[test]
    fn test_none_pricing_provider_exact_does_not_shadow_stripped_priced_entry() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "anthropic/claude-sonnet-4-5".into(),
            ModelPricing::default(),
        );
        litellm.insert(
            "claude-sonnet-4-5".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup("anthropic/claude-sonnet-4-5").unwrap();

        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.matched_key, "claude-sonnet-4-5");

        let cost = lookup.calculate_cost("anthropic/claude-sonnet-4-5", 100, 20, 0, 0, 0);
        assert!(
            cost > 0.0,
            "cost should use stripped priced entry, got {cost}"
        );
    }

    #[test]
    fn test_zero_pricing_exact_entry_is_usable() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "free-model".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup.lookup("free-model").unwrap();

        assert_eq!(result.matched_key, "free-model");
        assert_eq!(lookup.calculate_cost("free-model", 100, 20, 0, 0, 0), 0.0);
    }

    #[test]
    fn test_calculate_cost_tiered_all_buckets_with_reasoning_threshold_crossing() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000001),
                input_cost_per_token_above_200k_tokens: Some(0.000002),
                output_cost_per_token: Some(0.000003),
                output_cost_per_token_above_200k_tokens: Some(0.000004),
                cache_read_input_token_cost: Some(0.0000001),
                cache_read_input_token_cost_above_200k_tokens: Some(0.0000002),
                cache_creation_input_token_cost: Some(0.0000003),
                cache_creation_input_token_cost_above_200k_tokens: Some(0.0000004),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let cost = lookup.calculate_cost("claude-opus-4-6", 200_001, 199_999, 200_001, 200_001, 2);

        let expected_input = 200_000.0 * 0.000001 + 0.000002;
        let expected_output = 200_000.0 * 0.000003 + 0.000004; // output + reasoning = 200_001
        let expected_cache_read = 200_000.0 * 0.0000001 + 0.0000002;
        let expected_cache_write = 200_000.0 * 0.0000003 + 0.0000004;
        let expected =
            expected_input + expected_output + expected_cache_read + expected_cache_write;

        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_calculate_cost_unknown_model() {
        let lookup = create_lookup();
        let cost = lookup.calculate_cost("nonexistent-model", 1_000_000, 500_000, 0, 0, 0);
        assert_eq!(cost, 0.0);
    }

    // =========================================================================
    // INTELLIGENT PREFIX/SUFFIX STRIPPING TESTS
    // =========================================================================

    #[test]
    fn test_antigravity_prefix_gemini_3_flash() {
        let lookup = create_lookup();
        let result = lookup.lookup("antigravity-gemini-3-flash").unwrap();
        assert_eq!(result.matched_key, "vertex_ai/gemini-3-flash-preview");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_antigravity_prefix_gemini_3_pro() {
        let lookup = create_lookup();
        let result = lookup.lookup("antigravity-gemini-3-pro").unwrap();
        assert_eq!(result.matched_key, "openrouter/google/gemini-3-pro-preview");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_antigravity_prefix_with_tier_suffix() {
        let lookup = create_lookup();
        let result = lookup.lookup("antigravity-gemini-3-pro-high").unwrap();
        assert_eq!(result.matched_key, "openrouter/google/gemini-3-pro-preview");
    }

    #[test]
    fn test_antigravity_prefix_claude() {
        let lookup = create_lookup();
        let result = lookup.lookup("antigravity-claude-sonnet-4-5").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_antigravity_prefix_gpt() {
        let lookup = create_lookup();
        let result = lookup.lookup("antigravity-gpt-4o").unwrap();
        assert_eq!(result.matched_key, "gpt-4o");
        assert_eq!(result.source, "LiteLLM");
    }

    #[test]
    fn test_antigravity_prefix_case_insensitive() {
        let lookup = create_lookup();
        let result = lookup.lookup("Antigravity-gpt-4o").unwrap();
        assert_eq!(result.matched_key, "gpt-4o");
    }

    #[test]
    fn test_antigravity_cost_calculation() {
        let lookup = create_lookup();
        let cost_with_prefix =
            lookup.calculate_cost("antigravity-gpt-5.2", 1_000_000, 500_000, 0, 0, 0);
        let cost_without_prefix = lookup.calculate_cost("gpt-5.2", 1_000_000, 500_000, 0, 0, 0);
        assert!((cost_with_prefix - cost_without_prefix).abs() < 0.001);
        assert!(cost_with_prefix > 0.0);
    }

    // New tests for intelligent detection

    #[test]
    fn test_unknown_prefix_generic() {
        let lookup = create_lookup();
        let result = lookup.lookup("myplugin-gpt-4o").unwrap();
        assert_eq!(result.matched_key, "gpt-4o");
    }

    #[test]
    fn test_unknown_prefix_two_segments() {
        let lookup = create_lookup();
        let result = lookup.lookup("router-v2-claude-sonnet-4-5").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
    }

    #[test]
    fn test_unknown_suffix_thinking() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-sonnet-4-5-thinking").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
    }

    #[test]
    fn test_unknown_suffix_two_segments() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-opus-4-5-thinking-pro").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-5");
    }

    #[test]
    fn test_prefix_and_suffix_combined() {
        let lookup = create_lookup();
        let result = lookup
            .lookup("antigravity-claude-opus-4-5-thinking")
            .unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-5");
    }

    #[test]
    fn test_prefix_and_suffix_with_tier() {
        let lookup = create_lookup();
        let result = lookup
            .lookup("antigravity-claude-opus-4-5-thinking-high")
            .unwrap();
        assert_eq!(result.matched_key, "claude-opus-4-5");
    }

    #[test]
    fn test_no_false_positive_valid_model() {
        let lookup = create_lookup();
        // gpt-4o-mini is a valid model, should NOT strip "gpt"
        let result = lookup.lookup("gpt-4o-mini").unwrap();
        assert_eq!(result.matched_key, "gpt-4o-mini");
    }

    #[test]
    fn test_suffix_strip_high() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-sonnet-4-5-high").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
    }

    #[test]
    fn test_suffix_strip_xhigh() {
        let lookup = create_lookup();
        let result = lookup.lookup("claude-sonnet-4-5-xhigh").unwrap();
        assert_eq!(result.matched_key, "claude-sonnet-4-5");
    }

    #[test]
    fn test_suffix_strip_low() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-4o-low").unwrap();
        assert_eq!(result.matched_key, "gpt-4o");
    }

    #[test]
    fn test_suffix_strip_codex() {
        let lookup = create_lookup();
        let result = lookup.lookup("gpt-5.2-codex").unwrap();
        assert_eq!(result.matched_key, "gpt-5.2");
    }

    #[test]
    fn test_provider_hint_empty_and_unknown_treated_as_none() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                ..Default::default()
            },
        );
        litellm.insert(
            "azure_ai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let r_none = lookup.lookup_with_provider("gpt-4", None).unwrap();
        let r_empty = lookup.lookup_with_provider("gpt-4", Some("")).unwrap();
        let r_unknown = lookup
            .lookup_with_provider("gpt-4", Some("unknown"))
            .unwrap();

        assert_eq!(r_none.matched_key, r_empty.matched_key);
        assert_eq!(r_none.matched_key, r_unknown.matched_key);
    }

    #[test]
    fn test_provider_hint_mistralai_matches_mistral_keys() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "mistralai/mistral-large".into(),
            ModelPricing {
                input_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup
            .lookup_with_provider("mistral-large", Some("mistral"))
            .unwrap();
        assert_eq!(result.matched_key, "mistralai/mistral-large");
    }

    #[test]
    fn test_provider_hint_minimax_matches_minimax_keys() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "minimax/minimax-m2.1".into(),
            ModelPricing {
                input_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let result = lookup
            .lookup_with_provider("MiniMax-M2.1", Some("minimax"))
            .unwrap();
        assert_eq!(result.matched_key, "minimax/minimax-m2.1");
    }

    #[test]
    fn test_prefixed_model_with_conflicting_provider_uses_provider_aware_path() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                ..Default::default()
            },
        );
        litellm.insert(
            "azure/openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let r_azure = lookup
            .lookup_with_provider("openai/gpt-4", Some("azure"))
            .unwrap();
        assert_eq!(
            r_azure.matched_key, "azure/openai/gpt-4",
            "should prefer azure key when provider_id=azure"
        );

        let r_openai = lookup
            .lookup_with_provider("openai/gpt-4", Some("openai"))
            .unwrap();
        assert_eq!(
            r_openai.matched_key, "openai/gpt-4",
            "should use exact prefixed key when provider_id matches prefix"
        );

        let r_none = lookup.lookup_with_provider("openai/gpt-4", None).unwrap();
        assert_eq!(
            r_none.matched_key, "openai/gpt-4",
            "should use exact prefixed key when no provider hint"
        );
    }

    #[test]
    fn test_prefixed_model_conflicting_provider_falls_back_to_stripped() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                ..Default::default()
            },
        );
        litellm.insert(
            "gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let r = lookup
            .lookup_with_provider("openai/gpt-4", Some("azure"))
            .unwrap();
        assert_eq!(
            r.matched_key, "gpt-4",
            "with no azure-specific key, should fall back to stripped generic"
        );
    }

    #[test]
    fn test_compound_provider_hint_prefers_reseller_over_prefix() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                ..Default::default()
            },
        );
        litellm.insert(
            "azure/openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());
        let r = lookup
            .lookup_with_provider("openai/gpt-4", Some("azure/openai"))
            .unwrap();
        assert_eq!(
            r.matched_key, "azure/openai/gpt-4",
            "compound hint azure/openai should prefer azure-specific key over openai/ prefix"
        );
    }

    #[test]
    fn test_source_and_provider_normalizes_unknown_hint() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                ..Default::default()
            },
        );

        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let r_unknown = lookup
            .lookup_with_source_and_provider("openai/gpt-4", None, Some("unknown"))
            .unwrap();
        let r_none = lookup
            .lookup_with_source_and_provider("openai/gpt-4", None, None)
            .unwrap();
        assert_eq!(
            r_unknown.matched_key, r_none.matched_key,
            "unknown hint via source_and_provider should behave like None"
        );
    }

    /// Regression (#1092): equal-length candidate keys must be ordered by the
    /// index, not by `HashMap` iteration order. This exercises the ordered
    /// candidate list consumed by `select_best_match` — two litellm keys of the
    /// same length, no models.dev entries, so the only thing that can decide the
    /// winner is the tiebreak in the key sort. Without it the lookup returns
    /// whichever key the hasher happened to yield first, and the reported rate
    /// flips between $0.01 and $0.02 across processes.
    #[test]
    fn test_pricing_index_deterministic_key_sorting_equal_length() {
        let build = |first: (&str, f64), second: (&str, f64)| {
            let mut litellm = HashMap::new();
            for (key, input_cost) in [first, second] {
                litellm.insert(
                    key.to_string(),
                    ModelPricing {
                        input_cost_per_token: Some(input_cost),
                        ..Default::default()
                    },
                );
            }
            PricingLookup::new_with_models_dev(
                litellm,
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
            )
        };

        let east = ("bedrock/us-east-1/zai.glm-5", 0.01);
        let west = ("bedrock/us-west-2/zai.glm-5", 0.02);
        assert_eq!(east.0.len(), west.0.len());

        for (order, index) in [build(east, west), build(west, east)].iter().enumerate() {
            let result = index
                .lookup_with_provider("zai.glm-5", Some("bedrock"))
                .unwrap_or_else(|| panic!("insertion order {order} must resolve zai.glm-5"));
            assert_eq!(
                result.matched_key, "bedrock/us-east-1/zai.glm-5",
                "equal-length candidates must resolve to the alphabetically first key regardless of insertion order (order {order})"
            );
            assert_eq!(result.pricing.input_cost_per_token, Some(0.01));
        }
    }

    /// Regression (#1062): a bare router label must not be priced from a
    /// coincidence of spelling. `auto` used to elect `morph/auto` at
    /// $0.85/$1.55 — an unrelated code-apply vendor — and submit at those
    /// rates, because covers_usage only demands rates for populated buckets.
    #[test]
    fn bare_routing_labels_do_not_resolve_but_qualified_ones_do() {
        let mut models_dev = HashMap::new();
        for key in ["morph/auto", "llmgateway/auto", "cursor/agent_review"] {
            models_dev.insert(
                key.to_string(),
                ModelPricing {
                    input_cost_per_token: Some(8.5e-7),
                    output_cost_per_token: Some(1.55e-6),
                    ..Default::default()
                },
            );
        }
        let lookup = PricingLookup::new_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        // Five parsers emit these bare; nothing records the real model.
        assert!(lookup.lookup("auto").is_none());
        assert!(lookup.lookup("AUTO").is_none());
        assert!(lookup.lookup("agent_review").is_none());

        // A tier suffix does not make it a model: this normalizes to `auto`
        // before the model-part fallback runs.
        assert!(lookup.lookup("auto(high)").is_none());
        // Nor does an unrecognized vendor prefix, which is dropped to retry
        // the bare id. A real `morph/auto` never reaches that fallback.
        assert!(lookup.lookup("cx/auto").is_none());

        // A qualified id names a real vendor's model and still prices.
        assert!(lookup.lookup("morph/auto").is_some());
    }

    /// The shortest-key tie-break is a coin flip. Preferring the original
    /// provider generalizes the `anthropic/` special case it replaced, so a
    /// reseller no longer wins on key length alone.
    #[test]
    fn model_part_tie_break_prefers_the_original_provider_over_a_shorter_key() {
        assert!(super::prefers_model_part_key(
            "openai/some-model",
            "xy/some-model"
        ));
        assert!(!super::prefers_model_part_key(
            "xy/some-model",
            "openai/some-model"
        ));
        // Neither is an original provider: length still decides.
        assert!(super::prefers_model_part_key(
            "ab/some-model",
            "abcd/some-model"
        ));
    }

    /// Folding `deepseek-ai` into `deepseek` widens the provider-hint
    /// candidate pool, and `deepseek` is exactly the hint
    /// `inferred_provider_from_model` synthesizes for every model named
    /// `deepseek-*` whose client reports no provider. Both rows below then
    /// match the hint, they disagree by 16x on output, and nothing else in
    /// `select_best_match` tells them apart — the winner would fall out of key
    /// ordering, which is length-descending over a HashMap's key iteration and
    /// so not stable between processes for equal-length keys. The row spelling
    /// the vendor the way the hint does has to win.
    #[test]
    fn vendor_spelling_fold_does_not_move_pricing_onto_another_reseller() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "novita/deepseek/deepseek-r1-distill-qwen-32b".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0000003),
                output_cost_per_token: Some(0.0000003),
                ..Default::default()
            },
        );
        litellm.insert(
            "cloudflare/@cf/deepseek-ai/deepseek-r1-distill-qwen-32b".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000000497),
                output_cost_per_token: Some(0.000004881),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let result = lookup
            .lookup_with_provider("deepseek-r1-distill-qwen-32b", Some("deepseek"))
            .expect("deepseek-hinted distill must price");
        assert_eq!(
            result.matched_key, "novita/deepseek/deepseek-r1-distill-qwen-32b",
            "a `deepseek` hint must not cross onto the `deepseek-ai`-spelled reseller row"
        );
        assert_eq!(result.pricing.output_cost_per_token, Some(0.0000003));
    }

    /// The other direction of the same fold, which the spelling preference must
    /// not break: a `deepseek-ai` hint exists so it can reach rows spelled
    /// `deepseek`, and DeepSeek's own first-party row is the whole point. A
    /// reseller row that happens to spell the vendor the hint's way must not
    /// displace it.
    #[test]
    fn vendor_spelling_preference_never_displaces_the_first_party_row() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "deepseek/deepseek-v3".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.00000027),
                output_cost_per_token: Some(0.0000011),
                ..Default::default()
            },
        );
        litellm.insert(
            "hyperbolic/deepseek-ai/DeepSeek-V3".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0000002),
                output_cost_per_token: Some(0.0000002),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        for hint in ["deepseek-ai", "deepseek_ai", "DeepSeek-AI", "deepseek"] {
            let result = lookup
                .lookup_with_provider("deepseek-v3", Some(hint))
                .unwrap_or_else(|| panic!("{hint}-hinted deepseek-v3 must price"));
            assert_eq!(
                result.matched_key, "deepseek/deepseek-v3",
                "{hint} must still reach DeepSeek's own row"
            );
        }
    }

    /// The spelling preference is a tiebreak among rows that merely nest the
    /// vendor, so it must yield to the hinted provider's own top-level row.
    /// `poe/novita/kimi-k2.6` spells `novita` only because Poe is reselling
    /// Novita's endpoint, and it charges $0.96/$4.04 per MTok against Novita's
    /// own $0.80/$3.40.
    #[test]
    fn hinted_provider_own_row_outranks_a_nested_spelling_match() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "novita-ai/moonshotai/kimi-k2.6".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0000008),
                output_cost_per_token: Some(0.0000034),
                ..Default::default()
            },
        );
        litellm.insert(
            "poe/novita/kimi-k2.6".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.00000096),
                output_cost_per_token: Some(0.00000404),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let result = lookup
            .lookup_with_provider("kimi-k2.6", Some("novita"))
            .expect("novita-hinted kimi-k2.6 must price");
        assert_eq!(
            result.matched_key, "novita-ai/moonshotai/kimi-k2.6",
            "Novita's own row must win over Poe reselling it"
        );
    }

    /// Kimi, Warp, Kiro, Codebuff and Tencent Buddy report the literal string
    /// `unknown` when they cannot name a provider, and `normalize_provider_hint`
    /// drops it, so the unhinted path is reached in production. It has no
    /// vendor spelling to prefer and must resolve exactly as a missing hint
    /// does.
    #[test]
    fn unhinted_lookup_is_unchanged_by_the_vendor_spelling_preference() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "novita/deepseek/deepseek-r1-distill-qwen-32b".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0000003),
                output_cost_per_token: Some(0.0000003),
                ..Default::default()
            },
        );
        litellm.insert(
            "cloudflare/@cf/deepseek-ai/deepseek-r1-distill-qwen-32b".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000000497),
                output_cost_per_token: Some(0.000004881),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let bare = lookup.lookup_with_provider("deepseek-r1-distill-qwen-32b", None);
        for hint in [Some("unknown"), Some("UNKNOWN"), Some(""), Some("  ")] {
            let hinted = lookup.lookup_with_provider("deepseek-r1-distill-qwen-32b", hint);
            assert_eq!(
                hinted.map(|r| r.matched_key),
                bare.as_ref().map(|r| r.matched_key.clone()),
                "{hint:?} is dropped by normalize_provider_hint and must match the unhinted result"
            );
        }
    }

    /// `key_root_matches_hint` recognises the hinted vendor's own top-level
    /// row, and that row has to be *selected*, not merely used to switch the
    /// spelling preference off. Z.ai publishes `zai/glm-4.6` at $0.60/$2.20 per
    /// MTok and Vercel's gateway resells it at $0.45/$1.80 under
    /// `vercel_ai_gateway/zai/glm-4.6`; neither key is in
    /// `ORIGINAL_PROVIDER_PREFIXES` (Z.ai's first-party spelling there is
    /// `z-ai/`) nor in `RESELLER_PROVIDER_PREFIXES`, and candidates are ordered
    /// longest key first, so a `zai` hint must not be billed at the gateway's
    /// sheet just because its key is longer.
    #[test]
    fn hinted_vendor_own_row_wins_over_a_longer_row_that_only_nests_the_vendor() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "zai/glm-4.6".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.0000006),
                output_cost_per_token: Some(0.0000022),
                ..Default::default()
            },
        );
        litellm.insert(
            "vercel_ai_gateway/zai/glm-4.6".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.00000045),
                output_cost_per_token: Some(0.0000018),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let result = lookup
            .lookup_with_provider("glm-4.6", Some("zai"))
            .expect("zai-hinted glm-4.6 must price");
        assert_eq!(
            result.matched_key, "zai/glm-4.6",
            "Z.ai's own row must win over a gateway that nests `zai` in a longer key"
        );
        assert_eq!(result.pricing.output_cost_per_token, Some(0.0000022));
    }

    /// The spelling preference exists to keep a hinted vendor on the row that
    /// spells the vendor its way, so it must not throw that row away for
    /// starting with a reseller prefix. Before the fold, a `deepseek-ai` hint
    /// matched only `together_ai/deepseek-ai/DeepSeek-R1` ($3.00/$7.00 per
    /// MTok); folding `deepseek-ai` into `deepseek` pulled
    /// `vercel_ai_gateway/deepseek/deepseek-r1` ($0.55/$2.19) into the same
    /// pool, and it wins on key length alone. That is the fold moving usage
    /// between two resellers, which is precisely what the preference is for.
    #[test]
    fn exact_vendor_spelling_wins_even_when_that_row_is_a_reseller() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "together_ai/deepseek-ai/DeepSeek-R1".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000007),
                ..Default::default()
            },
        );
        litellm.insert(
            "vercel_ai_gateway/deepseek/deepseek-r1".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.00000055),
                output_cost_per_token: Some(0.00000219),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let result = lookup
            .lookup_with_provider("deepseek-r1", Some("deepseek-ai"))
            .expect("deepseek-ai-hinted deepseek-r1 must price");
        assert_eq!(
            result.matched_key, "together_ai/deepseek-ai/DeepSeek-R1",
            "the row spelling the vendor the hint's way must win even though it is a reseller"
        );
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000007));
    }

    /// Vertex canonicalizes to Anthropic so an Anthropic hint can find Vertex's
    /// hosted Claude rows. That alias must not make the hosting platform's root
    /// look like Anthropic's own top-level row and outrank an exact-spelling row.
    #[test]
    fn reseller_alias_root_does_not_outrank_exact_vendor_spelling() {
        for vertex_root in ["vertex", "vertex_ai"] {
            let hosted_key = format!("{vertex_root}/claude-sonnet-4");
            let mut litellm = HashMap::new();
            litellm.insert(
                hosted_key.clone(),
                ModelPricing {
                    input_cost_per_token: Some(0.000003),
                    output_cost_per_token: Some(0.000015),
                    ..Default::default()
                },
            );
            litellm.insert(
                "host/anthropic/claude-sonnet-4".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(0.000004),
                    output_cost_per_token: Some(0.000020),
                    ..Default::default()
                },
            );
            let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

            let anthropic = lookup
                .lookup_with_provider("claude-sonnet-4", Some("anthropic"))
                .expect("anthropic-hinted claude-sonnet-4 must price");
            assert_eq!(
                anthropic.matched_key, "host/anthropic/claude-sonnet-4",
                "{vertex_root} must not impersonate Anthropic's own root"
            );

            let vertex = lookup
                .lookup_with_provider("claude-sonnet-4", Some(vertex_root))
                .unwrap_or_else(|| panic!("{vertex_root}-hinted claude-sonnet-4 must price"));
            assert_eq!(
                vertex.matched_key, hosted_key,
                "a Vertex hint must still select Vertex's hosted row"
            );
        }
    }

    /// Direct Vertex hints must keep Vertex's hosted pricing even when an
    /// Anthropic first-party row is also available. The canonical provider tag
    /// makes both candidates reachable; the raw hint decides which root owns
    /// the usage.
    #[test]
    fn direct_vertex_hint_outranks_anthropic_first_party_alias() {
        for vertex_root in ["vertex", "vertex_ai"] {
            let hosted_key = format!("{vertex_root}/claude-sonnet-4");
            let mut litellm = HashMap::new();
            litellm.insert(
                hosted_key.clone(),
                ModelPricing {
                    input_cost_per_token: Some(0.000003),
                    output_cost_per_token: Some(0.000015),
                    ..Default::default()
                },
            );
            litellm.insert(
                "anthropic/claude-sonnet-4".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(0.000004),
                    output_cost_per_token: Some(0.000020),
                    ..Default::default()
                },
            );
            let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

            let vertex = lookup
                .lookup_with_provider("claude-sonnet-4", Some(vertex_root))
                .unwrap_or_else(|| panic!("{vertex_root}-hinted claude-sonnet-4 must price"));
            assert_eq!(vertex.matched_key, hosted_key);

            let anthropic = lookup
                .lookup_with_provider("claude-sonnet-4", Some("anthropic"))
                .expect("anthropic-hinted claude-sonnet-4 must price");
            assert_eq!(anthropic.matched_key, "anthropic/claude-sonnet-4");
        }
    }

    /// The same explicit-root preference must survive source arbitration;
    /// otherwise each dataset selects correctly and the later cross-source
    /// first-party tier silently changes the winner back to Anthropic.
    #[test]
    fn direct_vertex_hint_outranks_cross_source_anthropic_alias() {
        for vertex_root in ["vertex", "vertex_ai"] {
            let hosted_key = format!("{vertex_root}/claude-sonnet-4");
            let mut litellm = HashMap::new();
            litellm.insert(
                hosted_key.clone(),
                ModelPricing {
                    input_cost_per_token: Some(0.000003),
                    output_cost_per_token: Some(0.000015),
                    ..Default::default()
                },
            );
            let mut openrouter = HashMap::new();
            openrouter.insert(
                "anthropic/claude-sonnet-4".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(0.000004),
                    output_cost_per_token: Some(0.000020),
                    ..Default::default()
                },
            );
            let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());

            let vertex = lookup
                .lookup_with_provider("claude-sonnet-4", Some(vertex_root))
                .unwrap_or_else(|| panic!("{vertex_root}-hinted claude-sonnet-4 must price"));
            assert_eq!(vertex.matched_key, hosted_key);
            assert_eq!(vertex.source, "LiteLLM");

            let anthropic = lookup
                .lookup_with_provider("claude-sonnet-4", Some("anthropic"))
                .expect("anthropic-hinted claude-sonnet-4 must price");
            assert_eq!(anthropic.matched_key, "anthropic/claude-sonnet-4");
            assert_eq!(anthropic.source, "OpenRouter");
        }
    }

    /// `vertex` and `vertex_ai` share a provider tag for fallback reachability,
    /// but are distinct billing endpoints. The literal root must win in either
    /// direction even though the longer `vertex_ai` key is ordered first.
    #[test]
    fn vertex_endpoint_aliases_do_not_impersonate_each_others_own_root() {
        let mut litellm = HashMap::new();
        for key in ["vertex/claude-sonnet-4", "vertex_ai/claude-sonnet-4"] {
            litellm.insert(
                key.to_string(),
                ModelPricing {
                    input_cost_per_token: Some(0.000003),
                    output_cost_per_token: Some(0.000015),
                    ..Default::default()
                },
            );
        }
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        for hint in ["vertex", "vertex_ai"] {
            let result = lookup
                .lookup_with_provider("claude-sonnet-4", Some(hint))
                .unwrap_or_else(|| panic!("{hint}-hinted claude-sonnet-4 must price"));
            assert_eq!(result.matched_key, format!("{hint}/claude-sonnet-4"));
        }
    }

    #[test]
    fn vertex_endpoint_literal_root_survives_cross_source_arbitration() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "vertex/claude-sonnet-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                ..Default::default()
            },
        );
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "vertex_ai/claude-sonnet-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000004),
                output_cost_per_token: Some(0.000020),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, openrouter, HashMap::new());

        for (hint, key, source) in [
            ("vertex", "vertex/claude-sonnet-4", "LiteLLM"),
            ("vertex_ai", "vertex_ai/claude-sonnet-4", "OpenRouter"),
        ] {
            let result = lookup
                .lookup_with_provider("claude-sonnet-4", Some(hint))
                .unwrap_or_else(|| panic!("{hint}-hinted claude-sonnet-4 must price"));
            assert_eq!(result.matched_key, key);
            assert_eq!(result.source, source);
        }
    }

    /// A literal provider root in Models.dev must participate in the same
    /// arbitration as LiteLLM and OpenRouter instead of losing to their
    /// alias-only row merely because Models.dev is normally the long-tail
    /// fallback. Exercise both directions of the Anthropic/Vertex relation.
    #[test]
    fn models_dev_literal_root_outranks_cross_source_endpoint_alias() {
        for (hint, own_root, alias_root) in [
            ("vertex", "vertex", "anthropic"),
            ("vertex_ai", "vertex_ai", "anthropic"),
            ("anthropic", "anthropic", "vertex"),
            ("anthropic", "anthropic", "vertex_ai"),
        ] {
            let mut litellm = HashMap::new();
            litellm.insert(
                format!("{alias_root}/claude-sonnet-4"),
                ModelPricing {
                    input_cost_per_token: Some(0.000003),
                    output_cost_per_token: Some(0.000015),
                    ..Default::default()
                },
            );
            let mut models_dev = HashMap::new();
            let own_key = format!("{own_root}/claude-sonnet-4");
            models_dev.insert(
                own_key.clone(),
                ModelPricing {
                    input_cost_per_token: Some(0.000004),
                    output_cost_per_token: Some(0.000020),
                    ..Default::default()
                },
            );
            let lookup = PricingLookup::new_with_models_dev(
                litellm,
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                models_dev,
            );

            let result = lookup
                .lookup_with_provider("claude-sonnet-4", Some(hint))
                .unwrap_or_else(|| panic!("{hint}-hinted claude-sonnet-4 must price"));
            assert_eq!(result.matched_key, own_key);
            assert_eq!(result.source, "Models.dev");
        }
    }

    #[test]
    fn normalized_models_dev_literal_root_outranks_cross_source_endpoint_alias() {
        for (hint, own_root, alias_root) in [
            ("vertex", "vertex", "anthropic"),
            ("vertex_ai", "vertex_ai", "anthropic"),
            ("anthropic", "anthropic", "vertex"),
            ("anthropic", "anthropic", "vertex_ai"),
        ] {
            let mut openrouter = HashMap::new();
            openrouter.insert(
                format!("{alias_root}/claude-sonnet-4-6"),
                ModelPricing {
                    input_cost_per_token: Some(0.000003),
                    output_cost_per_token: Some(0.000015),
                    ..Default::default()
                },
            );
            let mut models_dev = HashMap::new();
            let own_key = format!("{own_root}/claude-sonnet-4-6");
            models_dev.insert(
                own_key.clone(),
                ModelPricing {
                    input_cost_per_token: Some(0.000004),
                    output_cost_per_token: Some(0.000020),
                    ..Default::default()
                },
            );
            let lookup = PricingLookup::new_with_models_dev(
                HashMap::new(),
                openrouter,
                HashMap::new(),
                HashMap::new(),
                models_dev,
            );

            let result = lookup
                .lookup_with_provider("claude-sonnet-4.6", Some(hint))
                .unwrap_or_else(|| panic!("normalized {hint}-hinted Claude must price"));
            assert_eq!(result.matched_key, own_key);
            assert_eq!(result.source, "Models.dev");
        }
    }

    /// A root globally classified as a reseller can still be the hinted
    /// provider's own top-level row. Together's row must retain the root tier
    /// over a longer host that merely nests the Together spelling.
    #[test]
    fn reseller_classification_does_not_hide_hinted_provider_own_root() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "together_ai/model-x".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000001),
                output_cost_per_token: Some(0.000002),
                ..Default::default()
            },
        );
        litellm.insert(
            "long-host/together/model-x".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000004),
                ..Default::default()
            },
        );
        let lookup = PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let result = lookup
            .lookup_with_provider("model-x", Some("together"))
            .expect("together-hinted model-x must price");
        assert_eq!(result.matched_key, "together_ai/model-x");
    }
}
