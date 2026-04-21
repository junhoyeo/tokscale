use ratatui::prelude::*;
use tokscale_core::ClientId;

use crate::tui::client_ui;
use crate::tui::config::TokscaleConfig;

pub fn format_tokens_compact(tokens: u64) -> String {
    if tokens >= 1_000_000_000 {
        format!("{:.1}B", tokens as f64 / 1_000_000_000.0)
    } else if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        format_tokens_with_commas(tokens)
    }
}

pub fn format_tokens(tokens: u64) -> String {
    format_tokens_compact(tokens)
}

pub fn format_tokens_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.insert(0, ',');
        }
        result.insert(0, c);
    }
    result
}

pub fn format_cost(cost: f64) -> String {
    if !cost.is_finite() || cost < 0.0 {
        return "$0.00".to_string();
    }
    if cost >= 1000.0 {
        format!("${:.1}K", cost / 1000.0)
    } else {
        format!("${:.2}", cost)
    }
}

/// Cache reuse multiplier: cached reads per full-price input token.
/// `cache_read / (input + cache_write)` — how many low-cost reads you
/// got for every token you paid full price (fresh input or cache write).
pub fn format_cache_hit_rate(cache_read: u64, input: u64, cache_write: u64) -> String {
    let paid = input.saturating_add(cache_write);
    if paid == 0 {
        return if cache_read > 0 {
            "∞".to_string()
        } else {
            "—".to_string()
        };
    }
    let ratio = cache_read as f64 / paid as f64;
    format!("{:.1}x", ratio)
}

pub fn get_model_color(model: &str) -> Color {
    get_provider_shade(get_provider_from_model(model), 0)
}

/// Returns the shade for a given `(provider, rank)` pair.
/// Honors `[colors.providers]` config overrides at every rank by deriving
/// a 7-step lighten-to-white palette from the override base color.
pub fn get_provider_shade(provider: &str, rank: usize) -> Color {
    if let Some(base) = TokscaleConfig::load().get_provider_color(provider) {
        return shade_from_base(base, rank);
    }
    let palette: &[(u8, u8, u8)] = match provider {
        "anthropic" => &ANTHROPIC_SHADES,
        "openai" => &OPENAI_SHADES,
        "google" => &GOOGLE_SHADES,
        "deepseek" => &DEEPSEEK_SHADES,
        "xai" => &XAI_SHADES,
        "meta" => &META_SHADES,
        "cursor" => &CURSOR_SHADES,
        _ => &UNKNOWN_SHADES,
    };
    let idx = rank.min(palette.len() - 1);
    let (r, g, b) = palette[idx];
    Color::Rgb(r, g, b)
}

