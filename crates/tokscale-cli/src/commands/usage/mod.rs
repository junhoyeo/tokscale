mod amp;
mod claude;
pub mod codex;
mod copilot;
mod grok;
pub mod helpers;
mod kimi;
mod minimax;
mod minimax_tokenplan;
mod sakana;
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
pub struct UsageResetCredits {
    pub available_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credits: Vec<UsageResetCredit>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageResetCredit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageCreditStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_credits: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlimited: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overage_limit_reached: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageSpendControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub individual_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reached: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageOutput {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<UsageAccount>,
    pub plan: Option<String>,
    pub email: Option<String>,
    pub metrics: Vec<UsageMetric>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits: Option<UsageResetCredits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_status: Option<UsageCreditStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_control: Option<UsageSpendControl>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageAccount {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub is_active: bool,
}

impl UsageAccount {
    pub fn label_name(&self) -> Option<&str> {
        self.label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
    }

    pub fn short_id(&self) -> String {
        let id = self.id.trim();
        if id.is_empty() {
            return "unknown".to_string();
        }

        let char_count = id.chars().count();
        if char_count <= 12 {
            return id.to_string();
        }

        let head: String = id.chars().take(6).collect();
        let tail: String = id
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}...{tail}")
    }

    pub fn display_name(&self) -> String {
        self.label_name()
            .map(str::to_string)
            .unwrap_or_else(|| format!("Account {}", self.short_id()))
    }
}

impl UsageOutput {
    pub fn account_display_name(&self) -> Option<String> {
        let account = self.account.as_ref()?;

        if let Some(label) = account.label_name() {
            return Some(label.to_string());
        }

        if let Some(email) = self
            .email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(email.to_string());
        }

        Some(account.display_name())
    }

