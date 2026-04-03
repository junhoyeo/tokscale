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

pub fn get_model_color(model: &str) -> Color {
    let provider = get_provider_from_model(model);
    let config = TokscaleConfig::load();
    if let Some(color) = config.get_provider_color(provider) {
        return color;
    }
    match provider {
        "anthropic" => Color::Rgb(218, 119, 86), // #DA7756 Claude brand coral
        "openai" => Color::Rgb(16, 185, 129),    // #10B981
        "google" => Color::Rgb(59, 130, 246),    // #3B82F6
        "cursor" => Color::Rgb(139, 92, 246),    // #8B5CF6
        "deepseek" => Color::Rgb(6, 182, 212),   // #06B6D4
        "xai" => Color::Rgb(234, 179, 8),        // #EAB308
        "meta" => Color::Rgb(99, 102, 241),      // #6366F1
        _ => Color::Rgb(255, 255, 255),          // #FFFFFF (unknown)
    }
}

fn get_provider_from_model(model: &str) -> &'static str {
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
