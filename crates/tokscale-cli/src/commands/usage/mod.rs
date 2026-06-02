mod amp;
mod claude;
mod codex;
mod copilot;
mod grok;
pub mod helpers;
mod kimi;
mod minimax;
mod warp;
mod zai;

use anyhow::Result;

// ── Shared types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageMetric {
    pub label: String,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub remaining_label: Option<String>,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageOutput {
    pub provider: String,
    pub plan: Option<String>,
    pub email: Option<String>,
    pub metrics: Vec<UsageMetric>,
}

// ── Cache ──

fn cache_path() -> Option<std::path::PathBuf> {
    let dir = crate::paths::get_cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    Some(dir.join("subscription-usage-cache.json"))
}

pub fn save_cache(data: &[UsageOutput]) {
    let Some(path) = cache_path() else { return };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let json = serde_json::json!({
        "timestamp": timestamp,
        "data": data,
    });
    let _ = std::fs::write(&path, serde_json::to_string(&json).unwrap_or_default());
}

pub fn clear_cache() {
    if let Some(path) = cache_path() {
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg_attr(test, allow(dead_code))]
pub fn load_cache() -> Option<Vec<UsageOutput>> {
    let path = cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&content).ok()?;
    let timestamp = doc.get("timestamp")?.as_u64()?;
    let age = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(timestamp);
    // Cache expires after 5 minutes
    if age > 300 {
        return None;
    }
    serde_json::from_value(doc.get("data")?.clone()).ok()
}

// ── Public API ──

type UsageProvider = (&'static str, fn() -> bool, fn() -> Result<UsageOutput>);

pub fn fetch_all() -> Vec<UsageOutput> {
    let providers: Vec<UsageProvider> = vec![
        ("Claude", claude::has_credentials, claude::fetch),
        ("Codex", codex::has_credentials, codex::fetch),
        ("Z.ai", zai::has_credentials, zai::fetch),
        ("Amp", amp::has_credentials, amp::fetch),
        ("Copilot", copilot::has_credentials, copilot::fetch),
        ("Grok Build", grok::has_credentials, grok::fetch),
        ("Kimi", kimi::has_credentials, kimi::fetch),
        ("MiniMax", minimax::has_credentials, minimax::fetch),
        ("Warp/Oz", warp::has_credentials, warp::fetch),
    ];

    let active: Vec<_> = providers.into_iter().filter(|(_, has, _)| has()).collect();

    if active.is_empty() {
        return vec![];
    }

    std::thread::scope(|s| {
        active
            .into_iter()
            .map(|(_, _, fetch)| s.spawn(move || fetch().ok()))
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|h| h.join().ok().flatten())
            .collect()
    })
}

// ── Light-mode rendering ──

const BAR_WIDTH: usize = 12;
const CARD_WIDTH: usize = 62;

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_len - 1).collect();
    format!("{truncated}…")
}

fn render_light(output: &UsageOutput) {
    println!("╭{}╮", "─".repeat(CARD_WIDTH));
    // Provider header
    println!("│ {:<width$}│", output.provider, width = CARD_WIDTH - 1);
    for m in &output.metrics {
        let rem = m
            .remaining_label
            .clone()
            .unwrap_or_else(|| format!("{:.0}% left", m.remaining_percent));
        let rem = truncate(&rem, 11);
        let bar = helpers::render_ascii_bar(m.remaining_percent, BAR_WIDTH);
        let reset = m
            .resets_at
            .as_ref()
            .map(|r| helpers::format_reset_time(r))
            .unwrap_or_default();
        let label = truncate(&m.label, 14);
        println!("│ {:<14}{:<11}{:<14}{:<22}│", label, rem, bar, reset);
    }
    if let Some(ref email) = output.email {
        let email = truncate(email, CARD_WIDTH - 11);
        println!(
            "│ {:<10}{:<width$}│",
            "Account",
            email,
            width = CARD_WIDTH - 11
        );
    }
    if let Some(ref plan) = output.plan {
        let plan = truncate(plan, CARD_WIDTH - 11);
        println!("│ {:<10}{:<width$}│", "Plan", plan, width = CARD_WIDTH - 11);
    }
    println!("╰{}╯", "─".repeat(CARD_WIDTH));
}

pub fn run(json: bool, _light: bool) -> Result<()> {
    let outputs = fetch_all();
    if json {
        println!("{}", serde_json::to_string_pretty(&outputs)?);
    } else {
        for o in &outputs {
            render_light(o);
        }
    }
    Ok(())
}
