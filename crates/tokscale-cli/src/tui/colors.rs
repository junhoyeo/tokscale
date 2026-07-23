use std::collections::HashMap;

use ratatui::style::Color;

use super::data::ModelUsage;
use super::ui::widgets::{get_provider_from_model, get_provider_shade, provider_has_palette};

pub fn model_shade_key(provider: &str, model: &str) -> String {
    format!("{provider}\0{model}")
}

/// Builds a `(provider, model) -> Color` map where each provider's models are
/// ranked into the provider's shade ramp; rank 0 gets the base provider color
/// and later ranks get progressively lighter shades.
///
/// Ranking encodes the model hierarchy, not spend: family tier first (for
/// Anthropic: fable > opus > sonnet > haiku), then version (newer = darker),
/// then cost, then model name. Cost is only a tiebreaker so that variants of
/// the same version (e.g. `-thinking`, `-high`) get stable adjacent shades.
///
/// Aggregates cost per (provider, model) so the same model appearing in
/// multiple group-by buckets (e.g. `GroupBy::WorkspaceModel`) doesn't inflate
/// the rank count. Remaining ties are resolved by model name so shade
/// assignment stays deterministic across refreshes.
pub fn build_model_shade_map(models: &[ModelUsage]) -> HashMap<String, Color> {
    let mut by_provider: HashMap<&str, HashMap<&str, f64>> = HashMap::new();
    for m in models {
        let provider = provider_color_key(&m.provider, &m.color_key);
        let cost = if m.cost.is_finite() { m.cost } else { 0.0 };
        *by_provider
            .entry(provider)
            .or_default()
            .entry(m.color_key.as_str())
            .or_insert(0.0) += cost;
    }

    let mut map = HashMap::new();
    for (provider, models_map) in by_provider {
        let mut ranked: Vec<(&str, f64)> = models_map.into_iter().collect();
        ranked.sort_by(|a, b| {
            family_tier(provider, a.0)
                .cmp(&family_tier(provider, b.0))
                .then_with(|| model_version(b.0).cmp(&model_version(a.0)))
                .then_with(|| b.1.total_cmp(&a.1))
                .then_with(|| a.0.cmp(b.0))
        });
        for (rank, (name, _)) in ranked.iter().enumerate() {
            map.insert(
                model_shade_key(provider, name),
                get_provider_shade(provider, rank),
            );
        }
    }
    map
}

/// Resolves the provider whose color ramp a model's name should render in.
///
/// Empty or mixed (`", "`-joined) providers color by the model's own vendor.
/// So do gateway providers that resell other vendors' models (e.g.
/// `github-copilot`, `openrouter`): they have no vendor palette of their own, so
/// a Claude model served through Copilot still gets the Anthropic ramp instead
/// of the neutral "unknown" gray. Both the shade-map build and the per-cell
/// lookup route through this, so their keys always agree.
pub(crate) fn provider_color_key<'a>(provider: &'a str, model: &'a str) -> &'a str {
    if provider.is_empty() || provider.contains(", ") || !provider_has_palette(provider) {
        get_provider_from_model(model)
    } else {
        provider
    }
}

/// Position of a model's family within its provider's lineup; lower tiers get
/// darker shades. Only Anthropic has a known hierarchy — other providers rank
/// purely by version and cost.
fn family_tier(provider: &str, model: &str) -> u8 {
    if !provider.to_lowercase().contains("anthropic") {
        return 0;
    }
    let lower = model.to_lowercase();
    // "fable" is matched as a delimited token (mirrors get_provider_from_model)
    // so ids like "unfabled-x" don't land in the flagship tier.
    if lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token == "fable")
    {
        0
    } else if lower.contains("opus") {
        1
    } else if lower.contains("sonnet") {
        2
    } else if lower.contains("haiku") {
        3
    } else {
        4
    }
}

