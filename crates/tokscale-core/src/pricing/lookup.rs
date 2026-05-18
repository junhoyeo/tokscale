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
];

const FUZZY_BLOCKLIST: &[&str] = &["auto", "mini", "chat", "base"];

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
}

struct KeyModelPart {
    key: String,
    lower_model_part: String,
}

pub struct PricingLookup {
    litellm: HashMap<String, ModelPricing>,
    openrouter: HashMap<String, ModelPricing>,
    cursor: HashMap<String, ModelPricing>,
    litellm_keys: Vec<String>,
    openrouter_keys: Vec<String>,
    litellm_key_parts: Vec<KeyModelPart>,
    openrouter_key_parts: Vec<KeyModelPart>,
    litellm_lower: HashMap<String, String>,
    openrouter_lower: HashMap<String, String>,
    openrouter_model_part: HashMap<String, String>,
    cursor_lower: HashMap<String, String>,
    lookup_cache: RwLock<HashMap<String, Option<CachedResult>>>,
}

pub struct LookupResult {
    pub pricing: ModelPricing,
    pub source: String,
    pub matched_key: String,
}

impl PricingLookup {
    pub fn new(
        litellm: HashMap<String, ModelPricing>,
        openrouter: HashMap<String, ModelPricing>,
        cursor: HashMap<String, ModelPricing>,
    ) -> Self {
        let mut litellm_keys: Vec<String> = litellm.keys().cloned().collect();
        litellm_keys.sort_by_key(|k| std::cmp::Reverse(k.len()));

        let mut openrouter_keys: Vec<String> = openrouter.keys().cloned().collect();
        openrouter_keys.sort_by_key(|k| std::cmp::Reverse(k.len()));

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

        let mut cursor_lower = HashMap::with_capacity(cursor.len());
        for key in cursor.keys() {
            cursor_lower.insert(key.to_lowercase(), key.clone());
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

        Self {
            litellm,
            openrouter,
            cursor,
            litellm_keys,
            openrouter_keys,
            litellm_key_parts,
            openrouter_key_parts,
            litellm_lower,
            openrouter_lower,
            openrouter_model_part,
            cursor_lower,
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
        let provider_id = normalize_provider_hint(provider_id);
        let canonical = aliases::resolve_alias(model_id).unwrap_or(model_id);
        let lower = canonical.to_lowercase();

        // CLIProxyAPI strips `(level)` reasoning-effort suffixes before routing,
        // so for pricing lookup we resolve to the base model regardless of tier.
        // Mirrors the dash-suffix path (e.g. `-xhigh`), which is handled by
        // `try_strip_unknown_suffix` below.
        let normalized_owned = strip_parenthesized_reasoning_tier(&lower).map(str::to_owned);

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
            _ => self.lookup_auto(id, provider_id),
        };

        // 1. Try direct lookup
        if let Some(result) = do_lookup(lower_ref) {
            return Some(result);
        }

        // 2. Try stripping unknown suffixes (e.g., -thinking, -high, -codex)
        if let Some(result) = try_strip_unknown_suffix(lower_ref, do_lookup) {
            return Some(result);
        }

        // 3. Try stripping unknown prefixes (e.g., antigravity-, myplugin-)
        //    For each prefix candidate, also try suffix stripping
        if let Some(result) = try_strip_unknown_prefix(lower_ref, do_lookup) {
            return Some(result);
        }

        None
    }

    fn lookup_auto(&self, model_id: &str, provider_id: Option<&str>) -> Option<LookupResult> {
        if let Some(stripped) = strip_known_provider_prefix(model_id) {
            let prefix_matches_hint =
                provider_id.is_none() || model_prefix_matches_provider(model_id, provider_id);

            if prefix_matches_hint {
                if let Some(exact_litellm) = self.exact_match_litellm(model_id) {
                    return Some(exact_litellm);
                }

                let exact_openrouter = self.exact_match_openrouter(model_id);
                let stripped_litellm = self.exact_or_normalized_litellm(stripped, provider_id);

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
            } else {
                if let Some(result) = choose_best_source_result(
                    self.exact_match_litellm_for_provider(stripped, provider_id),
                    self.exact_match_openrouter_for_provider(stripped, provider_id),
                    provider_id,
                ) {
                    return Some(result);
                }
                if let Some(result) = self.exact_or_normalized_litellm(stripped, provider_id) {
                    return Some(result);
                }
            }
        }

        if let Some(result) = choose_best_source_result(
            self.exact_match_litellm_for_provider(model_id, provider_id),
            self.exact_match_openrouter_for_provider(model_id, provider_id),
            provider_id,
        ) {
            return Some(result);
        }

        if let Some(result) = self.exact_match_litellm(model_id) {
            return Some(result);
        }
        if let Some(result) = self.exact_match_openrouter(model_id) {
            return Some(result);
        }

        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = choose_best_source_result(
                self.exact_match_litellm_for_provider(&version_normalized, provider_id),
                self.exact_match_openrouter_for_provider(&version_normalized, provider_id),
                provider_id,
            ) {
                return Some(result);
            }
            if let Some(result) = self.exact_match_litellm(&version_normalized) {
                return Some(result);
            }
            if let Some(result) = self.exact_match_openrouter(&version_normalized) {
                return Some(result);
            }
        }

        if let Some(normalized) = normalize_model_name(model_id) {
            if let Some(result) = choose_best_source_result(
                self.exact_match_litellm_for_provider(&normalized, provider_id),
                self.exact_match_openrouter_for_provider(&normalized, provider_id),
                provider_id,
            ) {
                return Some(result);
            }
            if let Some(result) = self.exact_match_litellm(&normalized) {
                return Some(result);
            }
            if let Some(result) = self.exact_match_openrouter(&normalized) {
                return Some(result);
            }
        }

        if let Some(result) = self.prefix_match_litellm(model_id, provider_id) {
            return Some(result);
        }
        if let Some(result) = self.prefix_match_openrouter(model_id, provider_id) {
            return Some(result);
        }

        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.prefix_match_litellm(&version_normalized, provider_id) {
                return Some(result);
            }
            if let Some(result) = self.prefix_match_openrouter(&version_normalized, provider_id) {
                return Some(result);
            }
        }

