fn canonicalize_provider_segment(segment: &str) -> Option<String> {
    let normalized = segment
        .trim()
        .trim_end_matches('/')
        .to_lowercase()
        .replace('-', "_");
    if normalized.starts_with('<') && normalized.ends_with('>') {
        return None;
    }

    let canonical = match normalized.as_str() {
        "" | "unknown" => return None,
        "x_ai" | "xai" => "xai",
        "z_ai" | "zai" => "zai",
        "moonshot" | "moonshotai" => "moonshotai",
        "meta" | "meta_llama" => "meta_llama",
        "azure" | "azure_ai" => "azure_ai",
        "anthropic" | "vertex" | "vertex_ai" => "anthropic",
        "together" | "together_ai" => "together_ai",
        "fireworks" | "fireworks_ai" => "fireworks_ai",
        "google" | "gemini" => "google",
        "openai" | "openai_codex" => "openai",
        "minimax" | "minimaxai" | "minimax_ai" => "minimax",
        "mistral" | "mistralai" => "mistralai",
        "ai21" => "ai21",
        // The `-ai` suffix is a spelling of the vendor, not a different
        // vendor. DeepSeek is the case that actually costs us: the live
        // datasets split the same model almost evenly between the two
        // spellings depending on who is reselling it —
        // `zenmux/deepseek/deepseek-v3.2-exp` and `kilo/deepseek/...` against
        // `nano-gpt/deepseek-ai/deepseek-v3.2-exp` and `siliconflow/...` — so
        // whether usage of one model carried the vendor tag `deepseek` or
        // `deepseek_ai` was decided by which reseller served it.
        //
        // Folding is only safe because the two spellings never name the same
        // row: no reseller in either dataset lists a model under both, and
        // `deepseek-ai` is never a top-level provider (it is the HuggingFace
        // org name, always a nested segment), so nothing here collapses two
        // separately-priced rows into one.
        "deepseek" | "deepseek_ai" => "deepseek",
        "novita" | "novita_ai" => "novita",
        "stepfun" | "stepfun_ai" => "stepfun",
        // A `-cn` suffix is NOT a spelling variant and must never be folded in
        // here, however much it looks like one. It marks a regional endpoint
        // with its own price sheet: `alibaba` and `alibaba-cn` share 45 models
        // and disagree on 41 of them, with `qwen-max` at $1.60/$6.40 against
        // $0.345/$1.377 — a 4.6x error in whichever direction the fold went.
        // `siliconflow` and `siliconflow-cn` disagree on 7 of 35. Adding those
        // arms to "finish the pattern" would silently misprice every user on
        // the endpoint that lost the fold.
        // For unknown segments, reject if they contain digits — those are
        // almost certainly model-name fragments (e.g., "gpt-4", "claude-3")
        // rather than provider identifiers.
        other if other.chars().any(|ch| ch.is_ascii_digit()) => return None,
        other => other,
    };

    Some(canonical.into())
}

pub fn canonical_provider(raw: &str) -> Option<String> {
    provider_tags(raw).into_iter().next()
}

pub fn provider_tags(raw: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut push = |segment: &str| {
        if let Some(tag) = canonicalize_provider_segment(segment) {
            if !tags.iter().any(|existing| existing == &tag) {
                tags.push(tag);
            }
        }
    };

    for segment in raw.trim().trim_end_matches('/').split('/') {
        push(segment);
        if segment.contains('.') {
            for dotted in segment.split('.') {
                push(dotted);
            }
        }
    }

    tags
}

pub fn key_provider_tags(dataset_key: &str) -> Vec<String> {
    let key_parts: Vec<&str> = dataset_key.split('/').collect();
    if key_parts.len() < 2 {
        return Vec::new();
    }

    let mut tags = Vec::new();
    let mut push_all = |value: &str| {
        for tag in provider_tags(value) {
            if !tags.iter().any(|existing| existing == &tag) {
                tags.push(tag);
            }
        }
    };

    for segment in &key_parts[..key_parts.len() - 1] {
        push_all(segment);
    }
    for dotted in key_parts[key_parts.len() - 1].split('.') {
        push_all(dotted);
    }

    tags
}