/// Extracts a `(major, minor)` version from a model id:
/// "claude-opus-4-6" -> (4, 6), "gpt-5.4" -> (5, 4), "gpt-4o" -> (4, 0),
/// "claude-fable-5" -> (5, 0). Tokens only need to *start* with digits, so
/// alphanumeric versions like "4o" parse as their numeric prefix. Scanning
/// stops at the first 4+ digit value (dates like 20241022) so ids such as
/// "o1-2024-12-17" don't misparse a date fragment as a version; ids without
/// a version yield (0, 0).
fn model_version(model: &str) -> (u32, u32) {
    let tokens: Vec<&str> = model
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    for (i, token) in tokens.iter().enumerate() {
        let Some(major) = leading_number(token) else {
            continue;
        };
        if major >= 1000 {
            return (0, 0);
        }
        // Minor must be fully numeric: suffix tokens like "1m" (from a
        // "[1m]" context-window marker) are not version fragments.
        let minor = tokens
            .get(i + 1)
            .and_then(|t| t.parse::<u32>().ok())
            .filter(|&m| m < 1000)
            .unwrap_or(0);
        return (major, minor);
    }
    (0, 0)
}

/// Parses the leading digit run of a token: "4o" -> Some(4), "5" -> Some(5),
/// "turbo" -> None.
fn leading_number(token: &str) -> Option<u32> {
    let end = token
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(token.len(), |(i, _)| i);
    token[..end].parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{ModelUsage, TokenBreakdown};

    fn usage(model: &str, provider: &str, cost: f64) -> ModelUsage {
        ModelUsage {
            model: model.to_string(),
            color_key: model.to_string(),
            provider: provider.to_string(),
            client: "claude".to_string(),
            workspace_key: None,
            workspace_label: None,
            tokens: TokenBreakdown::default(),
            cost,
            performance: Default::default(),
            session_count: 1,
        }
    }

    fn shade(map: &HashMap<String, Color>, provider: &str, model: &str) -> Color {
        map.get(&model_shade_key(provider, model)).copied().unwrap()
    }

    #[test]
    fn provider_color_key_routes_gateways_to_model_vendor() {
        // Known vendor providers keep their own identity.
        assert_eq!(
            provider_color_key("anthropic", "claude-opus-4-8"),
            "anthropic"
        );
        assert_eq!(provider_color_key("openai", "gpt-5.4"), "openai");
        // Gateway providers with no vendor palette color by the model's vendor.
        assert_eq!(
            provider_color_key("github-copilot", "claude-fable-5"),
            "anthropic"
        );
        assert_eq!(
            provider_color_key("github-copilot", "gpt-5.3-codex"),
            "openai"
        );
        assert_eq!(provider_color_key("fireworks_ai", "GLM 5.2"), "zai");
        assert_eq!(
            provider_color_key("fireworks_ai", "Kimi K2.7 Code"),
            "moonshotai"
        );
        // Empty / mixed providers also defer to the model.
        assert_eq!(provider_color_key("", "claude-fable-5"), "anthropic");
        assert_eq!(
            provider_color_key("anthropic, github-copilot", "claude-fable-5"),
            "anthropic"
        );
    }

    #[test]
    fn gateway_and_native_claude_share_one_shade_bucket() {
        // Regression: the same Claude model served natively and via the
        // github-copilot gateway must fold into one Anthropic-ramp bucket and
        // rank by family tier, not split into a separate "unknown" gray key.
        let map = build_model_shade_map(&[
            usage("claude-fable-5", "github-copilot", 3.0),
            usage("claude-opus-4-8", "anthropic", 100.0),
        ]);
        assert_eq!(
            shade(&map, "anthropic", "claude-fable-5"),
            get_provider_shade("anthropic", 0),
            "copilot fable takes the flagship Anthropic shade"
        );
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-8"),
            get_provider_shade("anthropic", 1),
            "opus ranks below fable in the shared bucket"
        );
    }

    #[test]
    fn model_version_parses_common_id_shapes() {
        assert_eq!(model_version("claude-opus-4-6"), (4, 6));
        assert_eq!(model_version("claude-4-5-opus-high-thinking"), (4, 5));
        assert_eq!(model_version("claude-fable-5"), (5, 0));
        assert_eq!(model_version("claude-fable-5[1m]"), (5, 0));
        assert_eq!(model_version("gpt-5.4"), (5, 4));
        assert_eq!(model_version("claude-3-5-sonnet-20241022"), (3, 5));
        assert_eq!(model_version("gpt-4-turbo"), (4, 0));
        // Alphanumeric version tokens parse by their numeric prefix.
        assert_eq!(model_version("gpt-4o"), (4, 0));
        assert_eq!(model_version("gpt-4o-mini"), (4, 0));
        assert_eq!(model_version("gpt-3.5-turbo"), (3, 5));
        // A "[1m]" context marker is not a minor version.
        assert_eq!(model_version("gpt-4o[1m]"), (4, 0));
        // Date fragments must not be read as versions.
        assert_eq!(model_version("o1-2024-12-17"), (0, 0));
        assert_eq!(model_version("codex-mini-latest"), (0, 0));
    }

    #[test]
    fn fable_outranks_higher_cost_opus() {
        // Fable is the flagship tier: it takes the base shade even when the
        // user has spent far more on Opus models.
        let map = build_model_shade_map(&[
            usage("claude-opus-4-6", "anthropic", 900.0),
            usage("claude-fable-5", "anthropic", 1.0),
        ]);
        assert_eq!(
            shade(&map, "anthropic", "claude-fable-5"),
            get_provider_shade("anthropic", 0)
        );
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-6"),
            get_provider_shade("anthropic", 1)
        );
    }

    #[test]
    fn newer_opus_outranks_older_despite_lower_cost() {
        let map = build_model_shade_map(&[
            usage("claude-opus-4-5", "anthropic", 500.0),
            usage("claude-opus-4-7", "anthropic", 10.0),
            usage("claude-opus-4-6", "anthropic", 300.0),
        ]);
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-7"),
            get_provider_shade("anthropic", 0)
        );
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-6"),
            get_provider_shade("anthropic", 1)
        );
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-5"),
            get_provider_shade("anthropic", 2)
        );
    }

    #[test]
    fn family_tier_beats_version_within_anthropic() {
        // A newer Sonnet never renders darker than an older Opus.
        let map = build_model_shade_map(&[
            usage("claude-sonnet-5", "anthropic", 800.0),
            usage("claude-opus-4-1", "anthropic", 2.0),
        ]);
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-1"),
            get_provider_shade("anthropic", 0)
        );
        assert_eq!(
            shade(&map, "anthropic", "claude-sonnet-5"),
            get_provider_shade("anthropic", 1)
        );
    }

    #[test]
    fn variant_suffixes_group_with_their_base_version() {
        // Same version, different variants: cost then name break the tie, so
        // the 4-5 family occupies adjacent shades below 4-6.
        let map = build_model_shade_map(&[
            usage("claude-opus-4-5-thinking-high", "anthropic", 50.0),
            usage("claude-opus-4-6", "anthropic", 10.0),
            usage("claude-opus-4-5", "anthropic", 100.0),
        ]);
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-6"),
            get_provider_shade("anthropic", 0)
        );
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-5"),
            get_provider_shade("anthropic", 1)
        );
        assert_eq!(
            shade(&map, "anthropic", "claude-opus-4-5-thinking-high"),
            get_provider_shade("anthropic", 2)
        );
    }

    #[test]
    fn non_anthropic_providers_rank_by_version_first() {
        let map = build_model_shade_map(&[
            usage("gpt-4", "openai", 700.0),
            usage("gpt-5.4", "openai", 5.0),
        ]);
        assert_eq!(
            shade(&map, "openai", "gpt-5.4"),
            get_provider_shade("openai", 0)
        );
        assert_eq!(
            shade(&map, "openai", "gpt-4"),
            get_provider_shade("openai", 1)
        );
    }

    #[test]
    fn alphanumeric_versions_outrank_older_numeric_ones() {
        // Regression (PR #810 review): "4o" used to parse as no version at
        // all, letting gpt-3.5-turbo render darker than gpt-4o.
        let map = build_model_shade_map(&[
            usage("gpt-3.5-turbo", "openai", 900.0),
            usage("gpt-4o", "openai", 1.0),
        ]);
        assert_eq!(
            shade(&map, "openai", "gpt-4o"),
            get_provider_shade("openai", 0)
        );
        assert_eq!(
            shade(&map, "openai", "gpt-3.5-turbo"),
            get_provider_shade("openai", 1)
        );
    }
}