        if let Some(result) = self.exact_match_cursor(model_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.exact_match_cursor(&version_normalized) {
                return Some(result);
            }
        }

        if !is_fuzzy_eligible(model_id) {
            return None;
        }

        let litellm_result = self.fuzzy_match_litellm(model_id, provider_id);
        let openrouter_result = self.fuzzy_match_openrouter(model_id, provider_id);

        choose_best_source_result(litellm_result, openrouter_result, provider_id)
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
                return Some(result);
            }
            if let Some(result) = self.exact_match_litellm(&version_normalized) {
                return Some(result);
            }
        }
        if let Some(normalized) = normalize_model_name(model_id) {
            if let Some(result) = self.exact_match_litellm_for_provider(&normalized, provider_id) {
                return Some(result);
            }
            if let Some(result) = self.exact_match_litellm(&normalized) {
                return Some(result);
            }
        }
        None
    }

    fn lookup_litellm_only(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        if let Some(result) = self.exact_or_normalized_litellm(model_id, provider_id) {
            return Some(result);
        }
        if let Some(stripped) = strip_known_provider_prefix(model_id) {
            if let Some(result) = self.exact_or_normalized_litellm(stripped, provider_id) {
                return Some(result);
            }
        }
        if let Some(result) = self.prefix_match_litellm(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.prefix_match_litellm(&version_normalized, provider_id) {
                return Some(result);
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
        if let Some(result) = self.exact_match_openrouter_with_provider(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) =
                self.exact_match_openrouter_with_provider(&version_normalized, provider_id)
            {
                return Some(result);
            }
        }
        if let Some(normalized) = normalize_model_name(model_id) {
            if let Some(result) =
                self.exact_match_openrouter_with_provider(&normalized, provider_id)
            {
                return Some(result);
            }
        }
        if let Some(result) = self.prefix_match_openrouter(model_id, provider_id) {
            return Some(result);
        }
        if let Some(version_normalized) = normalize_version_separator(model_id) {
            if let Some(result) = self.prefix_match_openrouter(&version_normalized, provider_id) {
                return Some(result);
            }
        }
        if is_fuzzy_eligible(model_id) {
            if let Some(result) = self.fuzzy_match_openrouter(model_id, provider_id) {
                return Some(result);
            }
        }
        None
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

    fn exact_match_litellm(&self, model_id: &str) -> Option<LookupResult> {
        let key = self.litellm_lower.get(model_id)?;
        let pricing = self.litellm.get(key)?;
        Some(LookupResult {
            pricing: pricing.clone(),
            source: "LiteLLM".into(),
            matched_key: key.clone(),
        })
    }

    fn exact_match_openrouter(&self, model_id: &str) -> Option<LookupResult> {
        if let Some(key) = self.openrouter_lower.get(model_id) {
            if let Some(pricing) = self.openrouter.get(key) {
                return Some(LookupResult {
                    pricing: pricing.clone(),
                    source: "OpenRouter".into(),
                    matched_key: key.clone(),
                });
            }
        }
        if let Some(key) = self.openrouter_model_part.get(model_id) {
            if let Some(pricing) = self.openrouter.get(key) {
                return Some(LookupResult {
                    pricing: pricing.clone(),
                    source: "OpenRouter".into(),
                    matched_key: key.clone(),
                });
            }
        }
        None
    }

    fn exact_match_cursor(&self, model_id: &str) -> Option<LookupResult> {
        if let Some(key) = self.cursor_lower.get(model_id) {
            return Some(LookupResult {
                pricing: self.cursor.get(key).unwrap().clone(),
                source: "Cursor".into(),
                matched_key: key.clone(),
            });
        }
        if let Some(model_part) = model_id.split('/').next_back() {
            if model_part != model_id {
                if let Some(key) = self.cursor_lower.get(model_part) {
                    return Some(LookupResult {
                        pricing: self.cursor.get(key).unwrap().clone(),
                        source: "Cursor".into(),
                        matched_key: key.clone(),
                    });
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
                    return Some(LookupResult {
                        pricing: pricing.clone(),
                        source: "LiteLLM".into(),
                        matched_key: litellm_key.clone(),
                    });
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
                    return Some(LookupResult {
                        pricing: pricing.clone(),
                        source: "OpenRouter".into(),
                        matched_key: or_key.clone(),
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

        if let Some(result) =
            select_best_match(&family_matches_list, &self.litellm, "LiteLLM", provider_id)
        {
            return Some(result);
        }

        let mut all_matches: Vec<&String> = Vec::new();
        for key in &self.litellm_keys {
            let lower_key = key.to_lowercase();
            if contains_model_id(&lower_key, model_id) {
                all_matches.push(key);
            }
        }

        select_best_match(&all_matches, &self.litellm, "LiteLLM", provider_id)
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

        select_best_match(&all_matches, &self.openrouter, "OpenRouter", provider_id)
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
        let result = match self.lookup_with_provider(model_id, provider_id) {
            Some(r) => r,
            None => return 0.0,
        };

        compute_cost(
            &result.pricing,
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
            usage.reasoning,
        )
    }
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
    // `has_any_usable_pricing`, `has_any_valid_above_tier_value`, and
    // `has_meaningful_tier_support`; otherwise tier walks will silently
    // undercost long-context cache reads on those models.
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

    if lower.contains("opus") {
        if contains_delimited_fragment(&lower, "4.6") || contains_delimited_fragment(&lower, "4-6")
        {
            return Some("claude-opus-4-6".into());
        } else if contains_delimited_fragment(&lower, "4.5")
            || contains_delimited_fragment(&lower, "4-5")
        {
            return Some("claude-opus-4-5".into());
        } else if contains_delimited_fragment(&lower, "4") {
            return Some("claude-opus-4".into());
        }
    }
    if lower.contains("sonnet") {
        if contains_delimited_fragment(&lower, "4.5") || contains_delimited_fragment(&lower, "4-5")
        {
            return Some("claude-sonnet-4-5".into());
        } else if contains_delimited_fragment(&lower, "4") {
            return Some("claude-sonnet-4".into());
        } else if contains_delimited_fragment(&lower, "3.7")
            || contains_delimited_fragment(&lower, "3-7")
        {
            return Some("claude-3-7-sonnet".into());
        } else if contains_delimited_fragment(&lower, "3.5")
            || contains_delimited_fragment(&lower, "3-5")
        {
            return Some("claude-3.5-sonnet".into());
        }
    }
    if lower.contains("haiku") {
        if contains_delimited_fragment(&lower, "4.5") || contains_delimited_fragment(&lower, "4-5")
        {
            return Some("claude-haiku-4-5".into());
        } else if contains_delimited_fragment(&lower, "3.5")
            || contains_delimited_fragment(&lower, "3-5")
        {
            return Some("claude-3.5-haiku".into());
        }
    }

    None
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

fn is_valid_price_value(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// Returns true if the pricing entry has at least one usable cost field
/// (base or above-200k tier). Entries with all-None pricing (e.g.
/// subscription-based providers like Perplexity) are useless for
/// pay-per-token cost estimation and should be deprioritized.
fn has_any_usable_pricing(pricing: &ModelPricing) -> bool {
    [
        pricing.input_cost_per_token,
        pricing.output_cost_per_token,
        pricing.cache_read_input_token_cost,
        pricing.cache_creation_input_token_cost,
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
    .any(|opt| opt.is_some_and(is_valid_price_value))
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
    let parts: Vec<&str> = model_id.split('-').collect();

    if parts.len() < 2 {
        return None;
    }

    let max_strip = std::cmp::min(parts.len() - 1, MAX_SUFFIX_STRIP_SEGMENTS);

    for strip in 1..=max_strip {
        let candidate: String = parts[..parts.len() - strip].join("-");

        if candidate.len() >= MIN_MODEL_NAME_LEN {
            if let Some(result) = do_lookup(&candidate) {
                return Some(result);
            }
        }
    }

    None
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

fn is_original_provider(key: &str) -> bool {
    let lower = key.to_lowercase();
    ORIGINAL_PROVIDER_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn is_reseller_provider(key: &str) -> bool {
    let lower = key.to_lowercase();
    RESELLER_PROVIDER_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn select_best_match(
    matches: &[&String],
    dataset: &HashMap<String, ModelPricing>,
    source: &str,
    provider_id: Option<&str>,
) -> Option<LookupResult> {
    if matches.is_empty() {
        return None;
    }

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
            candidates
                .iter()
                .find(|k| is_original_provider(k))
                .or_else(|| candidates.iter().find(|k| !is_reseller_provider(k)))
                .or_else(|| candidates.first())
        };
        key.and_then(|k| {
            dataset.get(k.as_str()).map(|pricing| LookupResult {
                pricing: pricing.clone(),
                source: source.into(),
                matched_key: (*k).clone(),
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

    select_best_match(&matches, dataset, source, Some(provider_id))
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
    fn test_normalize_opus_4_60_does_not_map_to_4_6() {
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
        let result = lookup.lookup("opus-4-60").unwrap();
        assert_eq!(result.matched_key, "claude-opus-4");
        assert_ne!(result.matched_key, "claude-opus-4-6");
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
        assert!(is_fuzzy_eligible("claude"));
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
}