/// Generates a 7-step monochromatic palette from `base` by interpolating
/// toward white. Factors roughly match the end-of-ramp lightness of the
/// hardcoded palettes so overrides feel visually consistent.
fn shade_from_base(base: Color, rank: usize) -> Color {
    const FACTORS: [f32; 7] = [0.00, 0.11, 0.22, 0.33, 0.44, 0.56, 0.67];
    let Color::Rgb(r, g, b) = base else {
        return base;
    };
    let idx = rank.min(FACTORS.len() - 1);
    let f = FACTORS[idx];
    let lerp = |c: u8| -> u8 {
        let c = c as f32;
        (c + (255.0 - c) * f).round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(lerp(r), lerp(g), lerp(b))
}

const ANTHROPIC_SHADES: [(u8, u8, u8); 7] = [
    (218, 119, 86),  // #DA7756
    (223, 136, 107), // #DF886B
    (227, 153, 128), // #E39980
    (232, 170, 149), // #E8AA95
    (236, 184, 166), // #ECB8A6
    (239, 197, 183), // #EFC5B7
    (243, 210, 199), // #F3D2C7
];

const OPENAI_SHADES: [(u8, u8, u8); 7] = [
    (16, 185, 129),  // #10B981
    (18, 208, 145),  // #12D091
    (20, 232, 162),  // #14E8A2
    (41, 236, 172),  // #29ECAC
    (61, 238, 179),  // #3DEEB3
    (97, 241, 193),  // #61F1C1
    (133, 244, 208), // #85F4D0
];

const GOOGLE_SHADES: [(u8, u8, u8); 7] = [
    (59, 130, 246),  // #3B82F6
    (83, 146, 247),  // #5392F7
    (108, 161, 248), // #6CA1F8
    (132, 177, 249), // #84B1F9
    (153, 190, 250), // #99BEFA
    (172, 202, 251), // #ACCAFB
    (190, 214, 252), // #BED6FC
];

const DEEPSEEK_SHADES: [(u8, u8, u8); 7] = [
    (6, 182, 212),   // #06B6D4
    (7, 203, 237),   // #07CBED
    (21, 215, 248),  // #15D7F8
    (45, 219, 249),  // #2DDBF9
    (66, 223, 250),  // #42DFFA
    (85, 226, 250),  // #55E2FA
    (105, 229, 251), // #69E5FB
];

const XAI_SHADES: [(u8, u8, u8); 7] = [
    (234, 179, 8),   // #EAB308
    (247, 192, 21),  // #F7C015
    (248, 199, 45),  // #F8C72D
    (249, 205, 70),  // #F9CD46
    (249, 211, 91),  // #F9D35B
    (250, 216, 110), // #FAD86E
    (251, 221, 129), // #FBDD81
];

const META_SHADES: [(u8, u8, u8); 7] = [
    (99, 102, 241),  // #6366F1
    (122, 125, 243), // #7A7DF3
    (146, 148, 245), // #9294F5
    (169, 171, 247), // #A9ABF7
    (189, 190, 249), // #BDBEF9
    (207, 208, 251), // #CFD0FB
    (225, 226, 252), // #E1E2FC
];

const CURSOR_SHADES: [(u8, u8, u8); 7] = [
    (139, 92, 246),  // #8B5CF6
    (154, 114, 247), // #9A72F7
    (169, 135, 248), // #A987F8
    (184, 156, 250), // #B89CFA
    (199, 177, 251), // #C7B1FB
    (215, 199, 252), // #D7C7FC
    (230, 220, 253), // #E6DCFD
];

/// Neutral gray ramp for providers that don't match any known palette.
/// Still produces distinct shades per rank instead of collapsing to white.
const UNKNOWN_SHADES: [(u8, u8, u8); 7] = [
    (136, 136, 136), // #888888
    (156, 156, 156), // #9C9C9C
    (176, 176, 176), // #B0B0B0
    (196, 196, 196), // #C4C4C4
    (212, 212, 212), // #D4D4D4
    (228, 228, 228), // #E4E4E4
    (244, 244, 244), // #F4F4F4
];

pub fn get_provider_from_model(model: &str) -> &'static str {
    let model_lower = model.to_lowercase();

    if model_lower.contains("claude")
        || model_lower.contains("sonnet")
        || model_lower.contains("opus")
        || model_lower.contains("haiku")
    {
        "anthropic"
    } else if model_lower.contains("gpt")
        || model_lower.starts_with("o1")
        || model_lower.starts_with("o3")
        || model_lower.contains("codex")
        || model_lower.contains("text-embedding")
        || model_lower.contains("dall-e")
        || model_lower.contains("whisper")
        || model_lower.contains("tts")
    {
        "openai"
    } else if model_lower.contains("gemini") {
        "google"
    } else if model_lower.contains("deepseek") {
        "deepseek"
    } else if model_lower.contains("grok") {
        "xai"
    } else if model_lower.contains("llama") {
        "meta"
    } else if model_lower.contains("mixtral") {
        "mistral"
    } else if model_lower == "auto" || model_lower.contains("cursor") {
        "cursor"
    } else {
        "unknown"
    }
}