/// Provider segments a value names *verbatim*, with no alias folding.
///
/// Lowercased and underscore-normalized so `DeepSeek-AI`, `deepseek-ai` and
/// `deepseek_ai` compare equal, but `deepseek` and `deepseek_ai` do not.
fn raw_provider_segments(value: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut push = |segment: &str| {
        let normalized = segment
            .trim()
            .trim_end_matches('/')
            .to_lowercase()
            .replace('-', "_");
        if normalized.is_empty() || segments.iter().any(|existing| existing == &normalized) {
            return;
        }
        segments.push(normalized);
    };

    for segment in value.trim().trim_end_matches('/').split('/') {
        push(segment);
        if segment.contains('.') {
            for dotted in segment.split('.') {
                push(dotted);
            }
        }
    }

    segments
}

/// Whether `dataset_key` spells a vendor exactly the way `provider_id` does.
///
/// `canonicalize_provider_segment` folds spelling variants of one vendor
/// together (`deepseek-ai` -> `deepseek`) so a hint can reach rows that spell
/// the vendor the other way. That fold also widens the candidate pool: a
/// `deepseek` hint now matches both `novita/deepseek/<model>` and
/// `cloudflare/@cf/deepseek-ai/<model>`, which are two resellers with
/// different price sheets for the same weights. When the hinted vendor
/// publishes no first-party row for the model, nothing else in
/// `select_best_match` distinguishes those two, and the winner falls out of
/// dataset key ordering. This predicate is the tiebreak that keeps the fold
/// from re-rolling that choice: a row spelling the vendor exactly as the hint
/// does wins over one that only matches after folding.
pub(crate) fn matches_provider_spelling(dataset_key: &str, provider_id: &str) -> bool {
    let hint_segments = raw_provider_segments(provider_id);
    if hint_segments.is_empty() {
        return false;
    }

    let key_parts: Vec<&str> = dataset_key.split('/').collect();
    if key_parts.len() < 2 {
        return false;
    }

    // The final component is the model name, but an AWS-style id carries the
    // provider in a dotted prefix of it — `amazon-bedrock/us.deepseek.r1-v1:0`
    // is DeepSeek's row, not Amazon's own model. `key_provider_tags` already
    // splits that component on `.` for exactly this reason, so read the same
    // segments here: dropping them lets a `deepseek` hint miss the row that
    // spells the vendor its way and fall through to a differently spelled
    // reseller. Only the dotted *prefix* counts; the trailing piece is the
    // model name and is never a vendor spelling.
    let last = key_parts[key_parts.len() - 1];
    let dotted_provider_prefix = last.rsplit_once('.').map(|(prefix, _)| prefix);

    key_parts[..key_parts.len() - 1]
        .iter()
        .copied()
        .chain(dotted_provider_prefix)
        .flat_map(raw_provider_segments)
        .any(|key_segment| hint_segments.iter().any(|hint| hint == &key_segment))
}

pub fn matches_provider_hint(dataset_key: &str, provider_id: Option<&str>) -> bool {
    let Some(provider_id) = provider_id else {
        return false;
    };

    let hint_tags = provider_tags(provider_id);
    matches_provider_hint_with_tags(dataset_key, &hint_tags)
}

pub fn matches_provider_hint_with_tags(dataset_key: &str, hint_tags: &[String]) -> bool {
    if hint_tags.is_empty() {
        return false;
    }

    let key_tags = key_provider_tags(dataset_key);
    if key_tags.is_empty() {
        return false;
    }

    key_tags
        .iter()
        .any(|key_tag| hint_tags.iter().any(|hint_tag| hint_tag == key_tag))
}