    pub fn display_name(&self) -> String {
        match &self.account {
            Some(_) => format!(
                "{} ({})",
                self.provider,
                self.account_display_name().unwrap_or_default()
            ),
            None => self.provider.clone(),
        }
    }
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

#[derive(Clone, Copy)]
enum Fetch {
    Single(fn() -> Result<UsageOutput>),
    Multi(fn() -> Result<Vec<UsageOutput>>),
}

impl Fetch {
    fn call(self) -> Result<Vec<UsageOutput>> {
        match self {
            Fetch::Single(fetch) => fetch().map(|output| vec![output]),
            Fetch::Multi(fetch) => fetch(),
        }
    }
}

type UsageProvider = (&'static str, fn() -> bool, Fetch);

/// A provider that is active (has credentials) but whose fetch failed.
///
/// `name` is the human-facing provider label and `error` is the formatted
/// error message (e.g. sakana's "refresh SAKANA_SESSION_COOKIE" guidance).
#[derive(Debug, Clone)]
pub struct ProviderError {
    pub name: &'static str,
    pub error: String,
}

/// Backwards-compatible entry point: returns only successful provider outputs.
///
/// Per-provider errors are silently discarded here. Callers that need to make
/// failures visible (e.g. the CLI `run`) should use [`fetch_all_with_errors`].
///
/// Used by the TUI dashboard (non-test builds only); the TUI test build stubs
/// the fetch out, so allow it to be unused there.
#[cfg_attr(test, allow(dead_code))]
pub fn fetch_all() -> Vec<UsageOutput> {
    fetch_all_with_errors().0
}

/// Fetch usage for every active provider in parallel, returning both the
/// successful outputs and the per-provider errors.
///
/// Previously a provider whose `fetch` returned `Err` (notably a stale/expired
/// session-cookie auth error) was silently dropped: `has_credentials()` reports
/// the provider as active, yet it just vanished from the output. This collects
/// those errors so the caller can surface them to the user instead.
pub fn fetch_all_with_errors() -> (Vec<UsageOutput>, Vec<ProviderError>) {
    let providers: Vec<UsageProvider> = vec![
        (
            "Claude",
            claude::has_credentials,
            Fetch::Single(claude::fetch),
        ),
        (
            "Codex",
            codex::has_credentials,
            Fetch::Multi(codex::fetch_all),
        ),
        ("Z.ai", zai::has_credentials, Fetch::Single(zai::fetch)),
        ("Amp", amp::has_credentials, Fetch::Single(amp::fetch)),
        (
            "Copilot",
            copilot::has_credentials,
            Fetch::Single(copilot::fetch),
        ),
        (
            "Grok Build",
            grok::has_credentials,
            Fetch::Single(grok::fetch),
        ),
        ("Kimi", kimi::has_credentials, Fetch::Single(kimi::fetch)),
        (
            "MiniMax",
            minimax::has_credentials,
            Fetch::Single(minimax::fetch),
        ),
        (
            "MiniMax Token Plan",
            minimax_tokenplan::has_credentials,
            Fetch::Multi(minimax_tokenplan::fetch_all),
        ),
        ("Warp/Oz", warp::has_credentials, Fetch::Single(warp::fetch)),
        (
            "Sakana",
            sakana::has_credentials,
            Fetch::Single(sakana::fetch),
        ),
    ];

    let active: Vec<_> = providers.into_iter().filter(|(_, has, _)| has()).collect();

    if active.is_empty() {
        return (vec![], vec![]);
    }

    let results = std::thread::scope(|s| {
        let handles: Vec<_> = active
            .into_iter()
            .map(|(name, _, fetch)| s.spawn(move || (name, fetch.call())))
            .collect();

        handles
            .into_iter()
            .filter_map(|handle| {
                // A panicked provider thread should not take down the whole
                // command; skip it (a join error has no message to surface).
                handle.join().ok()
            })
            .collect::<Vec<_>>()
    });

    partition_results(results)
}

/// Split per-provider fetch results into (successful outputs, errors).
///
/// An active provider returning `Err` becomes a [`ProviderError`] rather than
/// being silently dropped.
fn partition_results(
    results: Vec<(&'static str, Result<Vec<UsageOutput>>)>,
) -> (Vec<UsageOutput>, Vec<ProviderError>) {
    let mut outputs = Vec::new();
    let mut errors = Vec::new();
    for (name, result) in results {
        match result {
            Ok(mut provider_outputs) => outputs.append(&mut provider_outputs),
            Err(err) => errors.push(ProviderError {
                name,
                error: err.to_string(),
            }),
        }
    }
    (outputs, errors)
}

// ── Light-mode rendering ──

const BAR_WIDTH: usize = 12;
const METRIC_LABEL_WIDTH: usize = 14;
const METRIC_REMAINING_WIDTH: usize = 11;
const METRIC_BAR_WIDTH: usize = BAR_WIDTH + 2;
const METRIC_RESET_WIDTH: usize = 24;
const CARD_WIDTH: usize =
    1 + METRIC_LABEL_WIDTH + METRIC_REMAINING_WIDTH + METRIC_BAR_WIDTH + METRIC_RESET_WIDTH;

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
    println!(
        "│ {:<width$}│",
        output.display_name(),
        width = CARD_WIDTH - 1
    );
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
        let reset = truncate(&reset, METRIC_RESET_WIDTH);
        let label = truncate(&m.label, METRIC_LABEL_WIDTH);
        println!(
            "│ {:<label_width$}{:<remaining_width$}{:<bar_width$}{:<reset_width$}│",
            label,
            rem,
            bar,
            reset,
            label_width = METRIC_LABEL_WIDTH,
            remaining_width = METRIC_REMAINING_WIDTH,
            bar_width = METRIC_BAR_WIDTH,
            reset_width = METRIC_RESET_WIDTH,
        );
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
    if let Some(ref credits) = output.reset_credits {
        println!(
            "│ {:<10}{:<width$}│",
            "Resets",
            format!("{} available", credits.available_count),
            width = CARD_WIDTH - 11
        );
    }
    println!("╰{}╯", "─".repeat(CARD_WIDTH));
}

pub fn run(json: bool, _light: bool) -> Result<()> {
    let (outputs, errors) = fetch_all_with_errors();
    if json {
        // Keep stdout pure JSON: do NOT emit provider warnings here, since they
        // would corrupt downstream `--json` consumers that read stderr too.
        println!("{}", serde_json::to_string_pretty(&outputs)?);
    } else {
        for o in &outputs {
            render_light(o);
        }
        // Surface active-but-failed providers (e.g. an expired session cookie)
        // so they don't silently vanish from the output. One concise line per
        // failing provider, on stderr to keep stdout clean.
        for err in &errors {
            eprintln!("{}: {} — skipped", err.name, err.error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_output_display_name_includes_account_label() {
        let output = UsageOutput {
            provider: "Codex".to_string(),
            account: Some(UsageAccount {
                id: "acct_123".to_string(),
                label: Some("work".to_string()),
                is_active: true,
            }),
            plan: None,
            email: None,
            metrics: Vec::new(),
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        };

        assert_eq!(output.display_name(), "Codex (work)");
    }

    #[test]
    fn usage_output_display_name_prefers_email_over_account_id() {
        let output = UsageOutput {
            provider: "Codex".to_string(),
            account: Some(UsageAccount {
                id: "acct_123".to_string(),
                label: Some("  ".to_string()),
                is_active: false,
            }),
            plan: None,
            email: Some("user@example.com".to_string()),
            metrics: Vec::new(),
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        };

        assert_eq!(output.display_name(), "Codex (user@example.com)");
    }

    #[test]
    fn usage_output_display_name_masks_long_account_id() {
        let output = UsageOutput {
            provider: "Codex".to_string(),
            account: Some(UsageAccount {
                id: "123e4567-e89b-12d3-a456-426614174000".to_string(),
                label: None,
                is_active: false,
            }),
            plan: None,
            email: None,
            metrics: Vec::new(),
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        };

        assert_eq!(output.display_name(), "Codex (Account 123e45...4000)");
    }

    fn sample_output(provider: &str) -> UsageOutput {
        UsageOutput {
            provider: provider.to_string(),
            account: None,
            plan: None,
            email: None,
            metrics: Vec::new(),
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        }
    }

    #[test]
    fn partition_results_surfaces_provider_errors_instead_of_dropping_them() {
        let results: Vec<(&'static str, Result<Vec<UsageOutput>>)> = vec![
            ("Claude", Ok(vec![sample_output("Claude")])),
            (
                "Sakana",
                Err(anyhow::anyhow!(
                    "Sakana session expired or invalid. Refresh SAKANA_SESSION_COOKIE."
                )),
            ),
            (
                "Codex",
                Ok(vec![sample_output("Codex"), sample_output("Codex")]),
            ),
        ];

        let (outputs, errors) = partition_results(results);

        // Successful providers are preserved (including a Multi provider's
        // several outputs), in order.
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].provider, "Claude");
        assert_eq!(outputs[1].provider, "Codex");
        assert_eq!(outputs[2].provider, "Codex");

        // The failing provider's error is surfaced, not silently discarded.
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].name, "Sakana");
        assert!(
            errors[0].error.contains("SAKANA_SESSION_COOKIE"),
            "expected the auth-refresh guidance to be preserved, got: {}",
            errors[0].error
        );
    }

    #[test]
    fn partition_results_reports_no_errors_when_all_succeed() {
        let results: Vec<(&'static str, Result<Vec<UsageOutput>>)> =
            vec![("Claude", Ok(vec![sample_output("Claude")]))];

        let (outputs, errors) = partition_results(results);

        assert_eq!(outputs.len(), 1);
        assert!(errors.is_empty());
    }

    #[test]
    fn usage_output_deserializes_legacy_json_without_account() -> Result<()> {
        let output: UsageOutput = serde_json::from_str(
            r#"{
                "provider": "Codex",
                "plan": null,
                "email": null,
                "metrics": []
            }"#,
        )?;

        assert!(output.account.is_none());
        assert_eq!(output.display_name(), "Codex");
        Ok(())
    }
}