pub fn get_client_color(client: &str) -> Color {
    let config = TokscaleConfig::load();
    if let Some(color) = config.get_client_color(client) {
        return color;
    }
    match client.to_lowercase().as_str() {
        "opencode" => Color::Rgb(34, 197, 94), // #22c55e
        "claude" => Color::Rgb(218, 119, 86),  // #DA7756 Claude brand coral
        "codex" => Color::Rgb(59, 130, 246),   // #3b82f6
        "cursor" => Color::Rgb(168, 85, 247),  // #a855f7
        "gemini" => Color::Rgb(6, 182, 212),   // #06b6d4
        "amp" => Color::Rgb(236, 72, 153),     // #EC4899
        "droid" => Color::Rgb(16, 185, 129),   // #10b981
        "openclaw" => Color::Rgb(239, 68, 68), // #ef4444
        "hermes" => Color::Rgb(255, 215, 0),   // #ffd700
        _ => Color::Rgb(136, 136, 136),        // #888888
    }
}

pub fn get_client_display_name(client: &str) -> String {
    let config = TokscaleConfig::load();
    if let Some(name) = config.get_client_display_name(client) {
        return name.to_string();
    }
    let client_lower = client.to_lowercase();
    if client_lower == ClientId::OpenClaw.as_str() {
        return "🦞 OpenClaw".to_string();
    }
    if let Some(client_id) = ClientId::from_str(&client_lower) {
        return client_ui::display_name(client_id).to_string();
    }
    client.to_string()
}

pub fn get_provider_display_name(provider: &str) -> String {
    let config = TokscaleConfig::load();
    if let Some(name) = config.get_provider_display_name(provider) {
        return name.to_string();
    }
    match provider.to_lowercase().as_str() {
        "anthropic" => "Anthropic".to_string(),
        "openai" => "OpenAI".to_string(),
        "google" => "Google".to_string(),
        "cursor" => "Cursor".to_string(),
        "deepseek" => "DeepSeek".to_string(),
        "xai" => "xAI".to_string(),
        "meta" => "Meta".to_string(),
        "mistral" => "Mistral".to_string(),
        "cohere" => "Cohere".to_string(),
        "opencode" => "OpenCode".to_string(),
        s if s.starts_with("github-cop") || s.contains("copilot") => "GitHub Copilot".to_string(),
        _ => capitalize_first(provider),
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shade_from_base_rank_0_equals_base() {
        let base = Color::Rgb(255, 0, 0);
        assert_eq!(shade_from_base(base, 0), base);
    }

    #[test]
    fn shade_from_base_lightens_monotonically_toward_white() {
        let base = Color::Rgb(0, 0, 0);
        let mut prev_r: u8 = 0;
        for rank in 0..7 {
            let Color::Rgb(r, _, _) = shade_from_base(base, rank) else {
                panic!("expected Rgb")
            };
            assert!(
                r >= prev_r,
                "shade at rank {} should not be darker than rank {}",
                rank,
                rank - 1
            );
            prev_r = r;
        }
    }

    #[test]
    fn shade_from_base_clamps_beyond_palette_length() {
        let base = Color::Rgb(100, 100, 100);
        // Rank beyond FACTORS.len() saturates to the lightest shade.
        assert_eq!(shade_from_base(base, 100), shade_from_base(base, 6));
    }

    #[test]
    fn shade_from_base_passes_through_non_rgb() {
        // Indexed terminal colors can't be lightened channel-wise — return as-is.
        assert_eq!(shade_from_base(Color::Indexed(42), 5), Color::Indexed(42));
    }

    #[test]
    fn unknown_provider_returns_gray_ramp_not_pure_white() {
        // Regression: the old fallback was [(255,255,255)], collapsing all
        // unknown-provider models to pure white. Now each rank is a distinct
        // gray so models stay distinguishable.
        let rank_0 = get_provider_shade("some-new-provider", 0);
        let rank_3 = get_provider_shade("some-new-provider", 3);
        assert_ne!(rank_0, rank_3);
        assert_ne!(rank_0, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn cursor_provider_has_distinct_shades_per_rank() {
        // Regression: CURSOR_SHADES used to be a single-entry palette so all
        // Cursor models collapsed to one color.
        let rank_0 = get_provider_shade("cursor", 0);
        let rank_6 = get_provider_shade("cursor", 6);
        assert_ne!(rank_0, rank_6);
    }

    #[test]
    fn get_provider_shade_saturates_at_palette_end() {
        let last = get_provider_shade("anthropic", 6);
        let past_end = get_provider_shade("anthropic", 99);
        assert_eq!(last, past_end);
    }
}