fn contains_delimited(haystack: &str, needle: &str) -> bool {
    for (pos, _) in haystack.match_indices(needle) {
        let before_ok = pos == 0 || !haystack.as_bytes()[pos - 1].is_ascii_alphanumeric();
        let after_pos = pos + needle.len();
        let after_ok =
            after_pos == haystack.len() || !haystack.as_bytes()[after_pos].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Match a model-family token at a real leading boundary. A decimal digit may
/// immediately follow the family because catalog ids commonly concatenate a
/// major version (`gpt4`, `claude3`, `qwen3`); an ASCII letter may not, which
/// rejects ordinary words that merely contain a family name.
fn contains_versioned_family(haystack: &str, family: &str) -> bool {
    for (pos, _) in haystack.match_indices(family) {
        let before_ok = haystack[..pos]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_alphanumeric());
        let after_pos = pos + family.len();
        let after_ok = haystack[after_pos..]
            .chars()
            .next()
            .is_none_or(|character| character.is_ascii_digit() || !character.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

fn contains_family(haystack: &str, family: &str, delimiter_aware: bool) -> bool {
    if delimiter_aware {
        contains_versioned_family(haystack, family)
    } else {
        haystack.contains(family)
    }
}

pub fn inferred_provider_from_model(model: &str) -> Option<&'static str> {
    inferred_provider_from_model_inner(model, false)
}

pub(crate) fn inferred_provider_from_model_delimited(model: &str) -> Option<&'static str> {
    inferred_provider_from_model_inner(model, true)
}

fn inferred_provider_from_model_inner(
    model: &str,
    delimit_family_names: bool,
) -> Option<&'static str> {
    let lower = model.to_lowercase();

    // Ollama is a routing prefix, not part of the upstream model family. In
    // particular, matching the `llama` in `ollama/...` would label every
    // otherwise-unknown Ollama model as Meta. Re-run inference on the routed
    // model so known families retain their actual providers.
    if let Some(routed_model) = lower.strip_prefix("ollama/") {
        return inferred_provider_from_model_inner(routed_model, delimit_family_names);
    }

    if contains_family(&lower, "claude", delimit_family_names)
        || contains_family(&lower, "anthropic", delimit_family_names)
        || contains_versioned_family(&lower, "opus")
        || contains_versioned_family(&lower, "sonnet")
        || contains_versioned_family(&lower, "haiku")
        || contains_versioned_family(&lower, "fable")
    {
        return Some("anthropic");
    }

    if contains_family(&lower, "gpt", delimit_family_names)
        || contains_family(&lower, "openai", delimit_family_names)
        || contains_delimited(&lower, "o1")
        || contains_delimited(&lower, "o3")
        || contains_delimited(&lower, "o4")
    {
        return Some("openai");
    }

    if contains_family(&lower, "gemini", delimit_family_names)
        || contains_family(&lower, "google", delimit_family_names)
    {
        return Some("google");
    }

    if contains_family(&lower, "grok", delimit_family_names) {
        return Some("xai");
    }

    if contains_family(&lower, "deepseek", delimit_family_names) {
        return Some("deepseek");
    }

    if contains_family(&lower, "minimax", delimit_family_names) {
        return Some("minimax");
    }

    if contains_family(&lower, "mistral", delimit_family_names)
        || contains_family(&lower, "mixtral", delimit_family_names)
    {
        return Some("mistral");
    }

    if contains_family(&lower, "llama", delimit_family_names)
        || contains_versioned_family(&lower, "meta")
    {
        return Some("meta");
    }

    if contains_family(&lower, "qwen", delimit_family_names) {
        return Some("qwen");
    }

    // Sakana's `fugu` / `fugu-ultra` model line. Bare `fugu` is intentionally
    // still mapped to the sakana provider here (provider identity is independent
    // of whether we can price the model — see build_sakana_overrides, which
    // deliberately does NOT price bare `fugu`).
    if contains_family(&lower, "fugu", delimit_family_names) {
        return Some("sakana");
    }

    // Kimi (Moonshot AI) — `kimi`, `kimi-k2.5`, `kimi-code` variants.
    if contains_versioned_family(&lower, "kimi") {
        return Some("moonshotai");
    }
    // Kimi's own coding-plan catalog also serves bare `k2`/`k3`-style ids with
    // no `kimi` prefix at all (e.g. `k3`, `k3-256k` from the K3 coding-plan
    // model), so the `kimi` substring check above misses them. No other known
    // provider uses a bare, delimited `k2`/`k3` model id (checked against the
    // full litellm/models.dev/openrouter pricing datasets), so this is safe
    // without the `kimi` prefix.
    if contains_delimited(&lower, "k2") || contains_delimited(&lower, "k3") {
        return Some("moonshotai");
    }
    // MiMo (Xiaomi) — `mimo-v2.5` etc.
    if contains_versioned_family(&lower, "mimo") {
        return Some("xiaomi");
    }
    // GLM (Zhipu AI / Zai) — `glm-4.6`, `glm-5.2` etc.
    if contains_versioned_family(&lower, "glm") {
        return Some("zai");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_tags_normalize_known_aliases() {
        let cases = [
            ("openai-codex", vec!["openai"]),
            ("gemini", vec!["google"]),
            ("vertex", vec!["anthropic"]),
            ("azure", vec!["azure_ai"]),
            ("fireworks", vec!["fireworks_ai"]),
            ("MiniMax", vec!["minimax"]),
            ("openrouter/google", vec!["openrouter", "google"]),
            ("bedrock/anthropic", vec!["bedrock", "anthropic"]),
        ];

        for (raw, expected) in cases {
            assert_eq!(provider_tags(raw), expected);
        }
    }

    #[test]
    fn test_canonical_provider_returns_first_canonical_tag() {
        assert_eq!(canonical_provider("openai-codex"), Some("openai".into()));
        assert_eq!(
            canonical_provider("openrouter/google"),
            Some("openrouter".into())
        );
        assert_eq!(canonical_provider("<synthetic>"), None);
        assert_eq!(canonical_provider("unknown"), None);
    }

    #[test]
    fn test_key_provider_tags_extract_nested_provider_segments() {
        assert_eq!(
            key_provider_tags("openrouter/google/gemini-3-pro-preview"),
            vec!["openrouter", "google"]
        );
        assert_eq!(
            key_provider_tags("bedrock/anthropic.claude-sonnet-4"),
            vec!["bedrock", "anthropic"]
        );
    }

    #[test]
    fn test_matches_provider_hint_for_known_aliases_and_nested_keys() {
        assert!(matches_provider_hint(
            "openai/gpt-5.2-preview",
            Some("openai-codex")
        ));
        assert!(matches_provider_hint(
            "openrouter/google/gemini-3-pro-preview",
            Some("google")
        ));
        assert!(matches_provider_hint("azure/openai/gpt-4", Some("azure")));
        assert!(matches_provider_hint(
            "fireworks_ai/deepseek-v3-0324",
            Some("fireworks")
        ));
        assert!(!matches_provider_hint("openai/gpt-4", Some("anthropic")));
    }

    #[test]
    fn fable_models_map_to_anthropic() {
        // Fable is a Claude model family; the bare, claude-prefixed, and [1m]
        // context-variant forms must all attribute to Anthropic.
        assert_eq!(inferred_provider_from_model("fable-5"), Some("anthropic"));
        assert_eq!(
            inferred_provider_from_model("claude-fable-5"),
            Some("anthropic")
        );
        assert_eq!(
            inferred_provider_from_model("claude-fable-5[1m]"),
            Some("anthropic")
        );
    }

    #[test]
    fn test_inferred_provider_from_model() {
        assert_eq!(
            inferred_provider_from_model("claude-sonnet-4"),
            Some("anthropic")
        );
        assert_eq!(inferred_provider_from_model("gpt-5.2"), Some("openai"));
        assert_eq!(inferred_provider_from_model("gpt-5.5"), Some("openai"));
        assert_eq!(
            inferred_provider_from_model("gemini-2.5-pro"),
            Some("google")
        );
        assert_eq!(
            inferred_provider_from_model("grok-code-fast-1"),
            Some("xai")
        );
        assert_eq!(
            inferred_provider_from_model("deepseek-v3"),
            Some("deepseek")
        );
        assert_eq!(
            inferred_provider_from_model("MiniMax-M2.1"),
            Some("minimax")
        );
        assert_eq!(
            inferred_provider_from_model("mixtral-8x7b"),
            Some("mistral")
        );
        assert_eq!(
            inferred_provider_from_model("mistral-large"),
            Some("mistral")
        );
        assert_eq!(inferred_provider_from_model("llama-3"), Some("meta"));
        assert_eq!(inferred_provider_from_model("qwen3-coder"), Some("qwen"));
        assert_eq!(inferred_provider_from_model("unknown-model"), None);
    }

    #[test]
    fn test_inferred_provider_bare_kimi_k_series_ids() {
        // Kimi's coding-plan catalog serves these with no `kimi` prefix at all.
        assert_eq!(inferred_provider_from_model("k3"), Some("moonshotai"));
        assert_eq!(inferred_provider_from_model("k3-256k"), Some("moonshotai"));
        assert_eq!(inferred_provider_from_model("K3"), Some("moonshotai"));
        assert_eq!(inferred_provider_from_model("k2"), Some("moonshotai"));
        // Already-prefixed forms keep matching via the `kimi` substring check.
        assert_eq!(
            inferred_provider_from_model("kimi-k2.5-thinking"),
            Some("moonshotai")
        );
        // A `k2`/`k3` substring that isn't a delimited token must not match.
        assert_eq!(inferred_provider_from_model("flock3"), None);
        assert_eq!(inferred_provider_from_model("network2"), None);
    }

    #[test]
    fn test_inferred_provider_ignores_ollama_route_prefix() {
        assert_eq!(inferred_provider_from_model("ollama/orca-mini"), None);
        assert_eq!(
            inferred_provider_from_model("ollama/qwen3-coder"),
            Some("qwen")
        );
        assert_eq!(
            inferred_provider_from_model("ollama/llama-3.3"),
            Some("meta")
        );
    }

    #[test]
    fn test_inferred_provider_fugu_maps_to_sakana() {
        assert_eq!(inferred_provider_from_model("fugu"), Some("sakana"));
        assert_eq!(inferred_provider_from_model("fugu-ultra"), Some("sakana"));
        assert_eq!(inferred_provider_from_model("Fugu"), Some("sakana"));
        assert_eq!(inferred_provider_from_model("FUGU-ULTRA"), Some("sakana"));
    }

    #[test]
    fn test_provider_tags_preserves_sakana() {
        assert_eq!(provider_tags("sakana"), vec!["sakana"]);
    }

    #[test]
    fn test_inferred_provider_no_false_positives() {
        assert_eq!(inferred_provider_from_model("protocol1-fast"), None);
        assert_eq!(inferred_provider_from_model("proto3-server"), None);
        assert_eq!(inferred_provider_from_model("co4pilot-v2"), None);
        assert_eq!(inferred_provider_from_model("metadata-model"), None);
        assert_eq!(inferred_provider_from_model("metamorphic-v1"), None);
    }

    #[test]
    fn delimiter_aware_inference_accepts_family_versions_not_word_substrings() {
        let genuine = [
            ("gpt4-turbo", "openai"),
            ("claude3-opus", "anthropic"),
            ("opus4", "anthropic"),
            ("sonnet4", "anthropic"),
            ("haiku3", "anthropic"),
            ("fable5", "anthropic"),
            ("gemini2-pro", "google"),
            ("grok3", "xai"),
            ("deepseek3", "deepseek"),
            ("minimax2", "minimax"),
            ("mistral4", "mistral"),
            ("mixtral8x7b", "mistral"),
            ("llama3", "meta"),
            ("meta3", "meta"),
            ("qwen3-coder", "qwen"),
            ("fugu2", "sakana"),
            ("kimi2", "moonshotai"),
            ("mimo2", "xiaomi"),
            ("glm5", "zai"),
        ];
        for (model, provider) in genuine {
            assert_eq!(
                inferred_provider_from_model_delimited(model),
                Some(provider),
                "{model}"
            );
        }

        for model in [
            "agpt-model",
            "declaude-x",
            "pregeminified",
            "engroked",
            "deepseeking",
            "minimaximal",
            "demistralized",
            "remixtralized",
            "collamated",
            "unqwened-model",
            "defugued",
            "skimimoed",
            "glimmer",
            "unkimied",
            "unsonneted",
            "alphabetafablegamma",
            "préqwened",
            "qwené-model",
        ] {
            assert_eq!(
                inferred_provider_from_model_delimited(model),
                None,
                "{model} is not a genuine family token"
            );
        }
    }

    /// The families below are matched with plain `contains`, not
    /// `contains_delimited`, and that asymmetry is load-bearing rather than an
    /// oversight. Vendors append version digits directly to the family token
    /// (`qwen3`, `mistral4`) and embed it mid-word (`chatgpt-4o-latest`,
    /// `codellama`), all of which a delimited match rejects.
    ///
    /// Switching these to delimited matching drops the provider on 536 model ids
    /// in the bundled models.dev/litellm/openrouter catalogs. `contains_delimited`
    /// stays reserved for short tokens that collide inside ordinary words -- see
    /// `test_inferred_provider_no_false_positives`.
    #[test]
    fn test_inferred_provider_matches_version_suffixed_and_embedded_families() {
        for model in [
            "qwen3-coder",
            "qwen3.7-plus",
            "qwen2-5-14b-instruct",
            "qwen3-235b-a22b-instruct-2507",
        ] {
            assert_eq!(inferred_provider_from_model(model), Some("qwen"), "{model}");
        }

        for model in ["chatgpt-4o-latest", "chatgpt-image-latest"] {
            assert_eq!(
                inferred_provider_from_model(model),
                Some("openai"),
                "{model}"
            );
        }

        assert_eq!(
            inferred_provider_from_model("mistral4-119b"),
            Some("mistral")
        );
        assert_eq!(
            inferred_provider_from_model("CodeLlama-34b-Instruct-hf"),
            Some("meta")
        );
    }

    #[test]
    fn test_inferred_provider_boundary_matches() {
        assert_eq!(inferred_provider_from_model("o1-preview"), Some("openai"));
        assert_eq!(inferred_provider_from_model("o3-mini"), Some("openai"));
        assert_eq!(inferred_provider_from_model("o4-mini"), Some("openai"));
        assert_eq!(inferred_provider_from_model("meta-llama-3"), Some("meta"));
    }

    #[test]
    fn test_provider_tags_mistral_alias() {
        assert_eq!(provider_tags("mistral"), vec!["mistralai"]);
        assert_eq!(provider_tags("mistralai"), vec!["mistralai"]);
    }

    #[test]
    fn test_matches_provider_hint_mistral_keys() {
        assert!(matches_provider_hint(
            "mistralai/mistral-large",
            Some("mistral")
        ));
        assert!(matches_provider_hint(
            "mistralai/mixtral-8x7b",
            Some("mistralai")
        ));
    }

    #[test]
    fn test_provider_tags_ai21_with_digits() {
        assert_eq!(provider_tags("ai21"), vec!["ai21"]);
    }

    #[test]
    fn test_matches_provider_hint_none_and_empty() {
        assert!(!matches_provider_hint("openai/gpt-4", None));
        assert!(!matches_provider_hint("openai/gpt-4", Some("")));
        assert!(!matches_provider_hint("openai/gpt-4", Some("unknown")));
    }

    #[test]
    fn test_gjc_unknown_provider_passthrough() {
        // gjc's common providers ARE known and canonicalize as usual.
        assert_eq!(canonical_provider("anthropic"), Some("anthropic".into()));
        assert_eq!(canonical_provider("openai"), Some("openai".into()));
        assert_eq!(canonical_provider("openai-codex"), Some("openai".into()));
        assert_eq!(canonical_provider("google"), Some("google".into()));
        assert_eq!(
            canonical_provider("github-copilot"),
            Some("github_copilot".into())
        );

        // A gjc provider value that looks like a model fragment (contains
        // digits) or a placeholder is NOT treated as a provider: canonical_provider
        // yields None so the aggregator keeps the raw value verbatim rather than
        // misattributing it. This guards the unknown-provider passthrough path.
        assert_eq!(canonical_provider("gjc-model-4o"), None);
        assert_eq!(canonical_provider("<unset>"), None);
    }

    #[test]
    fn vendor_ai_suffix_is_one_vendor() {
        // The datasets split the same DeepSeek model between two vendor
        // spellings depending on the reseller, so before this fold the tag a
        // user's usage carried was decided by who served it.
        for spelling in ["deepseek", "deepseek-ai", "deepseek_ai", "DeepSeek-AI"] {
            assert_eq!(
                canonical_provider(spelling),
                Some("deepseek".into()),
                "{spelling} must canonicalize to deepseek"
            );
        }

        // Real dataset keys, where the vendor sits in a nested segment.
        assert_eq!(
            provider_tags("nano-gpt/deepseek-ai/deepseek-v3.2-exp"),
            vec!["nano_gpt", "deepseek"]
        );
        assert_eq!(
            provider_tags("zenmux/deepseek/deepseek-v3.2-exp"),
            vec!["zenmux", "deepseek"]
        );

        assert_eq!(canonical_provider("novita-ai"), Some("novita".into()));
        assert_eq!(canonical_provider("stepfun-ai"), Some("stepfun".into()));
    }

    #[test]
    fn regional_cn_endpoint_is_not_folded_into_the_global_one() {
        // Guards the comment above the `-ai` arms. `-cn` reads like the same
        // kind of suffix and is not: alibaba and alibaba-cn share 45 models
        // and disagree on 41, qwen-max among them at $1.60/$6.40 against
        // $0.345/$1.377. Folding these would misprice by 4.6x, so they must
        // stay distinct providers.
        assert_ne!(
            canonical_provider("alibaba-cn"),
            canonical_provider("alibaba")
        );
        assert_ne!(
            canonical_provider("siliconflow-cn"),
            canonical_provider("siliconflow")
        );
        assert_eq!(canonical_provider("alibaba-cn"), Some("alibaba_cn".into()));
    }

    #[test]
    fn provider_spelling_match_is_exact_where_canonicalization_is_not() {
        // canonical_provider folds the two spellings together; this predicate
        // deliberately does not, so `select_best_match` can prefer the row that
        // spells the vendor the way the hint does.
        assert_eq!(
            canonical_provider("deepseek-ai"),
            canonical_provider("deepseek")
        );

        assert!(matches_provider_spelling(
            "novita/deepseek/deepseek-r1-distill-qwen-32b",
            "deepseek"
        ));
        assert!(!matches_provider_spelling(
            "cloudflare/@cf/deepseek-ai/deepseek-r1-distill-qwen-32b",
            "deepseek"
        ));

        // Case and `-`/`_` are spelling noise, not a different spelling.
        for hint in ["deepseek-ai", "deepseek_ai", "DeepSeek-AI"] {
            assert!(
                matches_provider_spelling("hyperbolic/deepseek-ai/DeepSeek-V3", hint),
                "{hint} spells the vendor the way this key does"
            );
            assert!(!matches_provider_spelling(
                "novita/deepseek/deepseek-v3-0324",
                hint
            ));
        }

        // The last segment is the model name, never a vendor spelling.
        assert!(!matches_provider_spelling("deepseek-ai", "deepseek-ai"));
        assert!(!matches_provider_spelling(
            "some-vendor/deepseek",
            "deepseek"
        ));
    }

    #[test]
    fn provider_spelling_reads_the_dotted_prefix_of_the_final_key_component() {
        // AWS-style ids carry the provider in a dotted prefix of the final key
        // component, which is why `key_provider_tags` splits it on `.`. The
        // spelling predicate has to read the same segments, or a `deepseek`
        // hint fails to recognise the row that spells the vendor its way and
        // falls through to a differently spelled reseller.
        assert_eq!(
            key_provider_tags("amazon-bedrock/us.deepseek.r1-v1:0"),
            vec!["amazon_bedrock", "us", "deepseek"]
        );
        assert!(matches_provider_spelling(
            "amazon-bedrock/us.deepseek.r1-v1:0",
            "deepseek"
        ));
        assert!(matches_provider_spelling(
            "bedrock/us-east-1/deepseek.v3.2",
            "deepseek"
        ));
        assert!(!matches_provider_spelling(
            "amazon-bedrock/us.deepseek.r1-v1:0",
            "deepseek-ai"
        ));

        // The trailing piece is still the model name, not a vendor spelling,
        // and an undotted final component contributes nothing at all.
        assert!(!matches_provider_spelling(
            "amazon-bedrock/us.deepseek.r1-v1:0",
            "r1-v1:0"
        ));
        assert!(!matches_provider_spelling(
            "some-router/deepseek-ai",
            "deepseek-ai"
        ));
    }
}
