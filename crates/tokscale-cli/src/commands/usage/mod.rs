#![cfg_attr(test, allow(dead_code))]

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
#[cfg(test)]
mod test_server;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_source: Option<String>,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageFetchDiagnostic {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<UsageAccount>,
    #[serde(default)]
    pub kind: UsageFetchDiagnosticKind,
    #[serde(default)]
    pub severity: UsageFetchDiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageFetchDiagnosticKind {
    #[default]
    FetchFailed,
    ImportCurrentLoginFailed,
    ProviderPanicked,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageFetchDiagnosticSeverity {
    Info,
    Warning,
    #[default]
    Error,
}

impl UsageFetchDiagnostic {
    pub fn new(
        provider: impl Into<String>,
        account: Option<UsageAccount>,
        message: impl Into<String>,
    ) -> Self {
        Self::with_kind(
            provider,
            account,
            UsageFetchDiagnosticKind::FetchFailed,
            UsageFetchDiagnosticSeverity::Error,
            message,
        )
    }

    pub fn with_kind(
        provider: impl Into<String>,
        account: Option<UsageAccount>,
        kind: UsageFetchDiagnosticKind,
        severity: UsageFetchDiagnosticSeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            account,
            kind,
            severity,
            message: message.into(),
        }
    }

    pub fn display_name(&self) -> String {
        match &self.account {
            Some(account) => format!("{} ({})", self.provider, account.display_name()),
            None => self.provider.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct UsageFetchReport {
    pub outputs: Vec<UsageOutput>,
    pub diagnostics: Vec<UsageFetchDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageFetchIntent {
    CliReadOnly,
    TuiSurface,
}

impl UsageFetchReport {
    fn from_outputs(outputs: Vec<UsageOutput>) -> Self {
        Self {
            outputs,
            diagnostics: Vec::new(),
        }
    }

    fn from_error(
        provider: &'static str,
        account: Option<UsageAccount>,
        error: anyhow::Error,
    ) -> Self {
        Self {
            outputs: Vec::new(),
            diagnostics: vec![UsageFetchDiagnostic::new(
                provider,
                account,
                error.to_string(),
            )],
        }
    }

    fn extend(&mut self, other: UsageFetchReport) {
        self.outputs.extend(other.outputs);
        self.diagnostics.extend(other.diagnostics);
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

fn usage_providers(codex_fetch: Fetch) -> Vec<UsageProvider> {
    vec![
        (
            "Claude",
            claude::has_credentials,
            Fetch::Single(claude::fetch),
        ),
        ("Codex", codex::has_credentials, codex_fetch),
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
    ]
}

fn fetch_provider_report(
    provider: &'static str,
    result: Result<Vec<UsageOutput>>,
) -> UsageFetchReport {
    match result {
        Ok(outputs) => UsageFetchReport::from_outputs(outputs),
        Err(error) => UsageFetchReport::from_error(provider, None, error),
    }
}

pub fn fetch_all_report_with_intent(intent: UsageFetchIntent) -> UsageFetchReport {
    let codex_fetch = match intent {
        UsageFetchIntent::CliReadOnly => codex::fetch_all_report,
        UsageFetchIntent::TuiSurface => codex::fetch_all_report_importing_current_auth,
    };
    fetch_all_report_with_codex(codex_fetch)
}

fn fetch_all_report_with_codex(codex_fetch: fn() -> UsageFetchReport) -> UsageFetchReport {
    let active: Vec<_> = usage_providers(Fetch::Multi(codex::fetch_all))
        .into_iter()
        .filter(|(_, has, _)| has())
        .collect();

    if active.is_empty() {
        return UsageFetchReport::default();
    }

    std::thread::scope(|scope| {
        let handles = active
            .into_iter()
            .map(|(provider, _, fetch)| {
                let handle = if provider == "Codex" {
                    scope.spawn(codex_fetch)
                } else {
                    scope.spawn(move || fetch_provider_report(provider, fetch.call()))
                };
                (provider, handle)
            })
            .collect::<Vec<_>>();

        let mut report = UsageFetchReport::default();
        for (provider, handle) in handles {
            match handle.join() {
                Ok(provider_report) => report.extend(provider_report),
                Err(_) => report.diagnostics.push(UsageFetchDiagnostic::with_kind(
                    provider,
                    None,
                    UsageFetchDiagnosticKind::ProviderPanicked,
                    UsageFetchDiagnosticSeverity::Error,
                    "usage fetch worker panicked",
                )),
            }
        }
        report
    })
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
    let report = fetch_all_report_with_intent(UsageFetchIntent::CliReadOnly);
    if json {
        // Keep stdout pure JSON: do NOT emit provider warnings here, since they
        // would corrupt downstream `--json` consumers that read stderr too.
        println!("{}", serde_json::to_string_pretty(&report.outputs)?);
    } else {
        for o in &report.outputs {
            render_light(o);
        }
        // Surface active-but-failed providers (e.g. an expired session cookie)
        // so they don't silently vanish from the output. One concise line per
        // failing provider, on stderr to keep stdout clean.
        for diagnostic in &report.diagnostics {
            eprintln!(
                "{}: {} — skipped",
                diagnostic.display_name(),
                diagnostic.message
            );
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
            credential_source: None,
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
            credential_source: None,
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
            credential_source: None,
            plan: None,
            email: None,
            metrics: Vec::new(),
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        };

        assert_eq!(output.display_name(), "Codex (Account 123e45...4000)");
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
        assert!(output.credential_source.is_none());
        assert_eq!(output.display_name(), "Codex");
        Ok(())
    }

    #[test]
    fn usage_output_round_trips_opencode_credential_source() -> Result<()> {
        let output: UsageOutput = serde_json::from_str(
            r#"{
                "provider": "Codex",
                "credential_source": "opencode",
                "plan": "Plus",
                "email": null,
                "metrics": []
            }"#,
        )?;

        assert_eq!(output.credential_source.as_deref(), Some("opencode"));
        assert_eq!(
            serde_json::to_value(output)?
                .get("credential_source")
                .and_then(serde_json::Value::as_str),
            Some("opencode")
        );
        Ok(())
    }

    fn sample_output(provider: &str) -> UsageOutput {
        UsageOutput {
            provider: provider.to_string(),
            account: None,
            credential_source: None,
            plan: None,
            email: None,
            metrics: Vec::new(),
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        }
    }

    /// A provider that has credentials but whose fetch fails must stay visible.
    ///
    /// `has_credentials()` reports such a provider as active, so dropping its
    /// error would make the provider silently vanish from the output instead of
    /// telling the user their session needs refreshing.
    #[test]
    fn fetch_provider_report_surfaces_provider_errors_instead_of_dropping_them() {
        let report = fetch_provider_report(
            "Sakana",
            Err(anyhow::anyhow!(
                "Sakana session expired or invalid. Refresh SAKANA_SESSION_COOKIE."
            )),
        );

        assert!(
            report.outputs.is_empty(),
            "a failed fetch must not produce usage outputs, got: {:?}",
            report.outputs
        );
        assert_eq!(
            report.diagnostics.len(),
            1,
            "expected exactly one diagnostic for the failing provider, got: {:?}",
            report.diagnostics
        );

        let diagnostic = &report.diagnostics[0];
        assert_eq!(diagnostic.provider, "Sakana");
        assert_eq!(diagnostic.kind, UsageFetchDiagnosticKind::FetchFailed);
        assert_eq!(diagnostic.severity, UsageFetchDiagnosticSeverity::Error);
        assert!(
            diagnostic.message.contains("SAKANA_SESSION_COOKIE"),
            "expected the auth-refresh guidance to be preserved, got: {}",
            diagnostic.message
        );
        assert_eq!(diagnostic.display_name(), "Sakana");
    }

    #[test]
    fn fetch_provider_report_reports_no_diagnostics_when_fetch_succeeds() {
        let report = fetch_provider_report(
            "Codex",
            Ok(vec![sample_output("Codex"), sample_output("Codex")]),
        );

        assert_eq!(report.outputs.len(), 2);
        assert!(report.outputs.iter().all(|o| o.provider == "Codex"));
        assert!(
            report.diagnostics.is_empty(),
            "a successful fetch must not emit diagnostics, got: {:?}",
            report.diagnostics
        );
    }
}
