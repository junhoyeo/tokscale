#![deny(clippy::all)]

mod aggregator;
pub mod bucket_tz;
mod cc_mirror;
pub mod clients;
pub mod content_extractor;
pub mod fs_atomic;
pub mod mcp;
mod message_cache;
pub mod model_alias;
pub mod opencode_model_name;
mod parser;
pub mod paths;
pub mod pricing;
mod provider_identity;
pub mod scanner;
pub mod sessionize;
pub mod sessions;
pub mod tui_signal;
pub mod wiki;

pub use aggregator::*;
pub use bucket_tz::BucketTimezone;
pub use clients::{ClientCounts, ClientDef, ClientId, PathRoot};
pub use model_alias::ModelAliasMap;
pub use parser::*;
pub use scanner::*;
pub use sessionize::{
    compute_daily_active_time, compute_daily_active_time_in, compute_time_metrics, sessionize,
    SessionInterval, TimeMetrics, DEFAULT_IDLE_GAP_MS,
};
pub use sessions::{CostSource, UnifiedMessage};

use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Counts Claude cache entries rebuilt because they predate retention
/// provenance. Tests assert the rebuild is a one-time upgrade cost: the count
/// must stop growing once every entry carries the marker.
#[cfg(test)]
static RETENTION_PROVENANCE_REBUILDS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Strip a CLIProxyAPI-style `(level)` reasoning-effort suffix from a model id.
///
/// Mirrors <https://help.router-for.me/configuration/thinking>: the proxy
/// strips the parentheses before routing, so for pricing lookups we treat the
/// suffix as cosmetic and resolve to the base model. Accepts the level set the
/// proxy documents (case-insensitive — callers pass the lowercased id):
/// `minimal`, `low`, `medium`, `high`, `xhigh`, `auto`, `none`. Numeric
/// thinking budgets are intentionally not handled here.
pub(crate) fn strip_parenthesized_reasoning_tier(model_id: &str) -> Option<&str> {
    let without_closing_paren = model_id.strip_suffix(')')?;
    let (base_model, tier) = without_closing_paren.rsplit_once('(')?;

    if base_model.is_empty() || base_model.trim() != base_model {
        return None;
    }

    if !matches!(
        tier,
        "minimal" | "low" | "medium" | "high" | "xhigh" | "auto" | "none"
    ) {
        return None;
    }

    Some(base_model)
}

/// Canonical model identity — the model id that leaves the machine.
///
/// This is [`normalize_syntactic`] with **no alias folding**: purely structural
/// canonicalization (lowercase, strip a `(reasoning-tier)` suffix, strip a
/// trailing `-YYYYMMDD` date, rewrite `.`→`-` inside claude version numbers, and
/// fold an `anthropic/claude-…` prefix). It never consults the user's
/// machine-local `modelAliases`.
///
/// Every path that submits, uploads, exports as raw data, or persists a model id
/// MUST use this, not [`normalize_model_for_grouping`]. A machine-local alias
/// config must never rewrite the model identity persisted server-side, or usage
/// history would fragment and fork across a user's devices.
pub fn canonical_model_id(model_id: &str) -> String {
    normalize_syntactic(model_id)
}

/// Local display/grouping model name: [`canonical_model_id`] plus the user's
/// configured `modelAliases` fold. Every local report-grouping surface — the
/// models report, every `--group-by`, monthly, hourly, and the TUI — routes
/// through this so name variants fold uniformly for presentation.
///
/// The alias fold is **presentation only** and must never reach the
/// submit/upload/export/persist path (those use [`canonical_model_id`]), or a
/// machine-local alias config would rewrite the uploaded model identity. An
/// empty/unset alias config makes this identical to [`canonical_model_id`].
pub fn normalize_model_for_grouping(model_id: &str) -> String {
    model_alias::global().apply(normalize_syntactic(model_id))
}

/// Local display/grouping name with OpenCode's configured model label applied
/// when one exists. The configured label is scoped to OpenCode and matched by
/// provider plus raw model key; all other messages use the normal grouping
/// name.
pub fn model_name_for_grouping(client: &str, provider_id: &str, model_id: &str) -> String {
    let fallback = normalize_model_for_grouping(model_id);
    if client == "opencode" {
        opencode_model_name::global()
            .display_name(provider_id, model_id)
            .map(str::to_string)
            .unwrap_or(fallback)
    } else {
        fallback
    }
}

/// Structural-only model-name normalization: lowercase, strip a
/// `(reasoning-tier)` suffix, strip a trailing `-YYYYMMDD` date, rewrite `.`→`-`
/// inside claude version numbers, and fold an `anthropic/claude-…` prefix.
///
/// This is the syntactic half of [`normalize_model_for_grouping`] /
/// [`canonical_model_id`]. It is also used by [`model_alias::ModelAliasResolver`]
/// to normalize configured alias keys and values into the same space, so a
/// configured alias matches its model regardless of case, dated suffix, or
/// `.`-vs-`-` spelling.
pub(crate) fn normalize_syntactic(model_id: &str) -> String {
    let mut name = model_id.to_lowercase();

    if let Some(base_model) = strip_parenthesized_reasoning_tier(&name) {
        name = base_model.to_string();
    }
    if name.len() > 9 {
        let potential_date = &name[name.len() - 8..];
        if potential_date.chars().all(|c| c.is_ascii_digit())
            && name.as_bytes()[name.len() - 9] == b'-'
        {
            name = name[..name.len() - 9].to_string();
        }
    }

    if name.contains("claude") {
        let chars: Vec<char> = name.chars().collect();
        let mut result = String::with_capacity(name.len());
        for i in 0..chars.len() {
            if chars[i] == '.'
                && i > 0
                && i < chars.len() - 1
                && chars[i - 1].is_ascii_digit()
                && chars[i + 1].is_ascii_digit()
            {
                result.push('-');
            } else {
                result.push(chars[i]);
            }
        }
        name = result;
    }

    if let Some(canonical) = normalize_anthropic_prefixed_claude_model(&name) {
        name = canonical;
    }

    name
}

fn normalize_anthropic_prefixed_claude_model(model_id: &str) -> Option<String> {
    let rest = model_id.strip_prefix("anthropic/claude-")?;
    let mut parts = rest.split('-');
    let major = parts.next()?;
    let minor = parts.next()?;
    let family = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    if !matches!(family, "opus" | "sonnet" | "haiku") {
        return None;
    }

    Some(format!("claude-{family}-{major}-{minor}"))
}

fn retain_for_requested_clients(
    client: &str,
    model_id: &str,
    provider_id: &str,
    requested: &HashSet<&str>,
) -> bool {
    requested.contains(client)
        || (requested.contains("claude") && client.starts_with("cc-mirror/"))
        // "gjc" is a superset request: 9Router bridge data IS gjc-format, so
        // requesting gjc retains 9router-stamped messages too. The reverse is
        // intentionally NOT true — `--client 9router` must retain only
        // 9router-stamped messages, not native gjc ones.
        || (requested.contains("gjc") && client.eq_ignore_ascii_case("9router"))
        || (requested.contains("synthetic")
            && sessions::synthetic::matches_synthetic_filter(client, model_id, provider_id))
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub enum GroupBy {
    Model,
    #[default]
    ClientModel,
    ClientProviderModel,
    WorkspaceModel,
    Session,
    ClientSession,
}

impl std::fmt::Display for GroupBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GroupBy::Model => write!(f, "model"),
            GroupBy::ClientModel => write!(f, "client,model"),
            GroupBy::ClientProviderModel => write!(f, "client,provider,model"),
            GroupBy::WorkspaceModel => write!(f, "workspace,model"),
            GroupBy::Session => write!(f, "session,model"),
            GroupBy::ClientSession => write!(f, "client,session,model"),
        }
    }
}

impl std::str::FromStr for GroupBy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized: String = s.split(',').map(|p| p.trim()).collect::<Vec<_>>().join(",");
        match normalized.to_lowercase().as_str() {
            "model" => Ok(GroupBy::Model),
            "client,model" | "client-model" => Ok(GroupBy::ClientModel),
            "client,provider,model" | "client-provider-model" => Ok(GroupBy::ClientProviderModel),
            "workspace,model" | "workspace-model" => Ok(GroupBy::WorkspaceModel),
            "session" | "session,model" | "session-model" => Ok(GroupBy::Session),
            "client,session" | "client-session" | "client,session,model" | "client-session-model" => {
                Ok(GroupBy::ClientSession)
            }
            _ => Err(format!(
                "Invalid group-by value: '{}'. Valid options: model, client,model, client,provider,model, workspace,model, session,model, client,session,model",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TokenBreakdown {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
}

impl TokenBreakdown {
    /// Add every token bucket from `other`, saturating each field independently.
    ///
    /// Use this for whole-breakdown aggregation so adding a new bucket cannot
    /// silently leave one hand-written accumulation site incomplete.
    pub fn add_assign_saturating(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
    }

    pub fn total(&self) -> i64 {
        // saturating so clamped (i64::MAX) buckets from a corrupt source can't
        // overflow the sum.
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }
}

impl std::ops::AddAssign<&TokenBreakdown> for TokenBreakdown {
    fn add_assign(&mut self, other: &TokenBreakdown) {
        self.add_assign_saturating(other);
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPerformance {
    #[serde(rename = "msPer1KTokens")]
    pub ms_per_1k_tokens: Option<f64>,
    pub total_duration_ms: i64,
    pub timed_tokens: i64,
    pub sample_count: i32,
    pub token_coverage: f64,
}

impl ModelPerformance {
    pub fn record_message(&mut self, token_total: i64, duration_ms: Option<i64>) {
        let Some(duration_ms) = duration_ms else {
            return;
        };
        if duration_ms <= 0 || token_total <= 0 {
            return;
        }

        self.total_duration_ms = self.total_duration_ms.saturating_add(duration_ms);
        self.timed_tokens = self.timed_tokens.saturating_add(token_total);
        self.sample_count = self.sample_count.saturating_add(1);
    }

    pub fn finalize(&mut self, total_tokens: i64) {
        self.ms_per_1k_tokens = if self.timed_tokens > 0 && self.total_duration_ms > 0 {
            Some(self.total_duration_ms as f64 * 1000.0 / self.timed_tokens as f64)
        } else {
            None
        };

        self.token_coverage = if total_tokens > 0 {
            (self.timed_tokens as f64 / total_tokens as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    pub fn from_totals(total_duration_ms: i64, timed_tokens: i64, sample_count: i32) -> Self {
        let mut performance = Self {
            total_duration_ms,
            timed_tokens,
            sample_count,
            ..Self::default()
        };
        performance.finalize(timed_tokens);
        performance
    }
}

#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub client: String,
    pub model_id: String,
    pub provider_id: String,
    pub session_id: String,
    pub workspace_key: Option<String>,
    pub workspace_label: Option<String>,
    pub timestamp: i64,
    pub date: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub duration_ms: Option<i64>,
    pub message_count: i32,
    pub agent: Option<String>,
}

pub struct ParsedMessages {
    pub messages: Vec<ParsedMessage>,
    pub counts: ClientCounts,
    pub processing_time_ms: u32,
}

impl Clone for ParsedMessages {
    fn clone(&self) -> Self {
        let mut counts = ClientCounts::new();
        for client in ClientId::iter() {
            counts.set(client, self.counts.get(client));
        }

        Self {
            messages: self.messages.clone(),
            counts,
            processing_time_ms: self.processing_time_ms,
        }
    }
}

impl std::fmt::Debug for ParsedMessages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("ParsedMessages");
        debug.field("messages", &self.messages);
        for client in ClientId::iter() {
            debug.field(client.as_str(), &self.counts.get(client));
        }
        debug.field("processing_time_ms", &self.processing_time_ms);
        debug.finish()
    }
}

/// Database state used to resolve Devin Desktop ACP titles. The source stream
/// is deliberately absent: one lookup is valid for every Desktop file that
/// observed the same CLI database/WAL snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DevinDesktopLookupSnapshot {
    db_paths: Vec<PathBuf>,
    related_files: Vec<message_cache::RelatedFileFingerprint>,
}

type DevinDesktopLookupCache = Mutex<
    HashMap<DevinDesktopLookupSnapshot, Arc<OnceLock<sessions::devin::DevinDesktopSessionLookup>>>,
>;

/// Return the shared title lookup cell for one post-validation database
/// snapshot. The cell is placed in the map before it is initialized, allowing
/// parallel Desktop files from one snapshot to share one SQLite scan without
/// holding the map lock during that scan.
fn devin_desktop_lookup_cell_for_snapshot(
    lookup_cache: &DevinDesktopLookupCache,
    db_paths: &[PathBuf],
    fingerprint: &message_cache::SourceFingerprint,
) -> Arc<OnceLock<sessions::devin::DevinDesktopSessionLookup>> {
    let snapshot = DevinDesktopLookupSnapshot {
        db_paths: db_paths.to_vec(),
        related_files: fingerprint.related_files.clone(),
    };
    let mut lookups = lookup_cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        lookups
            .entry(snapshot)
            .or_insert_with(|| Arc::new(OnceLock::new())),
    )
}

#[derive(Debug, Clone, Default)]
pub struct LocalParseOptions {
    pub home_dir: Option<String>,
    pub use_env_roots: bool,
    pub clients: Option<Vec<String>>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
    /// Persistent scanner config loaded from `~/.config/tokscale/settings.json`.
    /// Defaults to empty when callers don't care about user-configured paths.
    pub scanner_settings: scanner::ScannerSettings,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DailyTotals {
    pub tokens: i64,
    pub cost: f64,
    pub messages: i32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClientContribution {
    pub client: String,
    pub model_id: String,
    pub provider_id: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub messages: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyContribution {
    pub date: String,
    pub totals: DailyTotals,
    pub intensity: u8,
    pub token_breakdown: TokenBreakdown,
    pub clients: Vec<ClientContribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_time_ms: Option<i64>,
}

/// Per-session aggregate of token usage, cost, and timing — keyed on
/// `session_id` so downstream consumers can attribute cost to a specific
/// agent-CLI session rather than just a date or model rollup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SessionContribution {
    pub session_id: String,
    pub client: String,
    pub provider: String,
    pub model: String,
    pub totals: DailyTotals,
    pub token_breakdown: TokenBreakdown,
    pub clients: Vec<ClientContribution>,
    /// Earliest message timestamp (unix seconds) in the session.
    pub first_seen: i64,
    /// Latest message timestamp (unix seconds) in the session.
    pub last_seen: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct YearSummary {
    pub year: String,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub range_start: String,
    pub range_end: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DataSummary {
    pub total_tokens: i64,
    pub total_cost: f64,
    pub total_days: i32,
    pub active_days: i32,
    pub average_per_day: f64,
    pub max_cost_in_single_day: f64,
    pub clients: Vec<String>,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphMeta {
    pub generated_at: String,
    pub version: String,
    pub date_range_start: String,
    pub date_range_end: String,
    pub processing_time_ms: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphResult {
    pub meta: GraphMeta,
    pub summary: DataSummary,
    pub years: Vec<YearSummary>,
    pub contributions: Vec<DailyContribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_metrics: Option<sessionize::TimeMetrics>,
    #[serde(skip)]
    pub unpriced_submission_exclusions: Vec<UnpricedSubmissionExclusion>,
}

/// Token-bearing usage excluded only from a submission because it cannot be
/// priced authoritatively. Local reports retain the original usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpricedSubmissionExclusion {
    pub provider_id: String,
    pub model_id: String,
    pub message_count: usize,
    pub total_tokens: i64,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct ReportOptions {
    pub home_dir: Option<String>,
    pub use_env_roots: bool,
    pub clients: Option<Vec<String>>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
    pub group_by: GroupBy,
    /// Whether `workspace,model` rows fold git worktrees into their parent repo.
    /// Only consulted for [`GroupBy::WorkspaceModel`].
    pub worktree_rollup: WorktreeRollup,
    /// Persistent scanner config loaded from `~/.config/tokscale/settings.json`.
    /// Defaults to empty when callers don't care about user-configured paths.
    pub scanner_settings: scanner::ScannerSettings,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelUsage {
    pub client: String,
    pub merged_clients: Option<String>,
    pub workspace_key: Option<String>,
    pub workspace_label: Option<String>,
    pub session_id: Option<String>,
    pub model: String,
    pub provider: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub message_count: i32,
    pub cost: f64,
    pub performance: ModelPerformance,
}

/// Original monthly usage shape returned by [`get_monthly_report`].
///
/// This type intentionally remains unchanged so downstream struct literals stay
/// source-compatible. Use [`MonthlyUsageV2`] and [`get_monthly_report_v2`] when
/// reasoning tokens are required.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MonthlyUsage {
    pub month: String,
    pub models: Vec<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub message_count: i32,
    pub cost: f64,
}

/// Complete monthly usage including reasoning tokens.
///
/// This versioned type keeps downstream [`MonthlyUsage`] struct literals
/// source-compatible while allowing CLI and API consumers to opt into the
/// complete token breakdown.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MonthlyUsageV2 {
    pub month: String,
    pub models: Vec<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub message_count: i32,
    pub cost: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelReport {
    pub entries: Vec<ModelUsage>,
    pub total_input: i64,
    pub total_output: i64,
    pub total_cache_read: i64,
    pub total_cache_write: i64,
    pub total_messages: i32,
    pub total_cost: f64,
    pub processing_time_ms: u32,
}

const UNKNOWN_WORKSPACE_LABEL: &str = "Unknown workspace";
const UNKNOWN_WORKSPACE_GROUP_KEY: &str = "\0unknown-workspace";

#[derive(Debug, Clone, serde::Serialize)]
pub struct MonthlyReport {
    pub entries: Vec<MonthlyUsage>,
    pub total_cost: f64,
    pub processing_time_ms: u32,
}

/// Complete monthly report whose entries retain reasoning tokens.
///
/// Use [`get_monthly_report_v2`] to generate this shape. The original
/// [`MonthlyReport`] and [`get_monthly_report`] remain available for source
/// compatibility.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MonthlyReportV2 {
    pub entries: Vec<MonthlyUsageV2>,
    pub total_cost: f64,
    pub processing_time_ms: u32,
}

impl MonthlyUsageV2 {
    /// Convert to the original public shape, intentionally discarding the
    /// `reasoning` bucket that [`MonthlyUsage`] predates.
    pub fn into_legacy(self) -> MonthlyUsage {
        MonthlyUsage {
            month: self.month,
            models: self.models,
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            message_count: self.message_count,
            cost: self.cost,
        }
    }
}

impl MonthlyReportV2 {
    /// Convert to the original public report shape.
    ///
    /// Each entry's `reasoning` bucket is intentionally discarded because
    /// [`MonthlyUsage`] predates that field.
    pub fn into_legacy(self) -> MonthlyReport {
        MonthlyReport {
            entries: self
                .entries
                .into_iter()
                .map(MonthlyUsageV2::into_legacy)
                .collect(),
            total_cost: self.total_cost,
            processing_time_ms: self.processing_time_ms,
        }
    }
}

/// Hourly usage entry for a single hour slot (e.g. "2026-03-23 14:00")
#[derive(Debug, Clone, serde::Serialize)]
pub struct HourlyUsage {
    pub hour: String,
    pub clients: Vec<String>,
    pub models: Vec<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub message_count: i32,
    /// Number of user interaction turns (user→assistant boundaries).
    pub turn_count: i32,
    pub reasoning: i64,
    pub cost: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HourlyReport {
    pub entries: Vec<HourlyUsage>,
    pub total_cost: f64,
    pub processing_time_ms: u32,
}

/// Resolve the home directory every `from_dir`-style parser scans from.
///
/// An explicit `--home` always wins. Everything else goes through
/// [`crate::paths::home_dir`], which is the *only* place allowed to read
/// `$HOME`.
///
/// Reading `$HOME` here directly — as this used to — defeated that resolver
/// entirely, because the raw read ran first and always won. On Windows a Git
/// Bash `HOME=/home/user` therefore still reached every caller, and `Path`
/// resolves that against the current drive, so the model/monthly/hourly
/// reports and local parsing scanned `C:\home\user` instead of the real
/// profile — precisely the case `paths::home_dir` was written to prevent. An
/// exported-but-empty `HOME` was worse: it produced `Ok("")`, and the
/// `format!("{home}/...")` joins downstream turned that into absolute scans
/// from the filesystem root.
pub fn get_home_dir_string(home_dir_option: &Option<String>) -> Result<String, String> {
    home_dir_option
        .clone()
        .or_else(|| crate::paths::home_dir().map(|p| p.to_string_lossy().into_owned()))
        .ok_or_else(|| {
            "HOME directory not specified and could not determine home directory".to_string()
        })
}

#[allow(dead_code)]
fn parse_all_messages_with_pricing(
    home_dir: &str,
    clients: &[String],
    pricing: Option<&pricing::PricingService>,
) -> Vec<UnifiedMessage> {
    parse_all_messages_with_pricing_with_env_strategy(
        home_dir,
        clients,
        pricing,
        true,
        &scanner::ScannerSettings::default(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceCachePolicy {
    Persistent,
    InMemory,
}

fn parse_all_messages_with_pricing_with_env_strategy(
    home_dir: &str,
    clients: &[String],
    pricing: Option<&pricing::PricingService>,
    use_env_roots: bool,
    scanner_settings: &scanner::ScannerSettings,
) -> Vec<UnifiedMessage> {
    parse_all_messages_with_pricing_with_cache_policy(
        home_dir,
        clients,
        pricing,
        use_env_roots,
        scanner_settings,
        SourceCachePolicy::Persistent,
    )
}

fn parse_all_messages_with_pricing_with_cache_policy(
    home_dir: &str,
    clients: &[String],
    pricing: Option<&pricing::PricingService>,
    use_env_roots: bool,
    scanner_settings: &scanner::ScannerSettings,
    cache_policy: SourceCachePolicy,
) -> Vec<UnifiedMessage> {
    #[derive(Debug)]
    struct CachedParseOutcome {
        messages: Vec<UnifiedMessage>,
        retained_message_keys: HashSet<String>,
        cache_entry: Option<message_cache::CachedSourceEntry>,
        invalidate_cache: bool,
    }

    fn apply_pricing_to_messages(
        messages: &mut [UnifiedMessage],
        pricing: Option<&pricing::PricingService>,
    ) {
        for message in messages {
            message.refresh_derived_fields();
            apply_pricing_if_available(message, pricing);
        }
    }

    /// Takes the entry's messages by value: the cache handed this entry to
    /// its one consumer (see `SourceMessageCache::take`), so cloning here
    /// would put a second copy of the transcript alongside the first for no
    /// reason.
    fn cached_messages(
        cached: message_cache::CachedSourceEntry,
        pricing: Option<&pricing::PricingService>,
    ) -> Vec<UnifiedMessage> {
        let mut messages = cached.messages;
        apply_pricing_to_messages(&mut messages, pricing);
        messages
    }

    fn parse_full_log_source(
        path: &Path,
        pricing: Option<&pricing::PricingService>,
        is_headless: bool,
    ) -> CachedParseOutcome {
        let fallback_timestamp = sessions::utils::file_modified_timestamp_ms(path);
        let parsed = sessions::codex::parse_codex_file_incremental(
            path,
            0,
            sessions::codex::CodexParseState::default(),
        );
        let messages = finalize_codex_messages(
            parsed.messages.clone(),
            pricing,
            is_headless,
            &parsed.fallback_timestamp_indices,
            fallback_timestamp,
        );
        if !parsed.parse_succeeded {
            return CachedParseOutcome {
                messages,
                retained_message_keys: HashSet::new(),
                cache_entry: None,
                invalidate_cache: false,
            };
        }

        if parsed.unresolved_model_events {
            return CachedParseOutcome {
                messages,
                retained_message_keys: HashSet::new(),
                cache_entry: None,
                invalidate_cache: false,
            };
        }

        let cache_entry = build_codex_cache_entry(
            path,
            parsed.messages,
            parsed.consumed_offset,
            parsed.state,
            parsed.fallback_timestamp_indices,
        );

        CachedParseOutcome {
            messages,
            retained_message_keys: HashSet::new(),
            cache_entry,
            invalidate_cache: false,
        }
    }

    fn finalize_codex_messages(
        mut messages: Vec<UnifiedMessage>,
        pricing: Option<&pricing::PricingService>,
        is_headless: bool,
        fallback_timestamp_indices: &[usize],
        fallback_timestamp: i64,
    ) -> Vec<UnifiedMessage> {
        for index in fallback_timestamp_indices {
            if let Some(message) = messages.get_mut(*index) {
                message.set_timestamp(fallback_timestamp);
            }
        }
        apply_pricing_to_messages(&mut messages, pricing);
        for message in &mut messages {
            apply_headless_agent(message, is_headless);
        }
        messages
    }

    fn build_codex_cache_entry(
        path: &Path,
        raw_messages: Vec<UnifiedMessage>,
        consumed_offset: u64,
        state: sessions::codex::CodexParseState,
        fallback_timestamp_indices: Vec<usize>,
    ) -> Option<message_cache::CachedSourceEntry> {
        let fingerprint = message_cache::SourceFingerprint::from_path(path)?;
        if fingerprint.size != consumed_offset {
            return None;
        }

        let codex_incremental = message_cache::build_codex_incremental_cache_with_prefix_hash(
            path,
            consumed_offset,
            state,
            fingerprint.content_hash,
        )?;

        Some(message_cache::CachedSourceEntry::new(
            message_cache::CacheIdentity::for_client(ClientId::Codex),
            path,
            fingerprint,
            raw_messages,
            fallback_timestamp_indices,
            Some(codex_incremental),
        ))
    }

    /// What a changed source file is allowed to do to the history the cache
    /// already holds for it.
    #[derive(Clone, Copy)]
    enum HistoryRetention {
        /// The live file is the whole truth. Anything it no longer contains
        /// leaves the totals — correct for a source whose file content is a
        /// faithful record of what the client did.
        LiveFileOnly,
        /// Carry forward messages this exact file was previously observed to
        /// contain. For clients that rewrite a transcript in place.
        RetainObserved {
            /// Decides, per dedup key, whether the message may be carried
            /// forward. A retained copy outlives the bytes that produced it,
            /// so it has to keep collapsing against a live copy of the same
            /// message written anywhere else; a key that is only unique
            /// within one file cannot do that and must be dropped instead.
            /// The lane owning the key format supplies this.
            key_is_globally_stable: fn(&str) -> bool,
        },
    }

    /// Merge the messages a previous scan recorded for this exact file back
    /// into a fresh parse of it.
    ///
    /// Within this entry the live file stays authoritative for everything it
    /// still contains: a key present on both sides keeps the freshly parsed
    /// message, so a corrected re-parse still wins and nothing is frozen at a
    /// stale value. Only keys the file no longer carries are carried forward.
    /// Across entries the Claude lane separately carries the returned key set,
    /// so cross-file dedup can merge a retained partial with a completed live
    /// replay instead of depending on lexical path order.
    ///
    /// Messages without a dedup key are never retained. The key is what lets a
    /// later scan recognise the message as already-seen; re-emitting an
    /// unkeyed one would double count it the moment the file regained it. Keys
    /// that `key_is_globally_stable` rejects are dropped for the same reason:
    /// they would never collapse against a live copy elsewhere.
    fn retain_observed_messages(
        parsed: &mut Vec<UnifiedMessage>,
        cached: &[UnifiedMessage],
        key_is_globally_stable: fn(&str) -> bool,
    ) -> HashSet<String> {
        let mut seen: HashSet<String> = parsed
            .iter()
            .filter_map(|message| message.dedup_key.clone())
            .collect();

        let mut retained = HashSet::new();
        for message in cached {
            let Some(key) = message.dedup_key.as_ref() else {
                continue;
            };
            if !key_is_globally_stable(key) {
                continue;
            }
            if seen.insert(key.clone()) {
                parsed.push(message.clone());
                retained.insert(key.clone());
            }
        }
        retained
    }

    fn load_or_parse_source_with_fingerprint_and_policy<F, FingerprintFn>(
        identity: message_cache::CacheIdentity,
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        history: HistoryRetention,
        fingerprint_from_path: FingerprintFn,
        parse: F,
    ) -> CachedParseOutcome
    where
        F: Fn(&Path, Option<&message_cache::SourceFingerprint>) -> (Vec<UnifiedMessage>, bool),
        FingerprintFn: Fn(
            &Path,
            Option<&message_cache::SourceFingerprint>,
        ) -> Option<message_cache::FingerprintStatus>,
    {
        let mut cached = source_cache.take(identity, path);
        // An entry written before retention provenance existed cannot say
        // which of its rows the live transcript already dropped, so serving it
        // warm presents a retained copy of a response as a live one — and the
        // live copy of that same response in a forked transcript then loses
        // the merge, freezing the stale model attribution and the cost priced
        // from it. Rebuild those entries by taking the ordinary re-parse path
        // once: it re-derives the retained set from the live bytes and writes
        // the entry back with the provenance marker, so the next scan is a
        // plain warm hit.
        let rebuild_retention_provenance = cached
            .as_ref()
            .is_some_and(message_cache::CachedSourceEntry::needs_retention_provenance_migration);
        #[cfg(test)]
        if rebuild_retention_provenance {
            RETENTION_PROVENANCE_REBUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let Some(fingerprint_status) =
            fingerprint_from_path(path, cached.as_ref().map(|entry| &entry.fingerprint))
        else {
            let (mut messages, _) = parse(path, None);
            apply_pricing_to_messages(&mut messages, pricing);
            return CachedParseOutcome {
                messages,
                retained_message_keys: HashSet::new(),
                cache_entry: None,
                invalidate_cache: false,
            };
        };

        let fingerprint = match fingerprint_status {
            message_cache::FingerprintStatus::Unchanged => {
                let Some(entry) = cached.take() else {
                    unreachable!("an uncached source always builds a complete fingerprint")
                };
                if !rebuild_retention_provenance && !entry.messages.is_empty() {
                    let retained_message_keys = entry.retained_message_keys();
                    return CachedParseOutcome {
                        messages: cached_messages(entry, pricing),
                        retained_message_keys,
                        cache_entry: None,
                        invalidate_cache: false,
                    };
                }
                let fingerprint = entry.fingerprint.clone();
                cached = Some(entry);
                fingerprint
            }
            message_cache::FingerprintStatus::Changed(fingerprint) => fingerprint,
        };

        if let Some(entry) = cached.take() {
            if !rebuild_retention_provenance
                && entry.fingerprint == fingerprint
                && !entry.messages.is_empty()
            {
                let retained_message_keys = entry.retained_message_keys();
                return CachedParseOutcome {
                    messages: cached_messages(entry, pricing),
                    retained_message_keys,
                    cache_entry: None,
                    invalidate_cache: false,
                };
            }
            cached = Some(entry);
        }

        let (mut messages, cacheable) = parse(path, Some(&fingerprint));
        // Reaching here means the file changed under a cache entry we still
        // hold. For a source that rewrites transcripts in place that is not
        // only "new content appeared" — it can also be "already-published
        // messages disappeared", and recomputing purely from the live bytes
        // would retire them from history (#994). Only merge when the parse is
        // cacheable: an untrustworthy parse must not be used to synthesise an
        // entry, and the caller invalidates on that path anyway.
        let mut retained_message_keys = HashSet::new();
        if let HistoryRetention::RetainObserved {
            key_is_globally_stable,
        } = history
        {
            if cacheable {
                if let Some(cached) = cached.as_ref() {
                    retained_message_keys = retain_observed_messages(
                        &mut messages,
                        &cached.messages,
                        key_is_globally_stable,
                    );
                }
            }
        }
        let cache_entry = if messages.is_empty() || !cacheable {
            None
        } else {
            Some(match history {
                HistoryRetention::LiveFileOnly => message_cache::CachedSourceEntry::new(
                    identity,
                    path,
                    fingerprint,
                    messages.clone(),
                    Vec::new(),
                    None,
                ),
                HistoryRetention::RetainObserved { .. } => {
                    message_cache::CachedSourceEntry::new_with_retained_message_keys(
                        identity,
                        path,
                        fingerprint,
                        messages.clone(),
                        &retained_message_keys,
                    )
                }
            })
        };
        apply_pricing_to_messages(&mut messages, pricing);

        CachedParseOutcome {
            messages,
            retained_message_keys,
            cache_entry,
            invalidate_cache: !cacheable,
        }
    }

    fn load_or_parse_source_with_fingerprint<F, FingerprintFn>(
        identity: message_cache::CacheIdentity,
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        fingerprint_from_path: FingerprintFn,
        parse: F,
    ) -> CachedParseOutcome
    where
        F: Fn(&Path) -> Vec<UnifiedMessage>,
        FingerprintFn: Fn(
            &Path,
            Option<&message_cache::SourceFingerprint>,
        ) -> Option<message_cache::FingerprintStatus>,
    {
        load_or_parse_source_with_fingerprint_and_policy(
            identity,
            path,
            source_cache,
            pricing,
            HistoryRetention::LiveFileOnly,
            fingerprint_from_path,
            |path, _| (parse(path), true),
        )
    }

    /// Same as `load_or_parse_source_with_fingerprint`, for clients that
    /// rewrite an existing transcript instead of only appending to it.
    ///
    /// Scoped deliberately rather than made the default. Retention is only
    /// sound where a message carries a dedup key that identifies it by
    /// content across files, because the retained copy has to collapse
    /// against any live copy of the same message elsewhere. Sources keyed by
    /// file position or by a per-scan ordinal do not qualify and would double
    /// count, and neither do some keys inside an otherwise-qualifying lane —
    /// hence `key_is_globally_stable` rather than a blanket per-client
    /// promise.
    fn load_or_parse_source_with_fingerprint_retaining_history<F, FingerprintFn>(
        identity: message_cache::CacheIdentity,
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        key_is_globally_stable: fn(&str) -> bool,
        fingerprint_from_path: FingerprintFn,
        parse: F,
    ) -> CachedParseOutcome
    where
        F: Fn(&Path) -> Vec<UnifiedMessage>,
        FingerprintFn: Fn(
            &Path,
            Option<&message_cache::SourceFingerprint>,
        ) -> Option<message_cache::FingerprintStatus>,
    {
        load_or_parse_source_with_fingerprint_and_policy(
            identity,
            path,
            source_cache,
            pricing,
            HistoryRetention::RetainObserved {
                key_is_globally_stable,
            },
            fingerprint_from_path,
            |path, _| (parse(path), true),
        )
    }

    fn load_or_parse_source_with_fingerprint_context<F, FingerprintFn>(
        identity: message_cache::CacheIdentity,
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        fingerprint_from_path: FingerprintFn,
        parse: F,
    ) -> CachedParseOutcome
    where
        F: Fn(&Path, Option<&message_cache::SourceFingerprint>) -> Vec<UnifiedMessage>,
        FingerprintFn: Fn(
            &Path,
            Option<&message_cache::SourceFingerprint>,
        ) -> Option<message_cache::FingerprintStatus>,
    {
        load_or_parse_source_with_fingerprint_and_policy(
            identity,
            path,
            source_cache,
            pricing,
            HistoryRetention::LiveFileOnly,
            fingerprint_from_path,
            |path, fingerprint| (parse(path, fingerprint), true),
        )
    }

    fn load_or_parse_source<F>(
        identity: message_cache::CacheIdentity,
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        parse: F,
    ) -> CachedParseOutcome
    where
        F: Fn(&Path) -> Vec<UnifiedMessage>,
    {
        load_or_parse_source_with_fingerprint(
            identity,
            path,
            source_cache,
            pricing,
            message_cache::SourceFingerprint::check_path_samples_only,
            parse,
        )
    }

    /// Parse one direct cached lane and merge it before the next lane starts.
    ///
    /// Outcomes are collected in parallel before either destination is mutated;
    /// this preserves the existing per-lane collection and post-collection cache
    /// insertion lifecycle. Callers invoke this helper in source order, so the
    /// call order also preserves the sequential lane order of the parser.
    ///
    /// The scan key and cache identity are intentionally derived from the same
    /// `ClientId`, making a same-client mapping impossible to mismatch. An
    /// asymmetric lane should use a separately named helper with its own contract.
    fn parse_cached_lane<F>(
        scan_result: &scanner::ScanResult,
        source_cache: &mut message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        all_messages: &mut Vec<UnifiedMessage>,
        scan_client: ClientId,
        parse: F,
    ) where
        F: Fn(&Path) -> Vec<UnifiedMessage> + Sync,
    {
        let cache_identity = message_cache::CacheIdentity::for_client(scan_client);
        let outcomes: Vec<CachedParseOutcome> = scan_result
            .get(scan_client)
            .par_iter()
            .map(|path| load_or_parse_source(cache_identity, path, source_cache, pricing, &parse))
            .collect();
        for outcome in outcomes {
            all_messages.extend(outcome.messages);
            if let Some(entry) = outcome.cache_entry {
                source_cache.insert(entry);
            }
        }
    }

    /// Same as [`parse_cached_lane`], for a client whose transcripts can repeat
    /// one another's rows verbatim.
    ///
    /// A DSH fork seeds the child transcript with the parent's completed prefix
    /// — same `message.id`, time and usage, under a different session id — so
    /// the per-source cache alone cannot collapse the copy. Dedup keys survive
    /// a warm cache hit, so the pass behaves identically cold and warm.
    ///
    /// Ownership, and what this pass does not decide: when the child header
    /// carries `seedLength` the parser drops the seeded rows at the source, so
    /// the parent's copy survives whatever order the scan hands the files over
    /// in. This pass is the fallback for a header that lost the field, which
    /// DSH's own readers treat as an unseeded log (`header.seedLength ?? 0` in
    /// `core/agent/src/inbox.ts` and `schedule/src/invariant.ts`) — nothing in
    /// the transcript then marks the prefix as inherited. It degrades to
    /// first-wins in scan-path order: totals and per-model rollups stay
    /// correct, and only the session label on the surviving row depends on
    /// which transcript sorts first.
    fn parse_cached_lane_deduped<F>(
        scan_result: &scanner::ScanResult,
        source_cache: &mut message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        all_messages: &mut Vec<UnifiedMessage>,
        scan_client: ClientId,
        parse: F,
    ) where
        F: Fn(&Path) -> Vec<UnifiedMessage> + Sync,
    {
        let cache_identity = message_cache::CacheIdentity::for_client(scan_client);
        let outcomes: Vec<CachedParseOutcome> = scan_result
            .get(scan_client)
            .par_iter()
            .map(|path| load_or_parse_source(cache_identity, path, source_cache, pricing, &parse))
            .collect();
        let mut seen: HashSet<String> = HashSet::new();
        for outcome in outcomes {
            all_messages.extend(
                outcome
                    .messages
                    .into_iter()
                    .filter(|message| should_keep_deduped_message(&mut seen, message)),
            );
            if let Some(entry) = outcome.cache_entry {
                source_cache.insert(entry);
            }
        }
    }

    fn uncached_prime_outcome(
        mut messages: Vec<UnifiedMessage>,
        accounting: sessions::prime_agent::PrimeFileAccounting,
        pricing: Option<&pricing::PricingService>,
    ) -> (
        CachedParseOutcome,
        sessions::prime_agent::PrimeFileAccounting,
    ) {
        apply_pricing_to_messages(&mut messages, pricing);
        (
            CachedParseOutcome {
                messages,
                retained_message_keys: HashSet::new(),
                cache_entry: None,
                invalidate_cache: false,
            },
            accounting,
        )
    }

    fn parse_stable_prime_source(
        path: &Path,
        identity: message_cache::CacheIdentity,
        mut fingerprint_before: message_cache::SourceFingerprint,
        pricing: Option<&pricing::PricingService>,
    ) -> (
        CachedParseOutcome,
        sessions::prime_agent::PrimeFileAccounting,
    ) {
        const MAX_STABLE_PARSE_ATTEMPTS: usize = 2;

        let mut last_parse = None;
        for _ in 0..MAX_STABLE_PARSE_ATTEMPTS {
            #[cfg(test)]
            sessions::prime_agent::run_stable_parse_test_hook(path);

            // Both views come from this one decoded record stream. Exact hashes
            // on either side ensure that the pair is only cached when the bytes
            // stayed at the fingerprint under which the entry is stored.
            let parsed = sessions::prime_agent::parse_prime_agent_file_with_accounting(path);
            let Some(fingerprint_after) = message_cache::SourceFingerprint::from_path(path) else {
                return uncached_prime_outcome(parsed.0, parsed.1, pricing);
            };
            if fingerprint_after == fingerprint_before {
                let (mut messages, accounting) = parsed;
                let cache_entry = (!messages.is_empty()).then(|| {
                    message_cache::CachedSourceEntry::new(
                        identity,
                        path,
                        fingerprint_after,
                        messages.clone(),
                        Vec::new(),
                        None,
                    )
                    .with_prime_accounting(accounting.clone())
                });
                apply_pricing_to_messages(&mut messages, pricing);
                return (
                    CachedParseOutcome {
                        messages,
                        retained_message_keys: HashSet::new(),
                        cache_entry,
                        invalidate_cache: false,
                    },
                    accounting,
                );
            }

            fingerprint_before = fingerprint_after;
            last_parse = Some(parsed);
        }

        // A continuously rewritten file still yields a coherent messages +
        // accounting pair from one pass, but no cache entry may claim that pair
        // belongs to either exact fingerprint observed around the read.
        let (messages, accounting) = last_parse.expect("the retry bound is non-zero");
        uncached_prime_outcome(messages, accounting, pricing)
    }

    fn load_or_parse_prime_source(
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
    ) -> (
        CachedParseOutcome,
        sessions::prime_agent::PrimeFileAccounting,
    ) {
        let identity = message_cache::CacheIdentity::for_client(ClientId::PrimeAgent);
        let cached = source_cache.take(identity, path);
        let Some(fingerprint_status) = message_cache::SourceFingerprint::check_path(
            path,
            cached.as_ref().map(|entry| &entry.fingerprint),
        ) else {
            let (messages, accounting) =
                sessions::prime_agent::parse_prime_agent_file_with_accounting(path);
            return uncached_prime_outcome(messages, accounting, pricing);
        };

        let mut fingerprint = match fingerprint_status {
            message_cache::FingerprintStatus::Unchanged => cached
                .as_ref()
                .expect("an uncached source always builds a complete fingerprint")
                .fingerprint
                .clone(),
            message_cache::FingerprintStatus::Changed(fingerprint) => fingerprint,
        };

        if let Some(cached) = cached {
            if cached.fingerprint == fingerprint && !cached.messages.is_empty() {
                if let Some(accounting) = cached.prime_accounting.clone() {
                    // Prime's accounting is byte-coupled to its messages. Warm
                    // v5 scans therefore hash the complete transcript before a
                    // hit, while still avoiding JSON decode and accounting walk.
                    match message_cache::SourceFingerprint::from_path(path) {
                        Some(refreshed) if refreshed == cached.fingerprint => {
                            return (
                                CachedParseOutcome {
                                    messages: cached_messages(cached, pricing),
                                    retained_message_keys: HashSet::new(),
                                    cache_entry: None,
                                    invalidate_cache: false,
                                },
                                accounting,
                            );
                        }
                        Some(refreshed) => fingerprint = refreshed,
                        None => {
                            let (messages, accounting) =
                                sessions::prime_agent::parse_prime_agent_file_with_accounting(path);
                            return uncached_prime_outcome(messages, accounting, pricing);
                        }
                    }
                } else {
                    // Version-4 entries already contain valid messages but predate
                    // Prime accounting metadata. Decode just the accounting view
                    // once, but never combine it with those messages until the
                    // fingerprint is revalidated with a full content hash: the
                    // file can change between the first bounded-sample check and
                    // this second transcript read, including outside sample windows.
                    #[cfg(test)]
                    sessions::prime_agent::run_accounting_backfill_test_hook(path);
                    let accounting = sessions::prime_agent::analyze_prime_agent_accounting(
                        path,
                        &cached.messages,
                    );
                    match message_cache::SourceFingerprint::from_path(path) {
                        Some(refreshed) if refreshed == fingerprint => {
                            let cache_entry =
                                cached.clone().with_prime_accounting(accounting.clone());
                            return (
                                CachedParseOutcome {
                                    messages: cached_messages(cached, pricing),
                                    retained_message_keys: HashSet::new(),
                                    cache_entry: Some(cache_entry),
                                    invalidate_cache: false,
                                },
                                accounting,
                            );
                        }
                        Some(refreshed) => fingerprint = refreshed,
                        None => {
                            let (messages, accounting) =
                                sessions::prime_agent::parse_prime_agent_file_with_accounting(path);
                            return uncached_prime_outcome(messages, accounting, pricing);
                        }
                    }
                }
            }
        }

        parse_stable_prime_source(path, identity, fingerprint, pricing)
    }

    fn load_or_parse_sqlite_source<F>(
        identity: message_cache::CacheIdentity,
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        parse: F,
    ) -> CachedParseOutcome
    where
        F: Fn(&Path) -> Vec<UnifiedMessage>,
    {
        load_or_parse_source_with_fingerprint(
            identity,
            path,
            source_cache,
            pricing,
            message_cache::SourceFingerprint::check_sqlite_path,
            parse,
        )
    }

    fn load_or_parse_codex_source(
        path: &Path,
        source_cache: &message_cache::SourceMessageCache,
        pricing: Option<&pricing::PricingService>,
        headless_roots: &[PathBuf],
    ) -> CachedParseOutcome {
        let identity = message_cache::CacheIdentity::for_client(ClientId::Codex);
        let is_headless = is_headless_path(path, headless_roots);
        let cached = source_cache.take(identity, path);
        if cached.is_none() {
            // The post-parse cache build computes the authoritative fingerprint
            // after reading the file. Avoid hashing an uncached source here
            // only to discard that digest before parsing it.
            return parse_full_log_source(path, pricing, is_headless);
        }
        let Some(fingerprint_status) = message_cache::SourceFingerprint::check_path(
            path,
            cached.as_ref().map(|entry| &entry.fingerprint),
        ) else {
            return parse_full_log_source(path, pricing, is_headless);
        };
        let fingerprint = match fingerprint_status {
            message_cache::FingerprintStatus::Unchanged => cached
                .as_ref()
                .expect("an uncached source always builds a complete fingerprint")
                .fingerprint
                .clone(),
            message_cache::FingerprintStatus::Changed(fingerprint) => fingerprint,
        };
        let fallback_timestamp = sessions::utils::file_modified_timestamp_ms(path);

        if let Some(cached) = cached {
            let reparse_from_start = |invalidate_cache: bool| {
                let mut outcome = parse_full_log_source(path, pricing, is_headless);
                outcome.invalidate_cache = invalidate_cache && outcome.cache_entry.is_none();
                outcome
            };

            if cached.fingerprint == fingerprint {
                if message_cache::codex_cache_entry_matches_fingerprint(&cached, &fingerprint) {
                    return CachedParseOutcome {
                        messages: finalize_codex_messages(
                            cached.messages,
                            pricing,
                            is_headless,
                            &cached.fallback_timestamp_indices,
                            fallback_timestamp,
                        ),
                        retained_message_keys: HashSet::new(),
                        cache_entry: None,
                        invalidate_cache: false,
                    };
                }

                return reparse_from_start(true);
            }

            if let Some(codex_incremental) = cached.codex_incremental.as_ref() {
                if fingerprint.size > codex_incremental.consumed_offset
                    && message_cache::codex_prefix_matches(path, codex_incremental)
                {
                    let parsed = sessions::codex::parse_codex_file_incremental(
                        path,
                        codex_incremental.consumed_offset,
                        codex_incremental.state.clone(),
                    );
                    if parsed.parse_succeeded && !parsed.unresolved_model_events {
                        let mut raw_messages = cached.messages.clone();
                        let mut fallback_timestamp_indices =
                            cached.fallback_timestamp_indices.clone();
                        let existing_len = raw_messages.len();
                        fallback_timestamp_indices.extend(
                            parsed
                                .fallback_timestamp_indices
                                .iter()
                                .map(|index| existing_len + index),
                        );
                        raw_messages.extend(parsed.messages);
                        let cache_entry = build_codex_cache_entry(
                            path,
                            raw_messages.clone(),
                            parsed.consumed_offset,
                            parsed.state,
                            fallback_timestamp_indices.clone(),
                        );
                        if let Some(cache_entry) = cache_entry {
                            let messages = finalize_codex_messages(
                                raw_messages,
                                pricing,
                                is_headless,
                                &fallback_timestamp_indices,
                                fallback_timestamp,
                            );

                            return CachedParseOutcome {
                                messages,
                                retained_message_keys: HashSet::new(),
                                cache_entry: Some(cache_entry),
                                invalidate_cache: false,
                            };
                        }
                    }
                }
            }

            return reparse_from_start(true);
        }

        unreachable!("uncached Codex sources return before fingerprint validation")
    }

    let scan_result = scanner::scan_all_clients_with_scanner_settings(
        home_dir,
        clients,
        use_env_roots,
        scanner_settings,
    );
    let headless_roots = scanner::headless_roots_with_env_strategy(home_dir, use_env_roots);
    // `load` reads no shard: each namespace is deserialized the first time a
    // lane asks for one of its sources, and entries whose file is gone are
    // pruned as that namespace loads. A scan therefore pays for the clients it
    // actually reads instead of for every client the machine has ever cached.
    let mut source_cache = match cache_policy {
        SourceCachePolicy::Persistent => message_cache::SourceMessageCache::load(),
        SourceCachePolicy::InMemory => message_cache::SourceMessageCache::default(),
    };
    let mut all_messages: Vec<UnifiedMessage> = Vec::new();
    let include_all = clients.is_empty();
    let include_synthetic = include_all || clients.iter().any(|c| c == "synthetic");
    let include_devin_cli = include_synthetic || clients.iter().any(|c| c == "devin-cli");
    let include_devin_desktop = include_synthetic || clients.iter().any(|c| c == "devin-desktop");
    // Freebuff and Codebuff share the manicode scan bucket in the scanner (the
    // two parsers partition the same file set). Each product parses and counts
    // only when it was actually requested, so a codebuff-only filter cannot
    // pick up estimated Freebuff rows and vice versa.
    let include_codebuff = include_all || clients.iter().any(|c| c == "codebuff");
    let include_freebuff = include_all || clients.iter().any(|c| c == "freebuff");

    // Parse OpenCode: prefer SQLite, collapse forked SQLite history there, then
    // suppress legacy JSON overlap by message identity.
    let mut opencode_seen: HashSet<String> = HashSet::new();

    for db_path in &scan_result.opencode_dbs {
        let CachedParseOutcome {
            messages,
            cache_entry,
            ..
        } = load_or_parse_sqlite_source(
            message_cache::CacheIdentity::for_client(ClientId::OpenCode),
            db_path,
            &source_cache,
            pricing,
            sessions::opencode::parse_opencode_sqlite,
        );

        // Dedup across channel-suffixed dbs: the same session can end up in
        // both `opencode.db` and `opencode-<channel>.db` if the user
        // switches channels mid-session. `discover_opencode_dbs` returns
        // paths in sorted order, so the first-seen copy is deterministic.
        all_messages.extend(messages.into_iter().filter(|message| {
            message
                .dedup_key
                .as_ref()
                .is_none_or(|key| opencode_seen.insert(key.clone()))
        }));

        if let Some(entry) = cache_entry {
            source_cache.insert(entry);
        }
    }

    let opencode_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::OpenCode)
        .par_iter()
        .filter_map(|path| {
            Some(load_or_parse_source(
                message_cache::CacheIdentity::for_client(ClientId::OpenCode),
                path,
                &source_cache,
                pricing,
                |path| {
                    sessions::opencode::parse_opencode_file(path)
                        .into_iter()
                        .collect()
                },
            ))
        })
        .collect();
    for outcome in opencode_outcomes {
        all_messages.extend(outcome.messages.into_iter().filter(|message| {
            message
                .dedup_key
                .as_ref()
                .is_none_or(|key| opencode_seen.insert(key.clone()))
        }));
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    // Parse MiMo Code: SQLite database(s)
    let mut micode_indices: HashMap<String, usize> = HashMap::new();

    for db_path in &scan_result.micode_dbs {
        // Pass `None` so the loader does not reprice: MiMo Code carries an
        // authoritative per-message cost that unconditional repricing would
        // overwrite (and persist to the cache). Reprice only messages that had
        // no embedded cost, mirroring the gjc lane's guard.
        let CachedParseOutcome {
            messages,
            cache_entry,
            ..
        } = load_or_parse_sqlite_source(
            message_cache::CacheIdentity::for_client(ClientId::MiMoCode),
            db_path,
            &source_cache,
            None,
            sessions::micode::parse_micode_sqlite,
        );

        for mut message in messages {
            if !message.has_authoritative_cost() {
                apply_pricing_if_available(&mut message, pricing);
            }
            if let Some(key) = message.dedup_key.as_ref() {
                if let Some(index) = micode_indices.get(key).copied() {
                    if message.has_authoritative_cost()
                        && !all_messages[index].has_authoritative_cost()
                    {
                        all_messages[index].cost = message.cost;
                        all_messages[index].mark_provider_reported_cost();
                    }
                    continue;
                }
                micode_indices.insert(key.clone(), all_messages.len());
            }
            all_messages.push(message);
        }

        if let Some(entry) = cache_entry {
            source_cache.insert(entry);
        }
    }

    let claude_home = PathBuf::from(home_dir);
    let claude_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Claude)
        .par_iter()
        .map(|path| {
            // Claude Code rewrites a transcript in place on resume/compact,
            // so a file can lose assistant turns it already published. The
            // retaining loader keeps those turns in the cache entry for as
            // long as the file itself exists; deleting the transcript still
            // drops them via `prune_missing_files`, which is what makes local
            // disk remain the source of truth.
            //
            // Only assistant turns are eligible: their `messageId:requestId`
            // key comes from the API response, so a retained copy still
            // collapses against the same turn replayed into a forked
            // transcript. Tool-result keys embed the transcript's file stem
            // and would not — see `dedup_key_is_globally_stable`.
            load_or_parse_source_with_fingerprint_retaining_history(
                message_cache::CacheIdentity::for_client(ClientId::Claude),
                path,
                &source_cache,
                pricing,
                sessions::claudecode::dedup_key_is_globally_stable,
                |path, cached| {
                    message_cache::SourceFingerprint::check_claude_code_path_with_home_samples_only(
                        path,
                        cached,
                        Some(&claude_home),
                    )
                },
                |path| sessions::claudecode::parse_claude_file_with_home(path, Some(&claude_home)),
            )
        })
        .collect();
    let mut claude_messages: Vec<(bool, UnifiedMessage)> = Vec::new();
    let mut claude_keyed_indices: HashMap<String, usize> = HashMap::new();
    for outcome in claude_outcomes {
        for message in outcome.messages {
            let Some(key) = message.dedup_key.clone().filter(|key| !key.is_empty()) else {
                claude_messages.push((false, message));
                continue;
            };
            let is_retained = outcome.retained_message_keys.contains(&key);
            let Some(index) = claude_keyed_indices.get(&key).copied() else {
                claude_keyed_indices.insert(key, claude_messages.len());
                claude_messages.push((is_retained, message));
                continue;
            };

            let (existing_is_retained, existing) = &mut claude_messages[index];
            merge_claude_cross_file_duplicate(
                existing,
                existing_is_retained,
                message,
                is_retained,
                pricing,
            );
        }
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }
    all_messages.extend(claude_messages.into_iter().map(|(_, message)| message));

    let codex_outcomes: Vec<(PathBuf, CachedParseOutcome)> = scan_result
        .get(ClientId::Codex)
        .par_iter()
        .map(|path| {
            (
                path.clone(),
                load_or_parse_codex_source(path, &source_cache, pricing, &headless_roots),
            )
        })
        .collect();
    let mut codex_seen: HashSet<String> = HashSet::new();
    for (path, outcome) in codex_outcomes {
        all_messages.extend(
            outcome
                .messages
                .into_iter()
                .filter(|message| should_keep_deduped_message(&mut codex_seen, message)),
        );
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        } else if outcome.invalidate_cache {
            source_cache.remove(
                message_cache::CacheIdentity::for_client(ClientId::Codex),
                &path,
            );
        }
    }

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Copilot,
        sessions::copilot::parse_copilot_file,
    );
    if let Some(db_path) = &scan_result.copilot_desktop_db {
        let otel_sessions: HashSet<String> = all_messages
            .iter()
            .filter(|message| message.client == "copilot")
            .map(|message| message.session_id.clone())
            .collect();
        let desktop_msgs = sessions::copilot_desktop::parse_copilot_desktop_db(db_path);
        all_messages.extend(
            desktop_msgs
                .into_iter()
                .filter(|message| !otel_sessions.contains(&message.session_id))
                .map(|mut message| {
                    apply_pricing_if_available(&mut message, pricing);
                    message
                }),
        );
    }
    {
        let existing_dedup_keys: HashSet<String> = all_messages
            .iter()
            .filter(|m| m.client == "copilot")
            .filter_map(|m| m.dedup_key.clone())
            .collect();
        let existing_copilot_session_timestamps: HashSet<(String, i64)> = all_messages
            .iter()
            .filter(|m| m.client == "copilot")
            .map(|m| (m.session_id.clone(), m.timestamp))
            .collect();
        let vscode_msgs = sessions::copilot_vscode::parse_copilot_vscode_sessions(
            &scan_result.copilot_vscode_sessions,
        );
        all_messages.extend(
            vscode_msgs
                .into_iter()
                .filter(|m| {
                    let key_unique = m
                        .dedup_key
                        .as_deref()
                        .map(|k| !existing_dedup_keys.contains(k))
                        .unwrap_or(true);
                    let session_ts_unique = !existing_copilot_session_timestamps
                        .contains(&(m.session_id.clone(), m.timestamp));
                    key_unique && session_ts_unique
                })
                .map(|mut message| {
                    apply_pricing_if_available(&mut message, pricing);
                    message
                }),
        );
    }

    let gemini_outcomes: Vec<(PathBuf, CachedParseOutcome)> = scan_result
        .get(ClientId::Gemini)
        .par_iter()
        .map(|path| {
            let outcome = load_or_parse_source_with_fingerprint_and_policy(
                message_cache::CacheIdentity::for_client(ClientId::Gemini),
                path,
                &source_cache,
                pricing,
                HistoryRetention::LiveFileOnly,
                message_cache::SourceFingerprint::check_path_samples_only,
                |path, _| {
                    let parsed = sessions::gemini::parse_gemini_file_with_cache_status(path);
                    (parsed.messages, parsed.cacheable)
                },
            );
            (path.clone(), outcome)
        })
        .collect();
    for (path, outcome) in gemini_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        } else if outcome.invalidate_cache {
            source_cache.remove(
                message_cache::CacheIdentity::for_client(ClientId::Gemini),
                &path,
            );
        }
    }

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Cursor,
        sessions::cursor::parse_cursor_file,
    );

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Mcode,
        sessions::mcode::parse_mcode_file,
    );

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Warp,
        sessions::warp::parse_warp_file,
    );

    let grok_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Grok)
        .par_iter()
        .map(|path| {
            // Use a Grok-aware fingerprint: legacy output depends on session
            // sidecars, while unified output reads metadata across the complete
            // sessions tree; all of those inputs must invalidate cached output.
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::Grok),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_grok_path_samples_only,
                sessions::grok::parse_grok_file,
            )
        })
        .collect();
    let mut grok_messages = Vec::new();
    for outcome in grok_outcomes {
        grok_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }
    let mut selected_grok_messages = sessions::grok::prefer_unified_log_messages(grok_messages);
    apply_pricing_to_messages(&mut selected_grok_messages, pricing);
    all_messages.extend(selected_grok_messages);

    let jcode_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Jcode)
        .par_iter()
        .map(|path| {
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::Jcode),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_jcode_path_samples_only,
                sessions::jcode::parse_jcode_file,
            )
        })
        .collect();
    let mut jcode_seen: HashSet<String> = HashSet::new();
    for outcome in jcode_outcomes {
        all_messages.extend(
            outcome
                .messages
                .into_iter()
                .filter(|message| should_keep_deduped_message(&mut jcode_seen, message)),
        );
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Amp,
        sessions::amp::parse_amp_file,
    );

    let codebuff_outcomes: Vec<CachedParseOutcome> = if include_codebuff {
        scan_result
            .get(ClientId::Codebuff)
            .par_iter()
            .map(|path| {
                load_or_parse_source(
                    message_cache::CacheIdentity::for_client(ClientId::Codebuff),
                    path,
                    &source_cache,
                    pricing,
                    sessions::codebuff::parse_codebuff_file,
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    for outcome in codebuff_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    // Freebuff shares Codebuff's ~/.config/manicode scan (same layout, same
    // directory — a separate product built on the same runtime). The two
    // parsers partition the shared file set under distinct cache identities:
    // codebuff emits chats with authoritative usage, freebuff emits estimated
    // rows for the rest.
    let freebuff_outcomes: Vec<CachedParseOutcome> = if include_freebuff {
        scan_result
            .get(ClientId::Codebuff)
            .par_iter()
            .map(|path| {
                load_or_parse_source(
                    message_cache::CacheIdentity::for_client(ClientId::Freebuff),
                    path,
                    &source_cache,
                    pricing,
                    sessions::freebuff::parse_freebuff_file,
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    for outcome in freebuff_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    let droid_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Droid)
        .par_iter()
        .map(|path| {
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::Droid),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_droid_path_samples_only,
                sessions::droid::parse_droid_file,
            )
        })
        .collect();
    for outcome in droid_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::OpenClaw,
        sessions::openclaw::parse_openclaw_transcript,
    );

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Pi,
        sessions::pi::parse_pi_file,
    );

    let prime_agent_outcomes: Vec<(
        CachedParseOutcome,
        sessions::prime_agent::PrimeFileAccounting,
    )> = scan_result
        .get(ClientId::PrimeAgent)
        .par_iter()
        .map(|path| load_or_parse_prime_source(path, &source_cache, pricing))
        .collect();
    let mut prime_agent_messages = Vec::new();
    let mut prime_agent_accounting = Vec::new();
    for (outcome, accounting) in prime_agent_outcomes {
        prime_agent_accounting.push(accounting);
        prime_agent_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }
    let mut prime_agent_messages = sessions::prime_agent::reconcile_prime_agent_messages(
        prime_agent_messages,
        &prime_agent_accounting,
    );
    apply_pricing_to_messages(&mut prime_agent_messages, pricing);
    all_messages.extend(prime_agent_messages);

    let kimchi_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Kimchi)
        .par_iter()
        .map(|path| {
            load_or_parse_source(
                message_cache::CacheIdentity::for_client(ClientId::Kimchi),
                path,
                &source_cache,
                pricing,
                sessions::kimchi::parse_kimchi_file,
            )
        })
        .collect();
    let mut kimchi_seen: HashSet<String> = HashSet::new();
    for outcome in kimchi_outcomes {
        all_messages.extend(
            outcome
                .messages
                .into_iter()
                .filter(|message| should_keep_deduped_message(&mut kimchi_seen, message)),
        );
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    let reasonix_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Reasonix)
        .par_iter()
        .map(|path| {
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::Reasonix),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_reasonix_path_samples_only,
                sessions::reasonix::parse_reasonix_file,
            )
        })
        .collect();
    for outcome in reasonix_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Senpi,
        sessions::senpi::parse_senpi_file,
    );

    let augment_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Augment)
        .par_iter()
        .map(|path| {
            load_or_parse_source(
                message_cache::CacheIdentity::for_client(ClientId::Augment),
                path,
                &source_cache,
                pricing,
                sessions::augment::parse_augment_file,
            )
        })
        .collect();
    let mut augment_seen: HashSet<String> = HashSet::new();
    for outcome in augment_outcomes {
        all_messages.extend(
            outcome
                .messages
                .into_iter()
                .filter(|message| should_keep_deduped_message(&mut augment_seen, message)),
        );
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    // Command Code does not persist token usage or cost locally, so tokens are
    // estimated and priced. The model id comes from ~/.commandcode/config.json
    // (canonicalized, e.g. "MiniMaxAI/MiniMax-M3-Free" -> "MiniMax-M3"), not the
    // transcript, so the source cache — which fingerprints only the transcript
    // file — is bypassed: otherwise a config.json model change would leave stale
    // cached pricing until the transcript itself changed.
    let commandcode_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::CommandCode)
        .par_iter()
        .flat_map(|path| {
            sessions::commandcode::parse_commandcode_file(path)
                .into_iter()
                .map(|mut msg| {
                    apply_pricing_if_available(&mut msg, pricing);
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(commandcode_messages);

    // gjc (gajae-code) JSONL sessions. Binding note N1: this cached cluster
    // MUST obtain messages via the non-repricing parser and apply the A1
    // Hermes guard explicitly (reprice only when the embedded usage.cost.total
    // was absent, i.e. cost <= 0.0). Routing through load_or_parse_source /
    // apply_pricing_to_messages / cached_messages would reprice unconditionally
    // and overwrite gjc's authoritative embedded cost, silently downgrading to
    // A2 on the dominant cached path. Message-level dedup via
    // should_keep_deduped_message collapses depth-1/depth-2 replays.
    let mut gjc_seen: HashSet<String> = HashSet::new();
    let gjc_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Gjc)
        .par_iter()
        .flat_map(|path| {
            sessions::gjc::parse_gjc_file(path)
                .into_iter()
                .map(|mut msg| {
                    if msg.cost <= 0.0 {
                        apply_pricing_if_available(&mut msg, pricing);
                    }
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(
        gjc_messages
            .into_iter()
            .filter(|message| should_keep_deduped_message(&mut gjc_seen, message)),
    );

    // Junie events carry authoritative per-call `modelUsage.cost` values.
    // Keep this off the generic source cache because cached_messages()
    // reprices every message unconditionally; only fill cost from pricing
    // when Junie emitted no usable cost.
    let mut junie_seen: HashSet<String> = HashSet::new();
    let junie_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Junie)
        .par_iter()
        .flat_map(|path| {
            sessions::junie::parse_junie_file(path)
                .into_iter()
                .map(|mut msg| {
                    if msg.cost <= 0.0 {
                        apply_pricing_if_available(&mut msg, pricing);
                    }
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(
        junie_messages
            .into_iter()
            .filter(|message| should_keep_deduped_message(&mut junie_seen, message)),
    );

    // ZCode v2 CLI stores authoritative model usage in SQLite.
    if let Some(db_path) = &scan_result.zcode_db {
        let CachedParseOutcome {
            messages,
            cache_entry,
            ..
        } = load_or_parse_sqlite_source(
            message_cache::CacheIdentity::for_client(ClientId::Zcode),
            db_path,
            &source_cache,
            pricing,
            sessions::zcode::parse_zcode_sqlite,
        );
        all_messages.extend(messages);
        if let Some(entry) = cache_entry {
            source_cache.insert(entry);
        }
    }

    // Cherry Studio (Electron desktop) writes standard Claude Code transcripts
    // under its app-data `.claude/projects`; parse them with the shared
    // claudecode logic re-tagged as `cherrystudio`.
    let cherrystudio_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::CherryStudio)
        .par_iter()
        .map(|path| {
            load_or_parse_source(
                message_cache::CacheIdentity::for_client(ClientId::CherryStudio),
                path,
                &source_cache,
                pricing,
                sessions::cherrystudio::parse_cherrystudio_file,
            )
        })
        .collect();
    for outcome in cherrystudio_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    // DeepSeek Harness (DSH) zstd JSONL transcripts. Every `assistant/message`
    // carries authoritative usage but never a cost, so pricing is the only cost
    // source — the generic source cache (which reprices unconditionally) is
    // safe here, same as opencodereview. Forking copies the parent's completed
    // prefix into the child transcript verbatim, so the lane also needs one
    // cross-file dedup pass on the per-call `message.id`.
    parse_cached_lane_deduped(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Dsh,
        sessions::dsh::parse_dsh_file,
    );

    // ZCode (Z.ai GLM-5.2 ADE) JSONL sessions. Token usage may be embedded
    // from the API response; otherwise estimated from content.
    let zcode_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Zcode)
        .par_iter()
        .flat_map(|path| {
            sessions::zcode::parse_zcode_file(path)
                .into_iter()
                .map(|mut msg| {
                    apply_pricing_if_available(&mut msg, pricing);
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(zcode_messages);

    // opencodereview `llm_response` records carry usage but never a cost, so
    // every message leaves the parser at 0.0 and pricing is its only cost
    // source. That makes the generic source cache safe here — unlike Junie
    // above, there is no authoritative embedded cost for cached_messages()'s
    // unconditional reprice to overwrite. The parser also dedups within a file
    // on its own, so no cross-file `should_keep_deduped_message` pass is needed.
    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::OpenCodeReview,
        sessions::opencodereview::parse_opencodereview_file,
    );

    let kimi_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Kimi)
        .par_iter()
        .map(|path| {
            let parse: fn(&Path) -> Vec<UnifiedMessage> = if sessions::kimi::is_kimi_code_path(path)
            {
                sessions::kimi::parse_kimi_code_file
            } else {
                sessions::kimi::parse_kimi_file
            };
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::Kimi),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_kimi_path_samples_only,
                parse,
            )
        })
        .collect();
    for outcome in kimi_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    // Parse Qwen files
    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Qwen,
        sessions::qwen::parse_qwen_file,
    );

    let roocode_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::RooCode)
        .par_iter()
        .map(|path| {
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::RooCode),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_roo_path_samples_only,
                sessions::roocode::parse_roocode_file,
            )
        })
        .collect();
    for outcome in roocode_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    let kilocode_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::KiloCode)
        .par_iter()
        .map(|path| {
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::KiloCode),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_roo_path_samples_only,
                sessions::kilocode::parse_kilocode_file,
            )
        })
        .collect();
    for outcome in kilocode_outcomes {
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    let cline_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Cline)
        .par_iter()
        .map(|path| {
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::Cline),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_cline_path_samples_only,
                sessions::cline::parse_cline_file,
            )
        })
        .collect();
    let mut cline_seen: HashSet<String> = HashSet::new();
    for outcome in cline_outcomes {
        all_messages.extend(
            outcome
                .messages
                .into_iter()
                .filter(|message| should_keep_deduped_message(&mut cline_seen, message)),
        );
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Mux,
        sessions::mux::parse_mux_file,
    );

    // fx (vercel-labs/fx) per-session usage snapshots
    // (`~/.fx/sessions/<id>/usage-v2.json`).
    parse_cached_lane(
        &scan_result,
        &mut source_cache,
        pricing,
        &mut all_messages,
        ClientId::Fx,
        sessions::fx::parse_fx_file,
    );

    // Kilo CLI: SQLite database
    if let Some(db_path) = &scan_result.kilo_db {
        let kilo_messages: Vec<UnifiedMessage> = sessions::kilo::parse_kilo_sqlite(db_path)
            .into_iter()
            .map(|mut msg| {
                apply_pricing_if_available(&mut msg, pricing);
                msg
            })
            .collect();
        all_messages.extend(kilo_messages);
    }

    let mut hermes_seen: HashSet<String> = HashSet::new();
    for db_path in scan_result.hermes_db_paths() {
        let hermes_messages = parse_hermes_sqlite_with_pricing(&db_path, pricing);
        all_messages.extend(
            hermes_messages
                .into_iter()
                .filter(|message| should_keep_deduped_message(&mut hermes_seen, message)),
        );
    }

    if let Some(db_path) = &scan_result.goose_db {
        let goose_messages: Vec<UnifiedMessage> = sessions::goose::parse_goose_sqlite(db_path)
            .into_iter()
            .map(|mut msg| {
                apply_pricing_if_available(&mut msg, pricing);
                msg
            })
            .collect();
        all_messages.extend(goose_messages);
    }

    // Devin CLI stores authoritative model usage in SQLite. Multiple paths can
    // be configured through scanner extra roots, so parse and dedupe all of
    // them instead of silently ignoring non-default databases.
    let mut devin_cli_session_ids: HashSet<String> = HashSet::new();
    if include_devin_cli {
        let devin_cli_outcomes: Vec<CachedParseOutcome> = scan_result
            .devin_dbs
            .par_iter()
            .map(|db_path| {
                load_or_parse_sqlite_source(
                    message_cache::CacheIdentity::for_client(ClientId::DevinCli),
                    db_path,
                    &source_cache,
                    pricing,
                    sessions::devin::parse_devin_cli_sqlite,
                )
            })
            .collect();
        let mut devin_cli_seen = HashSet::new();
        for outcome in devin_cli_outcomes {
            for message in outcome
                .messages
                .into_iter()
                .filter(|message| should_keep_deduped_message(&mut devin_cli_seen, message))
            {
                devin_cli_session_ids.insert(message.session_id.clone());
                all_messages.push(message);
            }
            if let Some(entry) = outcome.cache_entry {
                source_cache.insert(entry);
            }
        }
    }

    for db_path in scan_result.zed_db_paths() {
        let outcome = load_or_parse_sqlite_source(
            message_cache::CacheIdentity::for_client(ClientId::Zed),
            &db_path,
            &source_cache,
            pricing,
            sessions::zed::parse_zed_sqlite,
        );
        all_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    let kiro_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::Kiro)
        .par_iter()
        .map(|path| {
            // Kiro-aware fingerprint: IDE `sess_*/session.json` sources derive
            // their token counts from the sibling `messages.jsonl`, so that
            // file must participate in the cache key or an append landing
            // after the last `session.json` write is ignored forever.
            load_or_parse_source_with_fingerprint(
                message_cache::CacheIdentity::for_client(ClientId::Kiro),
                path,
                &source_cache,
                pricing,
                message_cache::SourceFingerprint::check_kiro_path_samples_only,
                sessions::kiro::parse_kiro_file,
            )
        })
        .collect();
    // Collect Kiro file messages before extending so snapshot suppression can
    // see execution coverage across files (it is a cross-file merge concern,
    // like merge_workbuddy_messages, and must run after cache loads).
    let mut kiro_file_messages: Vec<UnifiedMessage> = Vec::new();
    for outcome in kiro_outcomes {
        kiro_file_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }
    all_messages.extend(sessions::kiro::suppress_snapshots_covered_by_executions(
        kiro_file_messages,
    ));

    if let Some(db_path) = &scan_result.kiro_db {
        let kiro_db_messages: Vec<UnifiedMessage> = sessions::kiro::parse_kiro_sqlite(db_path)
            .into_iter()
            .map(|mut msg| {
                apply_pricing_if_available(&mut msg, pricing);
                msg
            })
            .collect();
        all_messages.extend(kiro_db_messages);
    }

    // Crush decides its day split at parse time (it allocates session cost
    // across days), so it needs the pinned zone here — the post-parse
    // `rebucket_days` pass cannot recover a split this grouping collapsed.
    let crush_bucket_timezone = bucket_tz::BucketTimezone::from_scanner_settings(scanner_settings);
    for source in &scan_result.crush_dbs {
        let crush_messages: Vec<UnifiedMessage> =
            sessions::crush::parse_crush_sqlite_in(&source.db_path, &crush_bucket_timezone)
                .into_iter()
                .map(|mut msg| {
                    msg.set_workspace(source.workspace_key.clone(), source.workspace_label.clone());
                    apply_pricing_if_available(&mut msg, pricing);
                    msg
                })
                .collect();
        all_messages.extend(crush_messages);
    }

    let antigravity_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Antigravity)
        .par_iter()
        .flat_map(|path| {
            sessions::antigravity::parse_antigravity_file(path)
                .into_iter()
                .map(|mut msg| {
                    apply_pricing_if_available(&mut msg, pricing);
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(antigravity_messages);

    let antigravity_cli_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::AntigravityCli)
        .par_iter()
        .flat_map(|path| {
            sessions::antigravity_cli::parse_antigravity_cli_file(path)
                .into_iter()
                .map(|mut msg| {
                    apply_pricing_if_available(&mut msg, pricing);
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(antigravity_cli_messages);

    // Trae API dump uses exact dollar_float totals, so pricing lookup is not needed.
    let trae_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Trae)
        .par_iter()
        .flat_map(|path| sessions::trae::parse_trae_file("trae", path))
        .collect();
    let deduped_trae_messages = dedupe_latest_trae_messages(trae_messages);
    all_messages.extend(deduped_trae_messages);

    let codebuddy_outcomes: Vec<CachedParseOutcome> = scan_result
        .get(ClientId::CodeBuddy)
        .par_iter()
        .map(|path| {
            load_or_parse_source(
                message_cache::CacheIdentity::for_client(ClientId::CodeBuddy),
                path,
                &source_cache,
                pricing,
                sessions::codebuddy::parse_codebuddy_file,
            )
        })
        .collect();
    let mut codebuddy_seen: HashSet<String> = HashSet::new();
    for outcome in codebuddy_outcomes {
        all_messages.extend(outcome.messages.into_iter().filter(|message| {
            message
                .dedup_key
                .as_ref()
                .is_none_or(|key| codebuddy_seen.insert(key.clone()))
        }));
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }

    // Devin Desktop ACP file names are unrelated to the CLI database session
    // ids. Resolve their session titles through the database so the CLI can
    // take precedence only when both sources really describe one session.
    if include_devin_desktop {
        // Lookups are constructed only for cache misses. Key them by the
        // post-validation database snapshot so parallel misses that observe
        // different SQLite states never share stale metadata; identical
        // snapshots still share one query on a cold scan.
        let devin_desktop_lookups = DevinDesktopLookupCache::default();
        let devin_desktop_outcomes: Vec<CachedParseOutcome> = scan_result
            .get(ClientId::DevinDesktop)
            .par_iter()
            .map(|path| {
                load_or_parse_source_with_fingerprint_context(
                    message_cache::CacheIdentity::for_client(ClientId::DevinDesktop),
                    path,
                    &source_cache,
                    pricing,
                    |path, cached| {
                        message_cache::SourceFingerprint::check_devin_desktop_path_samples_only(
                            path,
                            &scan_result.devin_dbs,
                            cached,
                        )
                    },
                    |path, fingerprint| {
                        if let Some(fingerprint) = fingerprint {
                            let lookup_cell = devin_desktop_lookup_cell_for_snapshot(
                                &devin_desktop_lookups,
                                &scan_result.devin_dbs,
                                fingerprint,
                            );
                            let lookup = lookup_cell.get_or_init(|| {
                                sessions::devin::load_devin_desktop_session_lookup(
                                    &scan_result.devin_dbs,
                                )
                            });
                            sessions::devin::parse_devin_desktop_ndjson_with_lookup(path, lookup)
                        } else {
                            // Unreadable sources cannot produce a cache entry,
                            // so they do not need a snapshot-keyed lookup.
                            sessions::devin::parse_devin_desktop_ndjson_with_lookup(
                                path,
                                &sessions::devin::load_devin_desktop_session_lookup(
                                    &scan_result.devin_dbs,
                                ),
                            )
                        }
                    },
                )
            })
            .collect();
        for outcome in devin_desktop_outcomes {
            all_messages.extend(
                outcome
                    .messages
                    .into_iter()
                    .filter(|message| !devin_cli_session_ids.contains(&message.session_id)),
            );
            if let Some(entry) = outcome.cache_entry {
                source_cache.insert(entry);
            }
        }
    }

    let (workbuddy_detailed_paths, workbuddy_fallback_paths) =
        partition_workbuddy_paths(scan_result.get(ClientId::WorkBuddy));
    let workbuddy_detailed_outcomes: Vec<CachedParseOutcome> = workbuddy_detailed_paths
        .par_iter()
        .map(|path| {
            load_or_parse_source(
                message_cache::CacheIdentity::for_client(ClientId::WorkBuddy),
                path,
                &source_cache,
                pricing,
                sessions::workbuddy::parse_workbuddy_file,
            )
        })
        .collect();
    let workbuddy_fallback_outcomes: Vec<CachedParseOutcome> = workbuddy_fallback_paths
        .par_iter()
        .map(|path| {
            load_or_parse_sqlite_source(
                message_cache::CacheIdentity::for_client(ClientId::WorkBuddy),
                path,
                &source_cache,
                pricing,
                sessions::workbuddy::parse_workbuddy_file,
            )
        })
        .collect();
    let mut workbuddy_detailed_messages = Vec::new();
    for outcome in workbuddy_detailed_outcomes {
        workbuddy_detailed_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }
    let mut workbuddy_fallback_messages = Vec::new();
    for outcome in workbuddy_fallback_outcomes {
        workbuddy_fallback_messages.extend(outcome.messages);
        if let Some(entry) = outcome.cache_entry {
            source_cache.insert(entry);
        }
    }
    all_messages.extend(merge_workbuddy_messages(
        workbuddy_detailed_messages,
        workbuddy_fallback_messages,
    ));

    if include_synthetic {
        if let Some(db_path) = &scan_result.synthetic_db {
            let outcome = load_or_parse_sqlite_source(
                message_cache::CacheIdentity::synthetic(),
                db_path,
                &source_cache,
                pricing,
                sessions::synthetic::parse_octofriend_sqlite,
            );
            all_messages.extend(outcome.messages);
            if let Some(entry) = outcome.cache_entry {
                source_cache.insert(entry);
            }
        }
    }

    // Filter BEFORE normalization so retain_for_requested_clients can see
    // original model/provider prefixes (e.g. "accounts/fireworks/models/…")
    // that is_synthetic_gateway relies on for gateway detection.
    if !include_all {
        let requested: HashSet<&str> = clients.iter().map(String::as_str).collect();
        all_messages.retain(|msg| {
            retain_for_requested_clients(&msg.client, &msg.model_id, &msg.provider_id, &requested)
        });
    }

    if include_synthetic {
        for msg in &mut all_messages {
            sessions::synthetic::normalize_synthetic_gateway_fields(
                &mut msg.model_id,
                &mut msg.provider_id,
            );
        }
    }

    if cache_policy == SourceCachePolicy::Persistent {
        source_cache.save_if_dirty();
    }

    rebucket_days(&mut all_messages, scanner_settings);

    all_messages
}

/// Re-key every message onto the device's pinned bucketing timezone.
///
/// The parsers derive `date` from `chrono::Local`, read afresh on every scan,
/// so which day a message lands in changes when the machine's zone does. This
/// is the one pass that knows the user's settings and sees every message, so it
/// is where the day key gets fixed to something a rescan cannot move.
///
/// Runs after the source cache is written on purpose: the cache stores raw
/// parser output and `refresh_derived_fields` re-derives `date` on every load,
/// so cached entries never carry a stale day key past this point and changing
/// the pinned zone needs no cache invalidation.
///
/// **No-op when nothing is pinned.** An unpinned device must report exactly
/// what it reported before, so the pass is skipped rather than re-derived
/// through `Local`.
fn rebucket_days(messages: &mut [UnifiedMessage], scanner_settings: &scanner::ScannerSettings) {
    let timezone = bucket_tz::BucketTimezone::from_scanner_settings(scanner_settings);
    if !timezone.is_pinned() {
        return;
    }

    for message in messages.iter_mut() {
        message.rebucket_date(&timezone);
    }
}

fn dedupe_latest_trae_messages(mut messages: Vec<UnifiedMessage>) -> Vec<UnifiedMessage> {
    let mut latest_by_session: HashMap<String, UnifiedMessage> = HashMap::new();

    for message in messages.drain(..) {
        let session_id = message.session_id.clone();
        match latest_by_session.get_mut(&session_id) {
            Some(existing) => {
                let should_replace = message.timestamp > existing.timestamp
                    || (message.timestamp == existing.timestamp
                        && message.dedup_key.as_ref().is_some_and(|key| {
                            existing
                                .dedup_key
                                .as_ref()
                                .is_none_or(|existing_key| key > existing_key)
                        }));
                if should_replace {
                    *existing = message;
                }
            }
            None => {
                let _ = latest_by_session.insert(session_id, message);
            }
        }
    }

    let mut deduped: Vec<UnifiedMessage> = latest_by_session.into_values().collect();
    deduped.sort_unstable_by(|a, b| {
        a.session_id
            .cmp(&b.session_id)
            .then_with(|| a.timestamp.cmp(&b.timestamp))
    });
    deduped
}

fn partition_workbuddy_paths(paths: &[PathBuf]) -> (Vec<&PathBuf>, Vec<&PathBuf>) {
    paths
        .iter()
        .partition(|path| sessions::workbuddy::is_detailed_workbuddy_source(path))
}

fn merge_workbuddy_messages(
    detailed_messages: Vec<UnifiedMessage>,
    fallback_messages: Vec<UnifiedMessage>,
) -> Vec<UnifiedMessage> {
    // The SQLite fallback carries ONE cumulative row per session (dated solely by
    // `updated_at`), while the detailed JSONL carries accurate per-message rows.
    // A fallback row is redundant exactly when its session already has detailed
    // coverage — independent of which calendar day `updated_at` lands on. Keying
    // this on the session (not the date) fixes two failures of the old
    // date-overlap check: it no longer double-counts a session whose aggregate
    // lands on a day with no detailed rows, and no longer drops a fallback-only
    // session that merely shares a day with unrelated detailed activity. Both
    // parsers derive `session_id` from the same WorkBuddy session identifier, so
    // the keys are directly comparable.
    let detailed_sessions: HashSet<String> = detailed_messages
        .iter()
        .filter(|message| !message.session_id.is_empty())
        .map(|message| message.session_id.clone())
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<UnifiedMessage> = detailed_messages
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut seen, message))
        .collect();

    merged.extend(fallback_messages.into_iter().filter(|message| {
        !detailed_sessions.contains(&message.session_id)
            && should_keep_deduped_message(&mut seen, message)
    }));
    merged
}

fn filter_unified_messages(
    messages: Vec<UnifiedMessage>,
    options: &LocalParseOptions,
) -> Vec<UnifiedMessage> {
    let mut filtered = messages;

    if let Some(year) = &options.year {
        let year_prefix = format!("{}-", year);
        filtered.retain(|m| m.date.starts_with(&year_prefix));
    }

    if let Some(since) = &options.since {
        filtered.retain(|m| m.date.as_str() >= since.as_str());
    }

    if let Some(until) = &options.until {
        filtered.retain(|m| m.date.as_str() <= until.as_str());
    }

    filtered
}

/// How workspace rows treat git worktrees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub enum WorktreeRollup {
    /// One row per worktree — a task-isolating agent CLI produces many rows per repo.
    #[default]
    Separate,
    /// Fold every worktree into its parent repository.
    MergeIntoRepo,
}

/// Resolving a workspace key to a display label reads the filesystem (see
/// [`sessions::decode_claude_project_slug`]). Reports iterate hundreds of
/// thousands of messages over a handful of distinct workspaces, so memoize per
/// key and keep the syscalls proportional to workspaces, not messages.
#[derive(Default)]
pub struct WorkspaceLabeler {
    labels: HashMap<String, String>,
    roots: HashMap<String, Option<String>>,
    paths: HashMap<String, Option<String>>,
    decoded: HashMap<String, Option<String>>,
    resolved_roots: HashMap<String, Option<String>>,
}

impl WorkspaceLabeler {
    pub fn label(&mut self, key: &str) -> String {
        if let Some(cached) = self.labels.get(key) {
            return cached.clone();
        }
        let decoded = self.decoded(key);
        let label = sessions::workspace_display_label_for_decoded_key(key, decoded.as_deref())
            .unwrap_or_else(|| UNKNOWN_WORKSPACE_LABEL.to_string());
        self.labels.insert(key.to_string(), label.clone());
        label
    }

    /// The real filesystem path `key` names, decoded from Claude Code's slug
    /// where it is one. `None` when the key is not a path (an opaque client id)
    /// or its directory is gone.
    pub fn path(&mut self, key: &str) -> Option<String> {
        if let Some(cached) = self.paths.get(key) {
            return cached.clone();
        }
        let decoded = self.decoded(key);
        let path = sessions::workspace_path_for_decoded_key(key, decoded.as_deref());
        self.paths.insert(key.to_string(), path.clone());
        path
    }

    /// Claude Code's slug decoded to a real path, memoized.
    ///
    /// The decode is the expensive half of every method here: it walks the
    /// filesystem, backtracking over the ambiguity in the dash encoding. Without
    /// this cache `label`, `path` and `repo_root` each ran their own walk for the
    /// same key, so a slug that took 7s to decode cost 23s across one row.
    fn decoded(&mut self, key: &str) -> Option<String> {
        if let Some(cached) = self.decoded.get(key) {
            return cached.clone();
        }
        let decoded = sessions::decode_claude_project_slug(key);
        self.decoded.insert(key.to_string(), decoded.clone());
        decoded
    }

    /// Distinct keys whose slug decode has been resolved, for tests that need to
    /// prove the walk is shared across `label`, `path` and `repo_root`.
    #[cfg(test)]
    pub(crate) fn decoded_key_count(&self) -> usize {
        self.decoded.len()
    }

    /// The repo root a resolved filesystem path belongs to, memoized.
    ///
    /// Distinct from [`Self::repo_root`], which is keyed by the workspace key and
    /// applies the slug fallbacks. This one is keyed by path because it reads the
    /// `.git` pointer file, and several keys can resolve to the same directory.
    pub fn repo_root_of_path(&mut self, path: &str) -> Option<String> {
        if let Some(cached) = self.resolved_roots.get(path) {
            return cached.clone();
        }
        let root = sessions::workspace_repo_root_resolved(path);
        self.resolved_roots.insert(path.to_string(), root.clone());
        root
    }

    /// The canonical repo identity for `key`: the real filesystem path, with any
    /// worktree suffix stripped. `None` when the key cannot be resolved to a path
    /// (an opaque client id, or a directory no longer on disk), leaving the
    /// original key as its own identity.
    ///
    /// Decoding is what makes the rollup actually merge. Claude Code writes a
    /// dash-mangled slug and Codex/OpenCode write real paths, so without this the
    /// same repo keeps two identities and the "one row per repo" promise fails.
    pub fn repo_root(&mut self, key: &str) -> Option<String> {
        if let Some(cached) = self.roots.get(key) {
            return cached.clone();
        }
        // Decode first: Claude's slug encodes `.claude/worktrees/` as dashes, so
        // the marker is only visible once the real path is recovered.
        let decoded = self.decoded(key);
        let path = decoded.clone().unwrap_or_else(|| key.to_string());
        let root = self
            .repo_root_of_path(&path)
            // Not a worktree: the decoded path is already the repo identity.
            .or_else(|| decoded.clone())
            // Undecodable slug (deleted worktree): fall back to the repo prefix
            // recovered from the slug string so it still merges with its repo.
            .or_else(|| sessions::workspace_repo_root_from_slug(key));
        self.roots.insert(key.to_string(), root.clone());
        root
    }
}

/// Grouping key, stored key and display label for a message's workspace.
///
/// Shared with the TUI, which runs its own aggregation over the same messages —
/// duplicating this would let the two drift on how worktrees roll up and how
/// Claude Code's dash-mangled keys are labeled.
pub fn workspace_bucket(
    msg: &UnifiedMessage,
    rollup: WorktreeRollup,
    labeler: &mut WorkspaceLabeler,
) -> (String, Option<String>, String) {
    let Some(key) = msg.workspace_key.as_deref() else {
        return (
            UNKNOWN_WORKSPACE_GROUP_KEY.to_string(),
            None,
            UNKNOWN_WORKSPACE_LABEL.to_string(),
        );
    };

    // Under MergeIntoRepo the repo root becomes the grouping identity, so every
    // worktree of a repo lands in one row and the row reports the repo's path.
    if rollup == WorktreeRollup::MergeIntoRepo {
        if let Some(root) = labeler.repo_root(key) {
            let label = labeler.label(&root);
            return (root.clone(), Some(root), label);
        }
    }

    // A parser-supplied label is authoritative — it is the only thing that can
    // name a workspace whose key is not a path (Warp's workspace UUID). Keys
    // that fell back to `workspace_label_from_key` are relabeled, because that
    // helper returns the whole dash-mangled slug for Claude Code.
    let label = match msg.workspace_label.as_deref() {
        Some(label) if Some(label.to_string()) != sessions::workspace_label_from_key(key) => {
            label.to_string()
        }
        _ => labeler.label(key),
    };

    (key.to_string(), Some(key.to_string()), label)
}

/// The label to display for every distinct workspace in `messages`, keyed by the
/// grouping identity its rows will use.
///
/// A label is a basename, so `~/work/api` and `~/oss/api` render as the same
/// text even though they stay separate rows with separate keys — the row is
/// still correct, but the reader cannot tell which repo it is looking at. Each
/// colliding label is qualified here with the fewest leading parent segments
/// that tell the group apart (`work/api`, `oss/api`).
///
/// Grouping keys are never touched: this rewrites display text only, so no usage
/// moves between rows and no total changes.
///
/// Resolved up front rather than as a post-pass over the rows: the daily
/// breakdown keys its legend off the label while it aggregates, so fixing the
/// table afterwards would leave the chart showing the ambiguous name.
pub fn workspace_label_overrides(
    messages: &[UnifiedMessage],
    rollup: WorktreeRollup,
    labeler: &mut WorkspaceLabeler,
) -> HashMap<String, String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut base: BTreeMap<String, String> = BTreeMap::new();
    for msg in messages {
        let Some(key) = msg.workspace_key.as_deref() else {
            continue;
        };
        if !seen.insert(key) {
            continue;
        }
        let (group_key, _, label) = workspace_bucket(msg, rollup, labeler);
        base.entry(group_key).or_insert(label);
    }

    disambiguate_workspace_labels(
        base.iter()
            .map(|(key, label)| (key.as_str(), label.as_str())),
        labeler,
    )
}

/// Rewrite `labeled` — (grouping key, base label) pairs — so no two keys share a
/// label, qualifying each collision with as few leading path segments as it
/// takes and falling back to the grouping key when the filesystem cannot
/// separate them at all.
fn disambiguate_workspace_labels<'a>(
    labeled: impl IntoIterator<Item = (&'a str, &'a str)>,
    labeler: &mut WorkspaceLabeler,
) -> HashMap<String, String> {
    // BTree everywhere: with two directories that encode identically there is
    // nothing on disk to order them by, so the output must not depend on hash
    // iteration order.
    let mut by_label: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (key, label) in labeled {
        by_label.entry(label).or_default().insert(key);
    }

    let mut resolved: BTreeMap<String, String> = BTreeMap::new();
    for (label, keys) in by_label {
        if keys.len() == 1 {
            for key in keys {
                resolved.insert(key.to_string(), label.to_string());
            }
            continue;
        }

        let keys: Vec<&str> = keys.into_iter().collect();
        let parents: Vec<Vec<String>> = keys
            .iter()
            .map(|key| workspace_parent_segments(labeler, key))
            .collect();
        let max_depth = parents.iter().map(Vec::len).max().unwrap_or(0);

        // Fewest segments that tell the most rows apart. Escalating past that
        // buys nothing: when two keys name the SAME directory every remaining
        // segment is identical on both rows, so a deeper qualifier only makes
        // the label longer and pushes the part that actually differs — the key
        // appended below — off a narrow row.
        let mut depth = 0;
        let mut separated = 0;
        for candidate in 0..=max_depth {
            let candidates: HashSet<String> = parents
                .iter()
                .map(|parents| qualify_workspace_label(label, parents, candidate))
                .collect();
            if candidates.len() > separated {
                separated = candidates.len();
                depth = candidate;
            }
            if separated == keys.len() {
                break;
            }
        }

        for (key, parents) in keys.iter().zip(&parents) {
            resolved.insert(
                (*key).to_string(),
                qualify_workspace_label(label, parents, depth),
            );
        }
    }

    // Whatever the filesystem could not separate — two keys that resolve to the
    // same directory, or keys with no path at all — is separated by the grouping
    // key, which is unique by construction.
    let mut duplicates: BTreeMap<&str, usize> = BTreeMap::new();
    for label in resolved.values() {
        *duplicates.entry(label.as_str()).or_default() += 1;
    }
    let ambiguous: HashSet<String> = duplicates
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(label, _)| label.to_string())
        .collect();

    resolved
        .into_iter()
        .map(|(key, label)| {
            if ambiguous.contains(&label) {
                let qualified = format!("{label} ({key})");
                (key, qualified)
            } else {
                (key, label)
            }
        })
        .collect()
}

/// Parent segments of the directory whose name the label leads with, nearest
/// first. Empty when the key resolves to no path, which is what makes the
/// caller fall through to qualifying by the key itself.
fn workspace_parent_segments(labeler: &mut WorkspaceLabeler, key: &str) -> Vec<String> {
    let Some(path) = labeler.path(key) else {
        return Vec::new();
    };
    // A worktree label reads `repo ⑃ worktree`, so it is the REPO whose parents
    // disambiguate it, not the worktree's `.claude/worktrees` scaffolding.
    let anchor = labeler.repo_root_of_path(&path).unwrap_or(path);
    let mut segments: Vec<String> = anchor
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect();
    // The last segment is already the label's own name.
    segments.pop();
    segments.reverse();
    segments
}

/// `label` prefixed with up to `depth` parent segments, outermost first, so the
/// result reads like the tail of the path it came from.
fn qualify_workspace_label(label: &str, parents: &[String], depth: usize) -> String {
    let taken = depth.min(parents.len());
    if taken == 0 {
        return label.to_string();
    }
    let mut prefix: Vec<&str> = parents[..taken].iter().map(String::as_str).collect();
    prefix.reverse();
    format!("{}/{label}", prefix.join("/"))
}

#[cfg(test)]
fn aggregate_model_usage_entries(
    messages: Vec<UnifiedMessage>,
    group_by: &GroupBy,
) -> Vec<ModelUsage> {
    aggregate_model_usage_entries_with_rollup(messages, group_by, WorktreeRollup::default())
}

fn aggregate_model_usage_entries_with_rollup(
    messages: Vec<UnifiedMessage>,
    group_by: &GroupBy,
    rollup: WorktreeRollup,
) -> Vec<ModelUsage> {
    let mut model_map: HashMap<String, ModelUsage> = HashMap::new();
    let mut labeler = WorkspaceLabeler::default();

    // Bucketing a workspace resolves its label, which reads the filesystem. Every
    // other grouping discards that label a few lines below, so skip the work rather
    // than paying it on `tokscale --light`, `monthly`, and every TUI refresh.
    let needs_workspace = matches!(group_by, GroupBy::WorkspaceModel);
    let label_overrides = if needs_workspace {
        workspace_label_overrides(&messages, rollup, &mut labeler)
    } else {
        HashMap::new()
    };

    for msg in messages {
        let normalized = model_name_for_grouping(&msg.client, &msg.provider_id, &msg.model_id);
        let (workspace_group_key, workspace_key, workspace_label) = if needs_workspace {
            let (group_key, key, label) = workspace_bucket(&msg, rollup, &mut labeler);
            let label = label_overrides.get(&group_key).cloned().unwrap_or(label);
            (group_key, key, label)
        } else {
            (String::new(), None, String::new())
        };
        let key = match group_by {
            GroupBy::Model => normalized.clone(),
            GroupBy::ClientModel => format!("{}:{}", msg.client, normalized),
            GroupBy::ClientProviderModel => {
                format!("{}:{}:{}", msg.client, msg.provider_id, normalized)
            }
            GroupBy::WorkspaceModel => format!("{}:{}", workspace_group_key, normalized),
            GroupBy::Session => format!("{}:{}", msg.session_id, normalized),
            GroupBy::ClientSession => {
                format!("{}:{}:{}", msg.client, msg.session_id, normalized)
            }
        };
        let merge_clients = matches!(group_by, GroupBy::Model | GroupBy::WorkspaceModel);
        let session_grouped = matches!(group_by, GroupBy::Session | GroupBy::ClientSession);
        let entry = model_map.entry(key).or_insert_with(|| ModelUsage {
            client: msg.client.clone(),
            merged_clients: if merge_clients {
                Some(msg.client.clone())
            } else {
                None
            },
            workspace_key: if matches!(group_by, GroupBy::WorkspaceModel) {
                workspace_key.clone()
            } else {
                None
            },
            workspace_label: if matches!(group_by, GroupBy::WorkspaceModel) {
                Some(workspace_label.clone())
            } else {
                None
            },
            session_id: if session_grouped {
                Some(msg.session_id.clone())
            } else {
                None
            },
            model: normalized.clone(),
            provider: msg.provider_id.clone(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 0,
            cost: 0.0,
            performance: ModelPerformance::default(),
        });

        if merge_clients {
            if !entry.client.split(", ").any(|s| s == msg.client) {
                entry.client = format!("{}, {}", entry.client, msg.client);
            }

            if let Some(merged_clients) = &mut entry.merged_clients {
                if !merged_clients.split(", ").any(|s| s == msg.client) {
                    *merged_clients = format!("{}, {}", merged_clients, msg.client);
                }
            }
        }

        if *group_by != GroupBy::ClientProviderModel
            && !entry.provider.split(", ").any(|p| p == msg.provider_id)
        {
            entry.provider = format!("{}, {}", entry.provider, msg.provider_id);
        }

        // saturating_add so clamped (i64::MAX) buckets from a corrupt source
        // can't overflow the fold (matches the grand-total sum below).
        entry.input = entry.input.saturating_add(msg.tokens.input);
        entry.output = entry.output.saturating_add(msg.tokens.output);
        entry.cache_read = entry.cache_read.saturating_add(msg.tokens.cache_read);
        entry.cache_write = entry.cache_write.saturating_add(msg.tokens.cache_write);
        entry.reasoning = entry.reasoning.saturating_add(msg.tokens.reasoning);
        entry.message_count += msg.message_count.max(0);
        entry.cost += msg.cost;
        entry
            .performance
            .record_message(positive_token_total(&msg.tokens), msg.duration_ms);
    }

    let mut entries: Vec<ModelUsage> = model_map
        .into_values()
        .map(|mut entry| {
            let total_tokens = entry
                .input
                .max(0)
                .saturating_add(entry.output.max(0))
                .saturating_add(entry.cache_read.max(0))
                .saturating_add(entry.cache_write.max(0))
                .saturating_add(entry.reasoning.max(0));
            entry.performance.finalize(total_tokens);
            let mut providers: Vec<&str> = entry.provider.split(", ").collect();
            providers.sort_unstable();
            providers.dedup();
            entry.provider = providers.join(", ");
            entry
        })
        .collect();
    entries.sort_by(|a, b| match (a.cost.is_nan(), b.cost.is_nan()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => b
            .cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal),
    });

    entries
}

fn positive_token_total(tokens: &TokenBreakdown) -> i64 {
    // saturating so multiple clamped (i64::MAX) buckets can't overflow the sum.
    tokens
        .input
        .max(0)
        .saturating_add(tokens.output.max(0))
        .saturating_add(tokens.cache_read.max(0))
        .saturating_add(tokens.cache_write.max(0))
        .saturating_add(tokens.reasoning.max(0))
}

/// Sum the (input, output, cache_read, cache_write) token fields across model
/// usage entries with saturating_add, so clamped (i64::MAX) entry buckets from a
/// corrupt source can't overflow the report-level totals (the entries are
/// already saturated per-field by aggregate_model_usage_entries).
fn model_report_token_totals(entries: &[ModelUsage]) -> (i64, i64, i64, i64) {
    entries.iter().fold(
        (0, 0, 0, 0),
        |(input, output, cache_read, cache_write), entry| {
            (
                input.saturating_add(entry.input),
                output.saturating_add(entry.output),
                cache_read.saturating_add(entry.cache_read),
                cache_write.saturating_add(entry.cache_write),
            )
        },
    )
}

pub async fn get_model_report(options: ReportOptions) -> Result<ModelReport, String> {
    let start = Instant::now();

    let home_dir = get_home_dir_string(&options.home_dir)?;

    let clients: Vec<String> = options.clients.clone().unwrap_or_else(|| {
        let mut clients: Vec<String> = ClientId::ALL
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());
        clients
    });

    let pricing = load_pricing_for_local_parse().await;
    let all_messages = parse_all_messages_with_pricing_with_env_strategy(
        &home_dir,
        &clients,
        pricing.as_deref(),
        options.use_env_roots,
        &options.scanner_settings,
    );

    let filtered = filter_messages_for_report(all_messages, &options);
    let entries = aggregate_model_usage_entries_with_rollup(
        filtered,
        &options.group_by,
        options.worktree_rollup,
    );

    let (total_input, total_output, total_cache_read, total_cache_write) =
        model_report_token_totals(&entries);
    let total_messages: i32 = entries.iter().map(|e| e.message_count).sum();
    // f64's Sum identity is -0.0, so an empty report would serialize as
    // "totalCost": -0.0; adding +0.0 normalizes the sign without changing
    // any non-zero total.
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum::<f64>() + 0.0;

    Ok(ModelReport {
        entries,
        total_input,
        total_output,
        total_cache_read,
        total_cache_write,
        total_messages,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

#[derive(Default)]
struct MonthAggregator {
    models: HashSet<String>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    message_count: i32,
    cost: f64,
}

fn aggregate_monthly_usage_v2_entries(
    messages: impl IntoIterator<Item = UnifiedMessage>,
) -> Vec<MonthlyUsageV2> {
    let mut month_map: HashMap<String, MonthAggregator> = HashMap::new();

    for msg in messages {
        let Ok(date) = chrono::NaiveDate::parse_from_str(&msg.date, "%Y-%m-%d") else {
            continue;
        };
        let month = date.format("%Y-%m").to_string();

        let entry = month_map.entry(month).or_default();

        entry.models.insert(model_name_for_grouping(
            &msg.client,
            &msg.provider_id,
            &msg.model_id,
        ));
        // Saturating arithmetic matches the model/hourly aggregators: parser
        // clamps can legitimately produce i64::MAX, and a corrupt source must
        // not make report generation overflow.
        entry.input = entry.input.saturating_add(msg.tokens.input);
        entry.output = entry.output.saturating_add(msg.tokens.output);
        entry.cache_read = entry.cache_read.saturating_add(msg.tokens.cache_read);
        entry.cache_write = entry.cache_write.saturating_add(msg.tokens.cache_write);
        entry.reasoning = entry.reasoning.saturating_add(msg.tokens.reasoning);
        entry.message_count = entry.message_count.saturating_add(msg.message_count.max(0));
        entry.cost += msg.cost;
    }

    let mut entries: Vec<MonthlyUsageV2> = month_map
        .into_iter()
        .map(|(month, agg)| MonthlyUsageV2 {
            month,
            models: agg.models.into_iter().collect(),
            input: agg.input,
            output: agg.output,
            cache_read: agg.cache_read,
            cache_write: agg.cache_write,
            reasoning: agg.reasoning,
            message_count: agg.message_count,
            cost: agg.cost,
        })
        .collect();

    entries.sort_by(|a, b| a.month.cmp(&b.month));
    entries
}

pub async fn get_monthly_report_v2(options: ReportOptions) -> Result<MonthlyReportV2, String> {
    let start = Instant::now();

    let home_dir = get_home_dir_string(&options.home_dir)?;

    let clients: Vec<String> = options.clients.clone().unwrap_or_else(|| {
        let mut clients: Vec<String> = ClientId::ALL
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());
        clients
    });

    let pricing = load_pricing_for_local_parse().await;
    let all_messages = parse_all_messages_with_pricing_with_env_strategy(
        &home_dir,
        &clients,
        pricing.as_deref(),
        options.use_env_roots,
        &options.scanner_settings,
    );

    let filtered = filter_messages_for_report(all_messages, &options);
    let entries = aggregate_monthly_usage_v2_entries(filtered);

    // f64's Sum identity is -0.0, so an empty report would serialize as
    // "totalCost": -0.0; adding +0.0 normalizes the sign without changing
    // any non-zero total.
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum::<f64>() + 0.0;

    Ok(MonthlyReportV2 {
        entries,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

/// Generate the original monthly report shape.
///
/// New callers that need reasoning tokens should use [`get_monthly_report_v2`].
pub async fn get_monthly_report(options: ReportOptions) -> Result<MonthlyReport, String> {
    Ok(get_monthly_report_v2(options).await?.into_legacy())
}

#[derive(Default)]
struct HourAggregator {
    clients: HashSet<String>,
    models: HashSet<String>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    message_count: i32,
    turn_count: i32,
    cost: f64,
}

fn aggregate_hourly_usage_entries(
    messages: impl IntoIterator<Item = UnifiedMessage>,
    bucket_timezone: bucket_tz::BucketTimezone,
) -> Vec<HourlyUsage> {
    let mut hour_map: HashMap<String, HourAggregator> = HashMap::new();

    for msg in messages {
        let hour_key = if msg.timestamp > 0 {
            bucket_timezone
                .hour_key(msg.timestamp)
                .unwrap_or_else(|| format!("{} 00:00", msg.date))
        } else {
            format!("{} 00:00", msg.date)
        };

        let entry = hour_map.entry(hour_key).or_default();
        entry.clients.insert(msg.client.clone());
        entry.models.insert(model_name_for_grouping(
            &msg.client,
            &msg.provider_id,
            &msg.model_id,
        ));
        entry.input = entry.input.saturating_add(msg.tokens.input);
        entry.output = entry.output.saturating_add(msg.tokens.output);
        entry.cache_read = entry.cache_read.saturating_add(msg.tokens.cache_read);
        entry.cache_write = entry.cache_write.saturating_add(msg.tokens.cache_write);
        entry.reasoning = entry.reasoning.saturating_add(msg.tokens.reasoning);
        entry.message_count += msg.message_count.max(0);
        if msg.is_turn_start {
            entry.turn_count += 1;
        }
        entry.cost += msg.cost;
    }

    let mut entries: Vec<HourlyUsage> = hour_map
        .into_iter()
        .map(|(hour, agg)| HourlyUsage {
            hour,
            clients: {
                let mut clients: Vec<String> = agg.clients.into_iter().collect();
                clients.sort();
                clients
            },
            models: {
                let mut models: Vec<String> = agg.models.into_iter().collect();
                models.sort();
                models
            },
            input: agg.input,
            output: agg.output,
            cache_read: agg.cache_read,
            cache_write: agg.cache_write,
            message_count: agg.message_count,
            turn_count: agg.turn_count,
            reasoning: agg.reasoning,
            cost: agg.cost,
        })
        .collect();

    entries.sort_by(|a, b| a.hour.cmp(&b.hour));
    entries
}

/// Generate hourly usage report, keyed by "YYYY-MM-DD HH:00".
///
/// Derives the hour slot from `UnifiedMessage.timestamp` (Unix ms).
/// Falls back to date + "00:00" when timestamp is zero or missing.
pub async fn get_hourly_report(options: ReportOptions) -> Result<HourlyReport, String> {
    let start = Instant::now();

    let home_dir = get_home_dir_string(&options.home_dir)?;

    let clients: Vec<String> = options.clients.clone().unwrap_or_else(|| {
        let mut clients: Vec<String> = ClientId::ALL
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());
        clients
    });

    let pricing = load_pricing_for_local_parse().await;
    let all_messages = parse_all_messages_with_pricing_with_env_strategy(
        &home_dir,
        &clients,
        pricing.as_deref(),
        options.use_env_roots,
        &options.scanner_settings,
    );

    let filtered = filter_messages_for_report(all_messages, &options);

    // The hour key embeds a date, and the timestamp-less fallback builds one
    // out of `msg.date`, which the rebucket pass already moved to the pinned
    // zone. Deriving it from the host would let one report disagree with
    // itself about which day an hour belongs to.
    let bucket_timezone =
        bucket_tz::BucketTimezone::from_scanner_settings(&options.scanner_settings);
    let entries = aggregate_hourly_usage_entries(filtered, bucket_timezone);

    // f64's Sum identity is -0.0, so an empty report would serialize as
    // "totalCost": -0.0; adding +0.0 normalizes the sign without changing
    // any non-zero total.
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum::<f64>() + 0.0;

    Ok(HourlyReport {
        entries,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

#[derive(Clone, Copy)]
enum GraphPricingRequirement {
    Lenient,
    Submission,
}

async fn generate_graph_with_loaded_pricing(
    options: ReportOptions,
    pricing: Option<&pricing::PricingService>,
    pricing_requirement: GraphPricingRequirement,
) -> Result<GraphResult, String> {
    let start = Instant::now();

    let home_dir = get_home_dir_string(&options.home_dir)?;

    let clients: Vec<String> = options.clients.clone().unwrap_or_else(|| {
        let mut clients: Vec<String> = ClientId::ALL
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());
        clients
    });

    let all_messages = parse_all_messages_with_pricing_with_env_strategy(
        &home_dir,
        &clients,
        pricing,
        options.use_env_roots,
        &options.scanner_settings,
    );

    let filtered = filter_messages_for_report(all_messages, &options);

    let bucket_timezone =
        bucket_tz::BucketTimezone::from_scanner_settings(&options.scanner_settings);

    build_graph_from_messages(
        filtered,
        pricing,
        pricing_requirement,
        start,
        &bucket_timezone,
    )
}

fn build_graph_from_messages(
    filtered: Vec<UnifiedMessage>,
    pricing: Option<&pricing::PricingService>,
    pricing_requirement: GraphPricingRequirement,
    start: Instant,
    bucket_timezone: &bucket_tz::BucketTimezone,
) -> Result<GraphResult, String> {
    let (filtered, unpriced_submission_exclusions) = match pricing_requirement {
        GraphPricingRequirement::Lenient => (filtered, Vec::new()),
        GraphPricingRequirement::Submission => {
            let (submitted, exclusions) = exclude_unpriced_submission_messages(filtered, pricing);
            require_trustworthy_exclusions(pricing, &exclusions)?;
            validate_priced_messages(&submitted, pricing)?;
            (submitted, exclusions)
        }
    };

    let intervals = sessionize::sessionize(&filtered, sessionize::DEFAULT_IDLE_GAP_MS);
    let time_metrics =
        sessionize::compute_time_metrics(&intervals, sessionize::DEFAULT_IDLE_GAP_MS);

    // Keyed by the same zone the messages were rebucketed into. Active time is
    // joined onto contributions by date below, so a mismatch here silently
    // drops a day's active time rather than misplacing it.
    let daily_active_time = sessionize::compute_daily_active_time_in(&intervals, bucket_timezone);
    let contributions = aggregator::aggregate_by_date(filtered);

    let processing_time_ms = start.elapsed().as_millis() as u32;
    let mut result = aggregator::generate_graph_result(contributions, processing_time_ms);
    result.time_metrics = Some(time_metrics);
    result.unpriced_submission_exclusions = unpriced_submission_exclusions;

    for contribution in &mut result.contributions {
        if let Some(&ms) = daily_active_time.get(&contribution.date) {
            contribution.active_time_ms = Some(ms);
        }
    }

    Ok(result)
}

const ROUTING_LABEL_UNPRICED_REASON: &str =
    "generic routing label has no authoritative model-to-price mapping";
const MISSING_MODEL_PRICING_REASON: &str = "no authoritative model-to-price mapping";
const INCOMPLETE_MODEL_PRICING_REASON: &str = "pricing does not cover every populated token bucket";
const AMBIGUOUS_MODEL_PRICING_REASON: &str =
    "model price lookup is ambiguous across non-equivalent candidates";
const UNVERIFIED_MODEL_IDENTITY_REASON: &str =
    "model price match does not exactly name the requested model";
const UNVERIFIED_PROVIDER_IDENTITY_REASON: &str =
    "model price match does not establish the requested provider";

/// Routing labels name the router that served the request, never the model
/// that answered it, so they have no authoritative model-to-price mapping.
/// This defers to `lookup::is_routing_label` (lookup.rs) rather than restating
/// its `ROUTING_LABELS` list: the reason a row is excluded has to name the same
/// labels the resolver refuses at its top, and a second copy of the list would
/// drift the moment a label is added to one side. Trimming matches for the same
/// reason — the resolver trims, so ` auto ` must not read as a routing label
/// here while being refused there. The historical `gemini-default` pair is
/// provider-scoped and lives only in this reason, not in the resolver gate.
fn is_generic_routing_label(provider_id: &str, model_id: &str) -> bool {
    (provider_id.eq_ignore_ascii_case("google")
        && model_id.trim().eq_ignore_ascii_case("gemini-default"))
        || pricing::lookup::is_routing_label(model_id)
}

fn has_positive_token_usage(tokens: &TokenBreakdown) -> bool {
    tokens.input > 0
        || tokens.output > 0
        || tokens.cache_read > 0
        || tokens.cache_write > 0
        || tokens.reasoning > 0
}

fn exclude_unpriced_submission_messages(
    messages: Vec<UnifiedMessage>,
    pricing: Option<&pricing::PricingService>,
) -> (Vec<UnifiedMessage>, Vec<UnpricedSubmissionExclusion>) {
    use pricing::lookup::SubmissionSafetyGap;

    let Some(pricing) = pricing else {
        return (messages, Vec::new());
    };

    let mut submitted = Vec::with_capacity(messages.len());
    let mut exclusions: std::collections::BTreeMap<(String, String), (usize, i64, &'static str)> =
        std::collections::BTreeMap::new();

    for message in messages {
        let is_unpriced = has_positive_token_usage(&message.tokens)
            && !message.has_authoritative_cost()
            && !pricing.covers_usage_with_provider(
                &message.model_id,
                Some(&message.provider_id),
                &message.tokens,
            );

        if is_unpriced {
            // Resolution is consulted before the routing-label reason, not
            // after. `custom-pricing.json` is read first by
            // `lookup_with_source_and_provider`, and stating a rate for `auto`
            // there is the user asserting the label does name something for
            // them — the escape hatch `lookup::ROUTING_LABELS` documents.
            // Checking the label first told that user their label "has no
            // authoritative model-to-price mapping" while their own file held
            // one, hiding the fixable gap (a bucket their entry omits).
            // Nothing regresses for unpriced labels: the resolver refuses
            // routing labels outright, so with no custom entry this returns
            // None and the routing-label reason still applies.
            let resolution = pricing.resolve_for_usage_with_provider(
                &message.model_id,
                Some(&message.provider_id),
                &message.tokens,
            );
            // The gap is read from the resolution that made the row
            // unpublishable rather than restated here: a lookup with a single
            // candidate is excluded for not naming the model, and reporting it
            // as ambiguous across candidates would describe a disagreement
            // that never happened.
            let safety_gap = resolution
                .as_ref()
                .and_then(|result| result.evidence.submission_safety_gap());
            let reason = if let Some(gap) = safety_gap {
                match gap {
                    SubmissionSafetyGap::PriceDisagreement => AMBIGUOUS_MODEL_PRICING_REASON,
                    SubmissionSafetyGap::UnverifiedModelIdentity => {
                        UNVERIFIED_MODEL_IDENTITY_REASON
                    }
                    SubmissionSafetyGap::UnverifiedProviderIdentity => {
                        UNVERIFIED_PROVIDER_IDENTITY_REASON
                    }
                }
            } else if resolution.is_some() {
                INCOMPLETE_MODEL_PRICING_REASON
            } else if is_generic_routing_label(&message.provider_id, &message.model_id) {
                ROUTING_LABEL_UNPRICED_REASON
            } else {
                MISSING_MODEL_PRICING_REASON
            };
            let entry = exclusions
                .entry((message.provider_id.clone(), message.model_id.clone()))
                .or_insert((0, 0, reason));
            entry.0 = entry
                .0
                .saturating_add(message.message_count.max(0) as usize);
            entry.1 = entry.1.saturating_add(message.tokens.total());
        } else {
            submitted.push(message);
        }
    }

    let exclusions = exclusions
        .into_iter()
        .map(
            |((provider_id, model_id), (message_count, total_tokens, reason))| {
                UnpricedSubmissionExclusion {
                    provider_id,
                    model_id,
                    message_count,
                    total_tokens,
                    reason,
                }
            },
        )
        .collect();
    (submitted, exclusions)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TimeMetricsReport {
    pub metrics: sessionize::TimeMetrics,
    pub processing_time_ms: u32,
}

pub async fn get_time_metrics_report(options: ReportOptions) -> Result<TimeMetricsReport, String> {
    let start = Instant::now();

    let home_dir = get_home_dir_string(&options.home_dir)?;

    let clients: Vec<String> = options.clients.clone().unwrap_or_else(|| {
        let mut clients: Vec<String> = ClientId::ALL
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());
        clients
    });

    let all_messages = parse_all_messages_with_pricing_with_env_strategy(
        &home_dir,
        &clients,
        None,
        options.use_env_roots,
        &options.scanner_settings,
    );

    let filtered = filter_messages_for_report(all_messages, &options);

    let intervals = sessionize::sessionize(&filtered, sessionize::DEFAULT_IDLE_GAP_MS);
    let metrics = sessionize::compute_time_metrics(&intervals, sessionize::DEFAULT_IDLE_GAP_MS);

    Ok(TimeMetricsReport {
        metrics,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

pub async fn generate_graph(options: ReportOptions) -> Result<GraphResult, String> {
    let pricing = pricing::PricingService::get_or_init().await?;
    generate_graph_with_loaded_pricing(options, Some(&pricing), GraphPricingRequirement::Lenient)
        .await
}

pub async fn generate_submission_graph(options: ReportOptions) -> Result<GraphResult, String> {
    let pricing = pricing::PricingService::get_or_init().await?;
    generate_graph_with_loaded_pricing(options, Some(&pricing), GraphPricingRequirement::Submission)
        .await
}

pub async fn generate_local_graph_report(options: ReportOptions) -> Result<GraphResult, String> {
    let pricing = load_pricing_for_local_parse().await;
    generate_graph_with_loaded_pricing(
        options,
        pricing.as_deref(),
        GraphPricingRequirement::Lenient,
    )
    .await
}

const UNAVAILABLE_SUBMISSION_PRICING: &str = "pricing data is unavailable for submission";

// @keep: the two conditions are load-bearing together; either alone is wrong.
/// Refuse to act on exclusions that no pricing dataset backs.
///
/// `exclude_unpriced_submission_messages` drops what the pricing service cannot
/// cover, but a service with no dataset covers *nothing*, so "unpriced" and "we
/// have no prices" produce identical exclusions. Left alone, a cold cache with
/// no network excludes the entire batch, leaves `total_tokens == 0`, and lets
/// the CLI print "No usage data found to submit" and exit 0 — indistinguishable
/// from genuinely having no usage, and reported as success to autosubmit.
///
/// Both conditions matter:
///
/// - Only when something was excluded. A batch whose messages all carry
///   provider-reported costs never consults pricing, so a missing dataset is
///   irrelevant and must not block it.
/// - Only when no dataset loaded. A populated dataset that simply lacks a price
///   for some model is the case #1053 exists to handle; failing there would
///   break autosubmit for anyone whose usage is legitimately unpriceable, which
///   is the trap #1044 documents.
///
/// This runs after exclusion because the exclusion list is the signal. It
/// cannot move into `validate_priced_messages`, which sees only the survivors —
/// and when everything is excluded that slice is empty and validates trivially.
fn require_trustworthy_exclusions(
    pricing: Option<&pricing::PricingService>,
    exclusions: &[UnpricedSubmissionExclusion],
) -> Result<(), String> {
    if exclusions.is_empty() {
        return Ok(());
    }

    match pricing {
        Some(pricing) if pricing.has_pricing_data() => Ok(()),
        _ => Err(UNAVAILABLE_SUBMISSION_PRICING.to_string()),
    }
}

fn validate_priced_messages(
    messages: &[UnifiedMessage],
    pricing: Option<&pricing::PricingService>,
) -> Result<(), String> {
    let Some(pricing) = pricing else {
        return Err(UNAVAILABLE_SUBMISSION_PRICING.to_string());
    };

    // Counted rather than listed per message: a real submission repeats the
    // same handful of ids thousands of times, and the raw list buried the
    // actionable model names under hundreds of kilobytes of output (#1013).
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for message in messages {
        let unpriced = has_positive_token_usage(&message.tokens)
            && !message.has_authoritative_cost()
            && !pricing.covers_usage_with_provider(
                &message.model_id,
                Some(&message.provider_id),
                &message.tokens,
            );
        if !unpriced {
            continue;
        }

        let id = if message.provider_id.is_empty() {
            message.model_id.clone()
        } else {
            format!("{}/{}", message.provider_id, message.model_id)
        };
        match counts.get_mut(&id) {
            Some(count) => *count += 1,
            None => {
                counts.insert(id.clone(), 1);
                order.push(id);
            }
        }
    }

    if order.is_empty() {
        return Ok(());
    }

    let summary = order
        .into_iter()
        .map(|id| match counts.get(&id).copied().unwrap_or(1) {
            1 => id,
            count => format!("{id} (x{count})"),
        })
        .collect::<Vec<String>>()
        .join(", ");

    Err(format!(
        "pricing is unavailable for submitted token usage: {summary}"
    ))
}

fn filter_messages_for_report(
    messages: Vec<UnifiedMessage>,
    options: &ReportOptions,
) -> Vec<UnifiedMessage> {
    let mut filtered = messages;

    if let Some(year) = &options.year {
        let year_prefix = format!("{}-", year);
        filtered.retain(|m| m.date.starts_with(&year_prefix));
    }

    if let Some(since) = &options.since {
        filtered.retain(|m| m.date.as_str() >= since.as_str());
    }

    if let Some(until) = &options.until {
        filtered.retain(|m| m.date.as_str() <= until.as_str());
    }
    filtered
}

fn is_headless_path(path: &Path, headless_roots: &[PathBuf]) -> bool {
    headless_roots.iter().any(|root| path.starts_with(root))
}

fn apply_headless_agent(message: &mut UnifiedMessage, is_headless: bool) {
    if is_headless && message.agent.is_none() {
        message.agent = Some("headless".to_string());
    }
}

fn pricing_multiplier(message: &UnifiedMessage) -> f64 {
    // Zed bills hosted models at provider list price + 10%.
    // Source: https://zed.dev/docs/ai/plans-and-usage and https://zed.dev/docs/ai/models
    //
    // The multiplier is keyed on the message's `provider_id`, not on the
    // provenance of the matched LiteLLM pricing row. Today this is safe because
    // tokscale's bundled LiteLLM dataset only carries upstream-provider rows
    // (anthropic, openai, google) for the underlying models. If a future
    // LiteLLM update adds rows under provider `zed.dev` that already include
    // Zed's markup, this function would double-bill — revisit by threading
    // the matched-price provenance through `apply_pricing_if_available`.
    if message.client == "zed"
        && message
            .provider_id
            .eq_ignore_ascii_case(sessions::zed::ZED_HOSTED_PROVIDER)
    {
        1.1
    } else {
        1.0
    }
}

fn apply_pricing_if_available(
    message: &mut UnifiedMessage,
    pricing: Option<&pricing::PricingService>,
) {
    if message.has_authoritative_cost() {
        return;
    }

    let Some(pricing) = pricing else {
        return;
    };

    let calculated_cost = pricing.calculate_cost_with_provider(
        &message.model_id,
        Some(&message.provider_id),
        &message.tokens,
    ) * pricing_multiplier(message);

    if calculated_cost > 0.0 {
        message.cost = calculated_cost;
        message.mark_estimated_cost();
    }
}

/// Merge two cross-file Claude observations without letting retained
/// provenance replace live metadata. Completeness is monotonic for usage
/// fields, while model/provider/session/workspace stay sourced from the live
/// observation. Estimated cost is then derived from that authoritative metadata
/// and the merged tokens; provider-reported cost remains immune to repricing
/// through `apply_pricing_if_available`'s authority guard.
fn merge_claude_cross_file_duplicate(
    existing: &mut UnifiedMessage,
    existing_is_retained: &mut bool,
    mut candidate: UnifiedMessage,
    candidate_is_retained: bool,
    pricing: Option<&pricing::PricingService>,
) {
    if *existing_is_retained && !candidate_is_retained {
        sessions::claudecode::merge_message_completeness(&mut candidate, existing);
        *existing = candidate;
    } else {
        sessions::claudecode::merge_message_completeness(existing, &candidate);
    }
    *existing_is_retained &= candidate_is_retained;
    existing.refresh_derived_fields();
    apply_pricing_if_available(existing, pricing);
}

fn parse_hermes_sqlite_with_pricing(
    db_path: &Path,
    pricing: Option<&pricing::PricingService>,
) -> Vec<UnifiedMessage> {
    sessions::hermes::parse_hermes_sqlite(db_path)
        .into_iter()
        .map(|mut msg| {
            if msg.cost <= 0.0 {
                apply_pricing_if_available(&mut msg, pricing);
            }
            msg
        })
        .collect()
}

fn select_local_parse_pricing<F>(
    fresh: Result<Arc<pricing::PricingService>, String>,
    stale: F,
) -> Option<Arc<pricing::PricingService>>
where
    F: FnOnce() -> Option<pricing::PricingService>,
{
    fresh.ok().or_else(|| stale().map(Arc::new))
}

async fn load_pricing_for_local_parse() -> Option<Arc<pricing::PricingService>> {
    if std::env::var("TOKSCALE_PRICING_CACHE_ONLY")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
    {
        return pricing::PricingService::load_cached_any_age().map(Arc::new);
    }

    // Interactive/local views should pick up newly released model pricing as soon
    // as a fresh fetch succeeds, but still remain usable offline by falling back
    // to any cached dataset when the network path fails.
    select_local_parse_pricing(
        pricing::PricingService::get_or_init().await,
        pricing::PricingService::load_cached_any_age,
    )
}

fn resolve_local_parse_request(
    options: &LocalParseOptions,
) -> Result<(String, Vec<String>), String> {
    let home_dir = get_home_dir_string(&options.home_dir)?;
    let clients = options.clients.clone().unwrap_or_else(|| {
        let mut clients: Vec<String> = ClientId::iter()
            .filter(|c| c.parse_local())
            .map(|c| c.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());
        clients
    });
    Ok((home_dir, clients))
}

fn parse_local_unified_messages_resolved(
    options: LocalParseOptions,
    home_dir: &str,
    clients: &[String],
    pricing: Option<&pricing::PricingService>,
    cache_policy: SourceCachePolicy,
) -> Result<Vec<UnifiedMessage>, String> {
    let messages = parse_all_messages_with_pricing_with_cache_policy(
        home_dir,
        clients,
        pricing,
        options.use_env_roots,
        &options.scanner_settings,
        cache_policy,
    );
    Ok(filter_unified_messages(messages, &options))
}
pub fn parse_local_clients(options: LocalParseOptions) -> Result<ParsedMessages, String> {
    let start = Instant::now();

    let home_dir = get_home_dir_string(&options.home_dir)?;

    let clients: Vec<String> = options.clients.clone().unwrap_or_else(|| {
        let mut clients: Vec<String> = ClientId::iter()
            .filter(|c| c.parse_local())
            .map(|c| c.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());
        clients
    });
    let include_all = clients.is_empty();
    let include_synthetic = include_all || clients.iter().any(|c| c == "synthetic");
    let include_devin_cli = include_synthetic || clients.iter().any(|c| c == "devin-cli");
    let include_devin_desktop = include_synthetic || clients.iter().any(|c| c == "devin-desktop");
    // Freebuff and Codebuff share the manicode scan bucket in the scanner (the
    // two parsers partition the same file set). Each product parses and counts
    // only when it was actually requested, so a codebuff-only filter cannot
    // pick up estimated Freebuff rows and vice versa.
    let include_codebuff = include_all || clients.iter().any(|c| c == "codebuff");
    let include_freebuff = include_all || clients.iter().any(|c| c == "freebuff");

    let scan_result = scanner::scan_all_clients_with_scanner_settings(
        &home_dir,
        &clients,
        options.use_env_roots,
        &options.scanner_settings,
    );
    let headless_roots =
        scanner::headless_roots_with_env_strategy(&home_dir, options.use_env_roots);

    let mut messages: Vec<ParsedMessage> = Vec::new();

    // Parse OpenCode: prefer SQLite, collapse forked SQLite history there, then
    // suppress legacy JSON overlap by message identity.
    let mut counts = ClientCounts::new();

    let opencode_count: i32 = {
        let mut seen: HashSet<String> = HashSet::new();
        let mut count: i32 = 0;

        for db_path in &scan_result.opencode_dbs {
            let sqlite_msgs: Vec<(String, ParsedMessage)> =
                sessions::opencode::parse_opencode_sqlite(db_path)
                    .into_iter()
                    .filter_map(|msg| {
                        let key = msg.dedup_key.clone().unwrap_or_default();
                        // Dedup across multiple channel-suffixed dbs: the
                        // same session can end up in both `opencode.db` and
                        // `opencode-<channel>.db` if the user switches
                        // channels mid-session.
                        if !key.is_empty() && !seen.insert(key.clone()) {
                            return None;
                        }
                        Some((key, unified_to_parsed(&msg)))
                    })
                    .collect();
            count += sqlite_msgs.len() as i32;
            for (_key, parsed) in sqlite_msgs {
                messages.push(parsed);
            }
        }

        let json_msgs: Vec<(String, ParsedMessage)> = scan_result
            .get(ClientId::OpenCode)
            .par_iter()
            .filter_map(|path| {
                let msg = sessions::opencode::parse_opencode_file(path)?;
                let key = msg.dedup_key.clone().unwrap_or_default();
                Some((key, unified_to_parsed(&msg)))
            })
            .collect();
        let deduped: Vec<ParsedMessage> = json_msgs
            .into_iter()
            .filter(|(key, _)| key.is_empty() || seen.insert(key.clone()))
            .map(|(_, msg)| msg)
            .collect();
        count += deduped.len() as i32;
        messages.extend(deduped);

        count
    };
    counts.set(ClientId::OpenCode, opencode_count);

    let claude_home = PathBuf::from(&home_dir);
    let claude_msgs_raw: Vec<(String, ParsedMessage)> = scan_result
        .get(ClientId::Claude)
        .par_iter()
        .map_init(std::collections::HashMap::new, |parent_cache, path| {
            sessions::claudecode::parse_claude_file_with_cache_and_home(
                path,
                parent_cache,
                Some(&claude_home),
            )
            .into_iter()
            .map(|msg| {
                let dedup_key = msg.dedup_key.clone().unwrap_or_default();
                (dedup_key, unified_to_parsed(&msg))
            })
            .collect::<Vec<_>>()
        })
        .flatten()
        .collect();

    let mut seen_keys: HashSet<String> = HashSet::new();
    let claude_msgs: Vec<ParsedMessage> = claude_msgs_raw
        .into_iter()
        .filter(|(key, _)| key.is_empty() || seen_keys.insert(key.clone()))
        .map(|(_, msg)| msg)
        .collect();
    let claude_count = claude_msgs.len() as i32;
    counts.set(ClientId::Claude, claude_count);
    messages.extend(claude_msgs);

    let codex_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Codex)
        .par_iter()
        .flat_map(|path| {
            let is_headless = is_headless_path(path, &headless_roots);
            sessions::codex::parse_codex_file(path)
                .into_iter()
                .map(|mut msg| {
                    apply_headless_agent(&mut msg, is_headless);
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut codex_seen: HashSet<String> = HashSet::new();
    let codex_msgs: Vec<ParsedMessage> = codex_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut codex_seen, message))
        .map(|message| unified_to_parsed(&message))
        .collect();
    let codex_count = codex_msgs.len() as i32;
    counts.set(ClientId::Codex, codex_count);
    messages.extend(codex_msgs);

    let mut copilot_unified_msgs: Vec<_> = scan_result
        .get(ClientId::Copilot)
        .par_iter()
        .flat_map(|path| {
            sessions::copilot::parse_copilot_file(path)
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect();
    if let Some(db_path) = &scan_result.copilot_desktop_db {
        let otel_sessions: HashSet<String> = copilot_unified_msgs
            .iter()
            .map(|message| message.session_id.clone())
            .collect();
        copilot_unified_msgs.extend(
            sessions::copilot_desktop::parse_copilot_desktop_db(db_path)
                .into_iter()
                .filter(|message| !otel_sessions.contains(&message.session_id)),
        );
    }
    {
        let existing_dedup_keys: HashSet<String> = copilot_unified_msgs
            .iter()
            .filter_map(|m| m.dedup_key.clone())
            .collect();
        let existing_copilot_session_timestamps: HashSet<(String, i64)> = copilot_unified_msgs
            .iter()
            .map(|m| (m.session_id.clone(), m.timestamp))
            .collect();
        copilot_unified_msgs.extend(
            sessions::copilot_vscode::parse_copilot_vscode_sessions(
                &scan_result.copilot_vscode_sessions,
            )
            .into_iter()
            .filter(|m| {
                let key_unique = m
                    .dedup_key
                    .as_deref()
                    .map(|k| !existing_dedup_keys.contains(k))
                    .unwrap_or(true);
                let session_ts_unique = !existing_copilot_session_timestamps
                    .contains(&(m.session_id.clone(), m.timestamp));
                key_unique && session_ts_unique
            }),
        );
    }
    let copilot_msgs: Vec<ParsedMessage> =
        copilot_unified_msgs.iter().map(unified_to_parsed).collect();
    let copilot_count = copilot_msgs.len() as i32;
    counts.set(ClientId::Copilot, copilot_count);
    messages.extend(copilot_msgs);

    let gemini_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Gemini)
        .par_iter()
        .flat_map(|path| {
            sessions::gemini::parse_gemini_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let gemini_count = gemini_msgs.len() as i32;
    counts.set(ClientId::Gemini, gemini_count);
    messages.extend(gemini_msgs);

    let amp_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Amp)
        .par_iter()
        .flat_map(|path| {
            sessions::amp::parse_amp_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let amp_count = amp_msgs.len() as i32;
    counts.set(ClientId::Amp, amp_count);
    messages.extend(amp_msgs);

    let codebuff_msgs: Vec<ParsedMessage> = if include_codebuff {
        scan_result
            .get(ClientId::Codebuff)
            .par_iter()
            .flat_map(|path| {
                sessions::codebuff::parse_codebuff_file(path)
                    .into_iter()
                    .map(|msg| unified_to_parsed(&msg))
                    .collect::<Vec<_>>()
            })
            .collect()
    } else {
        Vec::new()
    };
    let codebuff_count = codebuff_msgs.len() as i32;
    counts.set(ClientId::Codebuff, codebuff_count);
    messages.extend(codebuff_msgs);

    // Freebuff shares the manicode scan; the estimated parser runs over the
    // same file set (see the main dispatch block above).
    let freebuff_msgs: Vec<ParsedMessage> = if include_freebuff {
        scan_result
            .get(ClientId::Codebuff)
            .par_iter()
            .flat_map(|path| {
                sessions::freebuff::parse_freebuff_file(path)
                    .into_iter()
                    .map(|msg| unified_to_parsed(&msg))
                    .collect::<Vec<_>>()
            })
            .collect()
    } else {
        Vec::new()
    };
    let freebuff_count = freebuff_msgs.len() as i32;
    counts.set(ClientId::Freebuff, freebuff_count);
    messages.extend(freebuff_msgs);

    let droid_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Droid)
        .par_iter()
        .flat_map(|path| {
            sessions::droid::parse_droid_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let droid_count = droid_msgs.len() as i32;
    counts.set(ClientId::Droid, droid_count);
    messages.extend(droid_msgs);

    let openclaw_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::OpenClaw)
        .par_iter()
        .flat_map(|path| {
            sessions::openclaw::parse_openclaw_transcript(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let openclaw_count = openclaw_msgs.len() as i32;
    counts.set(ClientId::OpenClaw, openclaw_count);
    messages.extend(openclaw_msgs);

    let pi_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Pi)
        .par_iter()
        .flat_map(|path| {
            sessions::pi::parse_pi_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let pi_count = pi_msgs.len() as i32;
    counts.set(ClientId::Pi, pi_count);
    messages.extend(pi_msgs);

    let prime_agent_files: Vec<(
        Vec<UnifiedMessage>,
        sessions::prime_agent::PrimeFileAccounting,
    )> = scan_result
        .get(ClientId::PrimeAgent)
        .par_iter()
        .map(|path| sessions::prime_agent::parse_prime_agent_file_with_accounting(path))
        .collect();
    let mut prime_agent_msgs_raw = Vec::new();
    let mut prime_agent_accounting = Vec::new();
    for (file_messages, file_accounting) in prime_agent_files {
        prime_agent_msgs_raw.extend(file_messages);
        prime_agent_accounting.push(file_accounting);
    }
    let prime_agent_msgs: Vec<ParsedMessage> =
        sessions::prime_agent::reconcile_prime_agent_messages(
            prime_agent_msgs_raw,
            &prime_agent_accounting,
        )
        .into_iter()
        .map(|message| unified_to_parsed(&message))
        .collect();
    let prime_agent_count = prime_agent_msgs.len() as i32;
    counts.set(ClientId::PrimeAgent, prime_agent_count);
    messages.extend(prime_agent_msgs);

    let kimchi_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Kimchi)
        .par_iter()
        .flat_map(|path| sessions::kimchi::parse_kimchi_file(path))
        .collect();
    let mut kimchi_seen: HashSet<String> = HashSet::new();
    let kimchi_msgs: Vec<ParsedMessage> = kimchi_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut kimchi_seen, message))
        .map(|message| unified_to_parsed(&message))
        .collect();
    let kimchi_count = kimchi_msgs.len() as i32;
    counts.set(ClientId::Kimchi, kimchi_count);
    messages.extend(kimchi_msgs);

    let reasonix_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Reasonix)
        .par_iter()
        .flat_map(|path| {
            sessions::reasonix::parse_reasonix_file(path)
                .into_iter()
                .map(|message| unified_to_parsed(&message))
                .collect::<Vec<_>>()
        })
        .collect();
    let reasonix_count = reasonix_msgs.iter().fold(0_i32, |count, message| {
        count.saturating_add(message.message_count)
    });
    counts.set(ClientId::Reasonix, reasonix_count);
    messages.extend(reasonix_msgs);

    let senpi_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Senpi)
        .par_iter()
        .flat_map(|path| {
            sessions::senpi::parse_senpi_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let senpi_count = senpi_msgs.len() as i32;
    counts.set(ClientId::Senpi, senpi_count);
    messages.extend(senpi_msgs);

    let augment_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Augment)
        .par_iter()
        .flat_map(|path| sessions::augment::parse_augment_file(path))
        .collect();
    let mut augment_seen: HashSet<String> = HashSet::new();
    let augment_msgs: Vec<ParsedMessage> = augment_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut augment_seen, message))
        .map(|msg| unified_to_parsed(&msg))
        .collect();
    let augment_count = augment_msgs.len() as i32;
    counts.set(ClientId::Augment, augment_count);
    messages.extend(augment_msgs);

    let commandcode_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::CommandCode)
        .par_iter()
        .flat_map(|path| {
            sessions::commandcode::parse_commandcode_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let commandcode_count = commandcode_msgs.len() as i32;
    counts.set(ClientId::CommandCode, commandcode_count);
    messages.extend(commandcode_msgs);

    // gjc (gajae-code) JSONL sessions. This non-cached path produces
    // ParsedMessage (no cost field) and has no pricing service in scope, so
    // the A1 cost guard is a no-op here — cost correctness is enforced on the
    // cached pricing path (see the gjc_outcomes block). What matters here is
    // message-level dedup (codebuff-style key via should_keep_deduped_message)
    // to collapse depth-1/depth-2 replays, mirroring the codex cluster.
    let gjc_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Gjc)
        .par_iter()
        .flat_map(|path| sessions::gjc::parse_gjc_file(path))
        .collect();
    let mut gjc_seen: HashSet<String> = HashSet::new();
    let gjc_msgs: Vec<ParsedMessage> = gjc_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut gjc_seen, message))
        .map(|message| unified_to_parsed(&message))
        .collect();
    let gjc_count = gjc_msgs.len() as i32;
    counts.set(ClientId::Gjc, gjc_count);
    messages.extend(gjc_msgs);

    // ParsedMessage has no pricing service in scope, but Junie parser already
    // preserves the embedded session costs for callers that need UnifiedMessage.
    // Dedup still matters here because Junie can replay metadata events.
    let junie_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Junie)
        .par_iter()
        .flat_map(|path| sessions::junie::parse_junie_file(path))
        .collect();
    let mut junie_seen: HashSet<String> = HashSet::new();
    let junie_msgs: Vec<ParsedMessage> = junie_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut junie_seen, message))
        .map(|message| unified_to_parsed(&message))
        .collect();
    let junie_count = summed_parsed_message_count(&junie_msgs);
    counts.set(ClientId::Junie, junie_count);
    messages.extend(junie_msgs);

    // ZCode v2 CLI SQLite usage plus legacy JSONL session transcripts.
    let mut zcode_msgs: Vec<ParsedMessage> = scan_result
        .zcode_db
        .as_ref()
        .map(|db_path| {
            sessions::zcode::parse_zcode_sqlite(db_path)
                .into_iter()
                .map(|message| unified_to_parsed(&message))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    zcode_msgs.extend(
        scan_result
            .get(ClientId::Zcode)
            .par_iter()
            .flat_map(|path| sessions::zcode::parse_zcode_file(path))
            .map(|message| unified_to_parsed(&message))
            .collect::<Vec<_>>(),
    );
    let zcode_count = summed_parsed_message_count(&zcode_msgs);
    counts.set(ClientId::Zcode, zcode_count);
    messages.extend(zcode_msgs);

    // Cherry Studio agent-session transcripts (Claude Code format).
    let cherrystudio_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::CherryStudio)
        .par_iter()
        .flat_map(|path| {
            sessions::cherrystudio::parse_cherrystudio_file(path)
                .into_iter()
                .map(|message| unified_to_parsed(&message))
                .collect::<Vec<_>>()
        })
        .collect();
    let cherrystudio_count = summed_parsed_message_count(&cherrystudio_msgs);
    counts.set(ClientId::CherryStudio, cherrystudio_count);
    messages.extend(cherrystudio_msgs);

    // DeepSeek Harness zstd JSONL transcripts. A fork's seeded prefix repeats
    // the parent's rows verbatim in a second file, so dedup across the lane.
    let dsh_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Dsh)
        .par_iter()
        .flat_map(|path| sessions::dsh::parse_dsh_file(path))
        .collect();
    let mut dsh_seen: HashSet<String> = HashSet::new();
    let dsh_msgs: Vec<ParsedMessage> = dsh_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut dsh_seen, message))
        .map(|message| unified_to_parsed(&message))
        .collect();
    let dsh_count = summed_parsed_message_count(&dsh_msgs);
    counts.set(ClientId::Dsh, dsh_count);
    messages.extend(dsh_msgs);

    let mcode_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Mcode)
        .par_iter()
        .flat_map(|path| sessions::mcode::parse_mcode_file(path))
        .map(|message| unified_to_parsed(&message))
        .collect();
    let mcode_count = summed_parsed_message_count(&mcode_msgs);
    counts.set(ClientId::Mcode, mcode_count);
    messages.extend(mcode_msgs);

    let opencodereview_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::OpenCodeReview)
        .par_iter()
        .flat_map(|path| {
            sessions::opencodereview::parse_opencodereview_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let opencodereview_count = summed_parsed_message_count(&opencodereview_msgs);
    counts.set(ClientId::OpenCodeReview, opencodereview_count);
    messages.extend(opencodereview_msgs);

    // Parse Kimi wire.jsonl files in parallel
    let kimi_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Kimi)
        .par_iter()
        .flat_map(|path| {
            let msgs = if sessions::kimi::is_kimi_code_path(path) {
                sessions::kimi::parse_kimi_code_file(path)
            } else {
                sessions::kimi::parse_kimi_file(path)
            };
            msgs.into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let kimi_count = kimi_msgs.len() as i32;
    counts.set(ClientId::Kimi, kimi_count);
    messages.extend(kimi_msgs);

    // Parse Qwen JSONL files in parallel
    let qwen_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Qwen)
        .par_iter()
        .flat_map(|path| {
            sessions::qwen::parse_qwen_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let qwen_count = qwen_msgs.len() as i32;
    counts.set(ClientId::Qwen, qwen_count);
    messages.extend(qwen_msgs);

    let roocode_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::RooCode)
        .par_iter()
        .flat_map(|path| {
            sessions::roocode::parse_roocode_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let roocode_count = roocode_msgs.len() as i32;
    counts.set(ClientId::RooCode, roocode_count);
    messages.extend(roocode_msgs);

    let kilocode_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::KiloCode)
        .par_iter()
        .flat_map(|path| {
            sessions::kilocode::parse_kilocode_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let kilocode_count = summed_parsed_message_count(&kilocode_msgs);
    counts.set(ClientId::KiloCode, kilocode_count);
    messages.extend(kilocode_msgs);

    let cline_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Cline)
        .par_iter()
        .flat_map(|path| sessions::cline::parse_cline_file(path))
        .collect();
    let mut cline_seen: HashSet<String> = HashSet::new();
    let cline_msgs: Vec<ParsedMessage> = cline_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut cline_seen, message))
        .map(|message| unified_to_parsed(&message))
        .collect();
    let cline_count = summed_parsed_message_count(&cline_msgs);
    counts.set(ClientId::Cline, cline_count);
    messages.extend(cline_msgs);

    let mux_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Mux)
        .par_iter()
        .flat_map(|path| {
            sessions::mux::parse_mux_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let mux_count = summed_parsed_message_count(&mux_msgs);
    counts.set(ClientId::Mux, mux_count);
    messages.extend(mux_msgs);

    // Kilo CLI: SQLite database
    let _kilo_count: i32 = if let Some(db_path) = &scan_result.kilo_db {
        let kilo_msgs: Vec<ParsedMessage> = sessions::kilo::parse_kilo_sqlite(db_path)
            .into_iter()
            .map(|msg| unified_to_parsed(&msg))
            .collect();
        let count = summed_parsed_message_count(&kilo_msgs);
        counts.set(ClientId::Kilo, count);
        messages.extend(kilo_msgs);
        count
    } else {
        0
    };

    let hermes_db_paths = scan_result.hermes_db_paths();
    if !hermes_db_paths.is_empty() {
        let mut hermes_seen: HashSet<String> = HashSet::new();
        let hermes_msgs: Vec<ParsedMessage> = hermes_db_paths
            .iter()
            .flat_map(|db_path| sessions::hermes::parse_hermes_sqlite(db_path))
            .filter(|msg| should_keep_deduped_message(&mut hermes_seen, msg))
            .map(|msg| unified_to_parsed(&msg))
            .collect();
        let count = summed_parsed_message_count(&hermes_msgs);
        counts.set(ClientId::Hermes, count);
        messages.extend(hermes_msgs);
    }

    if let Some(db_path) = &scan_result.goose_db {
        let goose_msgs: Vec<ParsedMessage> = sessions::goose::parse_goose_sqlite(db_path)
            .into_iter()
            .map(|msg| unified_to_parsed(&msg))
            .collect();
        let count = summed_parsed_message_count(&goose_msgs);
        counts.set(ClientId::Goose, count);
        messages.extend(goose_msgs);
    }

    let zed_db_paths = scan_result.zed_db_paths();
    if !zed_db_paths.is_empty() {
        let zed_msgs: Vec<ParsedMessage> = zed_db_paths
            .iter()
            .flat_map(|db_path| sessions::zed::parse_zed_sqlite(db_path))
            .map(|msg| unified_to_parsed(&msg))
            .collect();
        let count = summed_parsed_message_count(&zed_msgs);
        counts.set(ClientId::Zed, count);
        messages.extend(zed_msgs);
    }

    let kiro_unified: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Kiro)
        .par_iter()
        .flat_map(|path| sessions::kiro::parse_kiro_file(path))
        .collect();
    let kiro_msgs: Vec<ParsedMessage> =
        sessions::kiro::suppress_snapshots_covered_by_executions(kiro_unified)
            .iter()
            .map(unified_to_parsed)
            .collect();
    let kiro_count = summed_parsed_message_count(&kiro_msgs);
    counts.set(ClientId::Kiro, kiro_count);
    messages.extend(kiro_msgs);

    if let Some(db_path) = &scan_result.kiro_db {
        let kiro_db_msgs: Vec<ParsedMessage> = sessions::kiro::parse_kiro_sqlite(db_path)
            .into_iter()
            .map(|msg| unified_to_parsed(&msg))
            .collect();
        let kiro_db_count = summed_parsed_message_count(&kiro_db_msgs);
        counts.add(ClientId::Kiro, kiro_db_count);
        messages.extend(kiro_db_msgs);
    }

    // See the crush block in `parse_all_messages_with_pricing_with_env_strategy`.
    let crush_bucket_timezone =
        bucket_tz::BucketTimezone::from_scanner_settings(&options.scanner_settings);
    let crush_msgs: Vec<ParsedMessage> = scan_result
        .crush_dbs
        .par_iter()
        .flat_map(|source| {
            sessions::crush::parse_crush_sqlite_in(&source.db_path, &crush_bucket_timezone)
                .into_iter()
                .map(|mut msg| {
                    msg.set_workspace(source.workspace_key.clone(), source.workspace_label.clone());
                    unified_to_parsed(&msg)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let crush_count = summed_parsed_message_count(&crush_msgs);
    counts.set(ClientId::Crush, crush_count);
    messages.extend(crush_msgs);

    let antigravity_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Antigravity)
        .par_iter()
        .flat_map(|path| {
            sessions::antigravity::parse_antigravity_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let antigravity_count = antigravity_msgs.len() as i32;
    counts.set(ClientId::Antigravity, antigravity_count);
    messages.extend(antigravity_msgs);

    let antigravity_cli_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::AntigravityCli)
        .par_iter()
        .flat_map(|path| {
            sessions::antigravity_cli::parse_antigravity_cli_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let antigravity_cli_count = antigravity_cli_msgs.len() as i32;
    counts.set(ClientId::AntigravityCli, antigravity_cli_count);
    messages.extend(antigravity_cli_msgs);

    let trae_msgs: Vec<ParsedMessage> = {
        let unique_trae_messages = dedupe_latest_trae_messages(
            scan_result
                .get(ClientId::Trae)
                .par_iter()
                .flat_map(|path| sessions::trae::parse_trae_file("trae", path))
                .collect(),
        );
        unique_trae_messages
            .into_iter()
            .map(|msg| unified_to_parsed(&msg))
            .collect()
    };
    let trae_count = trae_msgs.len() as i32;
    counts.set(ClientId::Trae, trae_count);
    messages.extend(trae_msgs);

    let warp_msgs: Vec<ParsedMessage> = scan_result
        .get(ClientId::Warp)
        .par_iter()
        .flat_map(|path| {
            sessions::warp::parse_warp_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let warp_count = summed_parsed_message_count(&warp_msgs);
    counts.set(ClientId::Warp, warp_count);
    messages.extend(warp_msgs);

    // Devin CLI SQLite usage plus Desktop NDJSON event streams. The CLI
    // database is authoritative only when the CLI client itself is selected;
    // Desktop-only reports still use the database for title/model metadata but
    // must not leak CLI usage into their result.
    let mut devin_cli_seen = HashSet::new();
    let devin_cli_messages: Vec<UnifiedMessage> = if include_devin_cli {
        scan_result
            .devin_dbs
            .iter()
            .flat_map(|db_path| sessions::devin::parse_devin_cli_sqlite(db_path))
            .filter(|message| should_keep_deduped_message(&mut devin_cli_seen, message))
            .collect()
    } else {
        Vec::new()
    };
    let cli_session_ids: HashSet<String> = devin_cli_messages
        .iter()
        .map(|message| message.session_id.clone())
        .collect();
    let devin_desktop_messages_raw: Vec<UnifiedMessage> = if include_devin_desktop {
        let devin_desktop_lookup =
            sessions::devin::load_devin_desktop_session_lookup(&scan_result.devin_dbs);
        scan_result
            .get(ClientId::DevinDesktop)
            .par_iter()
            .flat_map(|path| {
                sessions::devin::parse_devin_desktop_ndjson_with_lookup(path, &devin_desktop_lookup)
            })
            .collect()
    } else {
        Vec::new()
    };
    // Count before dedup so the `clients` command reflects how many Desktop
    // sessions were actually found, even when they overlap with the CLI DB.
    let devin_desktop_raw_count: i32 = devin_desktop_messages_raw
        .iter()
        .map(|msg| msg.message_count.max(0))
        .sum();
    let devin_desktop_messages: Vec<UnifiedMessage> = devin_desktop_messages_raw
        .into_iter()
        .filter(|message| !cli_session_ids.contains(&message.session_id))
        .collect();

    let devin_cli_parsed: Vec<ParsedMessage> = devin_cli_messages
        .into_iter()
        .map(|msg| unified_to_parsed(&msg))
        .collect();
    let devin_desktop_parsed: Vec<ParsedMessage> = devin_desktop_messages
        .into_iter()
        .map(|msg| unified_to_parsed(&msg))
        .collect();
    let devin_cli_count = summed_parsed_message_count(&devin_cli_parsed);
    // Use the pre-dedup count for the `clients` command display so users see
    // all discovered Desktop sessions. The dedup-filtered messages are still
    // what gets added to the combined `messages` vector.
    let devin_desktop_count = devin_desktop_raw_count;
    counts.set(ClientId::DevinCli, devin_cli_count);
    counts.set(ClientId::DevinDesktop, devin_desktop_count);
    messages.extend(devin_cli_parsed);
    messages.extend(devin_desktop_parsed);

    let codebuddy_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::CodeBuddy)
        .par_iter()
        .flat_map(|path| sessions::codebuddy::parse_codebuddy_file(path))
        .collect();
    let mut codebuddy_seen: HashSet<String> = HashSet::new();
    let codebuddy_msgs: Vec<ParsedMessage> = codebuddy_msgs_raw
        .into_iter()
        .filter(|message| {
            message
                .dedup_key
                .as_ref()
                .is_none_or(|key| codebuddy_seen.insert(key.clone()))
        })
        .map(|msg| unified_to_parsed(&msg))
        .collect();
    let codebuddy_count = summed_parsed_message_count(&codebuddy_msgs);
    counts.set(ClientId::CodeBuddy, codebuddy_count);
    messages.extend(codebuddy_msgs);
    let (workbuddy_detailed_paths, workbuddy_fallback_paths) =
        partition_workbuddy_paths(scan_result.get(ClientId::WorkBuddy));
    let workbuddy_detailed_messages: Vec<UnifiedMessage> = workbuddy_detailed_paths
        .par_iter()
        .flat_map(|path| sessions::workbuddy::parse_workbuddy_file(path))
        .collect();
    let workbuddy_fallback_messages: Vec<UnifiedMessage> = workbuddy_fallback_paths
        .par_iter()
        .flat_map(|path| sessions::workbuddy::parse_workbuddy_file(path))
        .collect();
    let workbuddy_msgs: Vec<ParsedMessage> =
        merge_workbuddy_messages(workbuddy_detailed_messages, workbuddy_fallback_messages)
            .into_iter()
            .map(|msg| unified_to_parsed(&msg))
            .collect();
    let workbuddy_count = summed_parsed_message_count(&workbuddy_msgs);
    counts.set(ClientId::WorkBuddy, workbuddy_count);
    messages.extend(workbuddy_msgs);

    let grok_messages: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Grok)
        .par_iter()
        .flat_map(|path| sessions::grok::parse_grok_file(path))
        .collect();
    let grok_msgs: Vec<ParsedMessage> = sessions::grok::prefer_unified_log_messages(grok_messages)
        .into_iter()
        .map(|msg| unified_to_parsed(&msg))
        .collect();
    let grok_count = summed_parsed_message_count(&grok_msgs);
    counts.set(ClientId::Grok, grok_count);
    messages.extend(grok_msgs);

    let jcode_msgs_raw: Vec<UnifiedMessage> = scan_result
        .get(ClientId::Jcode)
        .par_iter()
        .flat_map(|path| sessions::jcode::parse_jcode_file(path))
        .collect();
    let mut jcode_seen: HashSet<String> = HashSet::new();
    let jcode_msgs: Vec<ParsedMessage> = jcode_msgs_raw
        .into_iter()
        .filter(|message| should_keep_deduped_message(&mut jcode_seen, message))
        .map(|msg| unified_to_parsed(&msg))
        .collect();
    let jcode_count = summed_parsed_message_count(&jcode_msgs);
    counts.set(ClientId::Jcode, jcode_count);
    messages.extend(jcode_msgs);

    if include_synthetic {
        if let Some(db_path) = &scan_result.synthetic_db {
            let synthetic_msgs: Vec<ParsedMessage> =
                sessions::synthetic::parse_octofriend_sqlite(db_path)
                    .into_iter()
                    .map(|msg| unified_to_parsed(&msg))
                    .collect();
            messages.extend(synthetic_msgs);
        }
    }

    // Filter BEFORE normalization (see parse_all_messages_with_pricing).
    if !include_all {
        let requested: HashSet<&str> = clients.iter().map(String::as_str).collect();
        messages.retain(|msg| {
            retain_for_requested_clients(&msg.client, &msg.model_id, &msg.provider_id, &requested)
        });
    }

    if include_synthetic {
        for msg in &mut messages {
            sessions::synthetic::normalize_synthetic_gateway_fields(
                &mut msg.model_id,
                &mut msg.provider_id,
            );
        }
    }

    // Before the date filter, not after: `--since`/`--until` compare against
    // `date`, so filtering first would select rows by the machine's live zone
    // and then relabel them with the pinned one.
    //
    // This path builds `ParsedMessage` straight from the parsers instead of
    // going through `parse_all_messages_with_pricing_with_env_strategy`, so it
    // needs its own rebucket — `tokscale report` and `tokscale wrapped` read
    // day keys from here.
    rebucket_parsed_days(&mut messages, &options.scanner_settings);

    let filtered = filter_parsed_messages(messages, &options);

    Ok(ParsedMessages {
        messages: filtered,
        counts,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

/// [`rebucket_days`] for the `ParsedMessage` lane. Same contract: no-op unless
/// a zone is pinned.
fn rebucket_parsed_days(
    messages: &mut [ParsedMessage],
    scanner_settings: &scanner::ScannerSettings,
) {
    let timezone = bucket_tz::BucketTimezone::from_scanner_settings(scanner_settings);
    if !timezone.is_pinned() {
        return;
    }

    for message in messages.iter_mut() {
        // See `UnifiedMessage::rebucket_date` for why a non-positive timestamp
        // and an empty key are both kept out.
        if message.timestamp <= 0 {
            continue;
        }
        let key = timezone.day_key(message.timestamp);
        if !key.is_empty() {
            message.date = key;
        }
    }
}

#[doc(hidden)]
pub async fn parse_local_unified_messages_with_pricing(
    options: LocalParseOptions,
    pricing: Option<&pricing::PricingService>,
) -> Result<Vec<UnifiedMessage>, String> {
    let (home_dir, clients) = resolve_local_parse_request(&options)?;
    parse_local_unified_messages_resolved(
        options,
        &home_dir,
        &clients,
        pricing,
        SourceCachePolicy::Persistent,
    )
}

/// Parse local messages without reading or writing the persistent source cache.
///
/// A fresh in-memory cache is still shared by every source parsed during this
/// call, preserving normal within-call reuse and deduplication. This entry point
/// is intended for isolated callers such as integration tests.
#[doc(hidden)]
pub async fn parse_local_unified_messages_with_pricing_uncached(
    options: LocalParseOptions,
    pricing: Option<&pricing::PricingService>,
) -> Result<Vec<UnifiedMessage>, String> {
    let (home_dir, clients) = resolve_local_parse_request(&options)?;
    parse_local_unified_messages_resolved(
        options,
        &home_dir,
        &clients,
        pricing,
        SourceCachePolicy::InMemory,
    )
}

pub async fn parse_local_unified_messages(
    options: LocalParseOptions,
) -> Result<Vec<UnifiedMessage>, String> {
    let (home_dir, clients) = resolve_local_parse_request(&options)?;
    let pricing = load_pricing_for_local_parse().await;
    parse_local_unified_messages_resolved(
        options,
        &home_dir,
        &clients,
        pricing.as_deref(),
        SourceCachePolicy::Persistent,
    )
}

fn unified_to_parsed(msg: &UnifiedMessage) -> ParsedMessage {
    ParsedMessage {
        client: msg.client.clone(),
        model_id: msg.model_id.clone(),
        provider_id: msg.provider_id.clone(),
        session_id: msg.session_id.clone(),
        workspace_key: msg.workspace_key.clone(),
        workspace_label: msg.workspace_label.clone(),
        timestamp: msg.timestamp,
        date: msg.date.clone(),
        input: msg.tokens.input,
        output: msg.tokens.output,
        cache_read: msg.tokens.cache_read,
        cache_write: msg.tokens.cache_write,
        reasoning: msg.tokens.reasoning,
        duration_ms: msg.duration_ms,
        message_count: msg.message_count,
        agent: msg.agent.clone(),
    }
}

fn should_keep_deduped_message(seen_keys: &mut HashSet<String>, message: &UnifiedMessage) -> bool {
    message
        .dedup_key
        .as_ref()
        .is_none_or(|key| seen_keys.insert(key.clone()))
}

fn summed_parsed_message_count(messages: &[ParsedMessage]) -> i32 {
    messages
        .iter()
        .map(|msg| msg.message_count.max(0))
        .sum::<i32>()
}

fn filter_parsed_messages(
    messages: Vec<ParsedMessage>,
    options: &LocalParseOptions,
) -> Vec<ParsedMessage> {
    let mut filtered = messages;

    if let Some(year) = &options.year {
        let year_prefix = format!("{}-", year);
        filtered.retain(|m| m.date.starts_with(&year_prefix));
    }

    if let Some(since) = &options.since {
        filtered.retain(|m| m.date.as_str() >= since.as_str());
    }

    if let Some(until) = &options.until {
        filtered.retain(|m| m.date.as_str() <= until.as_str());
    }
    filtered
}

pub fn parsed_to_unified(msg: &ParsedMessage, cost: f64) -> UnifiedMessage {
    UnifiedMessage {
        client: msg.client.clone(),
        model_id: msg.model_id.clone(),
        provider_id: msg.provider_id.clone(),
        session_id: msg.session_id.clone(),
        workspace_key: msg.workspace_key.clone(),
        workspace_label: msg.workspace_label.clone(),
        timestamp: msg.timestamp,
        date: msg.date.clone(),
        tokens: TokenBreakdown {
            input: msg.input,
            output: msg.output,
            cache_read: msg.cache_read,
            cache_write: msg.cache_write,
            reasoning: msg.reasoning,
        },
        cost,
        cost_source: CostSource::Unknown,
        duration_ms: msg.duration_ms,
        message_count: msg.message_count,
        agent: msg.agent.clone(),
        dedup_key: None,
        session_title: None,
        is_turn_start: false,
        model_attribution_conflicted: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        aggregate_hourly_usage_entries, aggregate_model_usage_entries,
        aggregate_monthly_usage_v2_entries, apply_pricing_if_available, build_graph_from_messages,
        dedupe_latest_trae_messages, filter_messages_for_report,
        generate_graph_with_loaded_pricing, get_home_dir_string, is_generic_routing_label,
        merge_claude_cross_file_duplicate, message_cache, normalize_model_for_grouping,
        parse_all_messages_with_pricing_with_cache_policy,
        parse_all_messages_with_pricing_with_env_strategy, parse_local_clients, parsed_to_unified,
        paths, pricing, retain_for_requested_clients, scanner, select_local_parse_pricing,
        sessions, unified_to_parsed, validate_priced_messages, ClientId, GraphPricingRequirement,
        GroupBy, LocalParseOptions, MonthlyReportV2, MonthlyUsage, MonthlyUsageV2, ReportOptions,
        SourceCachePolicy, TokenBreakdown, UnifiedMessage, UnpricedSubmissionExclusion,
        AMBIGUOUS_MODEL_PRICING_REASON, INCOMPLETE_MODEL_PRICING_REASON,
        MISSING_MODEL_PRICING_REASON, ROUTING_LABEL_UNPRICED_REASON, UNKNOWN_WORKSPACE_LABEL,
        UNVERIFIED_MODEL_IDENTITY_REASON, UNVERIFIED_PROVIDER_IDENTITY_REASON,
    };
    // Kept as its own statement rather than folded into the list above: that list
    // is edited by nearly every PR that touches this file, and sharing it made
    // this branch conflict on every single upstream merge.
    use super::{aggregate_model_usage_entries_with_rollup, WorktreeRollup};
    use serial_test::serial;
    use std::collections::{HashMap, HashSet};
    use std::io::Write;

    #[test]
    fn token_breakdown_add_assign_includes_every_field() {
        let mut total = TokenBreakdown {
            input: 1,
            output: 2,
            cache_read: 3,
            cache_write: 4,
            reasoning: 5,
        };
        total += &TokenBreakdown {
            input: 10,
            output: 20,
            cache_read: 30,
            cache_write: 40,
            reasoning: 50,
        };

        assert_eq!(
            total,
            TokenBreakdown {
                input: 11,
                output: 22,
                cache_read: 33,
                cache_write: 44,
                reasoning: 55,
            }
        );
    }

    #[test]
    fn token_breakdown_add_assign_saturates_each_field() {
        let mut total = TokenBreakdown {
            input: i64::MAX,
            output: i64::MIN,
            cache_read: i64::MAX - 1,
            cache_write: i64::MIN + 1,
            reasoning: 100,
        };
        total += &TokenBreakdown {
            input: 1,
            output: -1,
            cache_read: 10,
            cache_write: -10,
            reasoning: 23,
        };

        assert_eq!(total.input, i64::MAX);
        assert_eq!(total.output, i64::MIN);
        assert_eq!(total.cache_read, i64::MAX);
        assert_eq!(total.cache_write, i64::MIN);
        assert_eq!(total.reasoning, 123);
    }

    #[test]
    fn legacy_monthly_usage_struct_literal_remains_source_compatible() {
        let usage = MonthlyUsage {
            month: "2026-01".to_string(),
            models: vec!["model".to_string()],
            input: 1,
            output: 2,
            cache_read: 3,
            cache_write: 4,
            message_count: 5,
            cost: 0.5,
        };

        let serialized = serde_json::to_value(usage).unwrap();
        assert!(serialized.get("reasoning").is_none());
    }

    #[test]
    fn monthly_usage_v2_serializes_reasoning_additively() {
        let usage = MonthlyUsageV2 {
            month: "2026-01".to_string(),
            models: vec!["model".to_string()],
            input: 1,
            output: 2,
            cache_read: 3,
            cache_write: 4,
            reasoning: 6,
            message_count: 5,
            cost: 0.5,
        };

        let serialized = serde_json::to_value(&usage).unwrap();
        assert_eq!(serialized["reasoning"], 6);

        let legacy = usage.into_legacy();
        assert_eq!(legacy.month, "2026-01");
        assert_eq!(legacy.models, ["model"]);
        assert_eq!(legacy.input, 1);
        assert_eq!(legacy.output, 2);
        assert_eq!(legacy.cache_read, 3);
        assert_eq!(legacy.cache_write, 4);
        assert_eq!(legacy.message_count, 5);
        assert_eq!(legacy.cost, 0.5);
        assert!(serde_json::to_value(legacy)
            .unwrap()
            .get("reasoning")
            .is_none());
    }

    #[test]
    fn monthly_report_v2_legacy_conversion_preserves_report_metadata() {
        let report = MonthlyReportV2 {
            entries: vec![MonthlyUsageV2 {
                month: "2026-02".to_string(),
                models: vec![],
                input: 1,
                output: 2,
                cache_read: 3,
                cache_write: 4,
                reasoning: 5,
                message_count: 6,
                cost: 0.75,
            }],
            total_cost: 0.75,
            processing_time_ms: 42,
        };

        let legacy = report.into_legacy();
        assert_eq!(legacy.entries.len(), 1);
        assert_eq!(legacy.entries[0].month, "2026-02");
        assert_eq!(legacy.entries[0].input, 1);
        assert_eq!(legacy.entries[0].output, 2);
        assert_eq!(legacy.entries[0].cache_read, 3);
        assert_eq!(legacy.entries[0].cache_write, 4);
        assert_eq!(legacy.entries[0].message_count, 6);
        assert_eq!(legacy.entries[0].cost, 0.75);
        assert_eq!(legacy.total_cost, 0.75);
        assert_eq!(legacy.processing_time_ms, 42);
    }

    #[test]
    fn monthly_reasoning_matches_model_and_hourly_aggregation() {
        let messages = vec![
            UnifiedMessage::new(
                "opencode",
                "reasoning-model",
                "openai",
                "session-a",
                1_767_225_600_000,
                TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 2,
                    cache_write: 1,
                    reasoning: 7,
                },
                0.1,
            ),
            UnifiedMessage::new(
                "codex",
                "reasoning-model",
                "openai",
                "session-b",
                1_767_229_200_000,
                TokenBreakdown {
                    input: 20,
                    output: 8,
                    cache_read: 3,
                    cache_write: 2,
                    reasoning: 11,
                },
                0.2,
            ),
        ];

        let monthly = aggregate_monthly_usage_v2_entries(messages.clone());
        let models = aggregate_model_usage_entries(messages.clone(), &GroupBy::Model);
        let hourly = aggregate_hourly_usage_entries(
            messages,
            super::bucket_tz::BucketTimezone::from_pinned_name(Some("UTC")),
        );

        assert_eq!(monthly.len(), 1);
        let monthly_reasoning = monthly[0].reasoning;
        let model_reasoning = models
            .iter()
            .fold(0_i64, |total, entry| total.saturating_add(entry.reasoning));
        let hourly_reasoning = hourly
            .iter()
            .fold(0_i64, |total, entry| total.saturating_add(entry.reasoning));
        assert_eq!(monthly_reasoning, 18);
        assert_eq!(monthly_reasoning, model_reasoning);
        assert_eq!(monthly_reasoning, hourly_reasoning);
    }

    #[test]
    fn monthly_aggregation_rejects_malformed_calendar_dates() {
        let message_with_date = |date: &str, input: i64| {
            let mut message = UnifiedMessage::new(
                "codex",
                "model",
                "openai",
                format!("session-{input}"),
                1_767_225_600_000,
                TokenBreakdown {
                    input,
                    ..TokenBreakdown::default()
                },
                0.0,
            );
            message.date = date.to_string();
            message
        };

        let monthly = aggregate_monthly_usage_v2_entries([
            message_with_date("2024-02-29", 1),
            message_with_date("2023-02-29", 2),
            message_with_date("2026-00-01", 4),
            message_with_date("2026-13-01", 8),
            message_with_date("2026-04-31", 16),
            message_with_date("2026-💥", 32),
            message_with_date("2026-01-31", 64),
        ]);

        assert_eq!(monthly.len(), 2);
        assert_eq!(
            monthly
                .iter()
                .find(|entry| entry.month == "2024-02")
                .unwrap()
                .input,
            1
        );
        assert_eq!(
            monthly
                .iter()
                .find(|entry| entry.month == "2026-01")
                .unwrap()
                .input,
            64
        );
    }

    #[test]
    fn monthly_message_count_saturates() {
        let mut first = UnifiedMessage::new(
            "codex",
            "model",
            "openai",
            "session-a",
            1_767_225_600_000,
            TokenBreakdown::default(),
            0.0,
        );
        first.message_count = i32::MAX;
        let second = UnifiedMessage::new(
            "codex",
            "model",
            "openai",
            "session-b",
            1_767_225_601_000,
            TokenBreakdown::default(),
            0.0,
        );

        let monthly = aggregate_monthly_usage_v2_entries([first, second]);
        assert_eq!(monthly[0].message_count, i32::MAX);
    }
    use std::str::FromStr;
    use std::sync::Arc;

    fn parse_all_messages_with_pricing(
        home_dir: &str,
        clients: &[String],
        pricing: Option<&pricing::PricingService>,
    ) -> Vec<UnifiedMessage> {
        parse_all_messages_with_pricing_with_env_strategy(
            home_dir,
            clients,
            pricing,
            false,
            &scanner::ScannerSettings::default(),
        )
    }

    fn large_prime_contents(input: i64, child_input: i64) -> String {
        const FILE_BYTES: usize = 100_000;
        const SEMANTIC_OFFSET: usize = 10_000;
        let before_padding = r#"{"type":"session","version":3,"id":"legacy","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","padding":""#;
        let before_semantic = r#"","usage":{"#;
        let padding_bytes = SEMANTIC_OFFSET
            .checked_sub(before_padding.len() + before_semantic.len())
            .unwrap();
        let mut contents = String::with_capacity(FILE_BYTES);
        contents.push_str(before_padding);
        contents.push_str(&"p".repeat(padding_bytes));
        contents.push_str(before_semantic);
        assert_eq!(contents.len(), SEMANTIC_OFFSET);
        contents.push_str(&format!(
            r#""input":{input},"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":{}}}}}}}
{{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{{"input":{child_input},"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":{child_input}}},"aggregateUsage":{{"input":{input},"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":{}}},"origin":"spawn_task"}}
"#,
            input + 10,
            input + 10,
        ));
        let tail_prefix = r#"{"type":"ignored","padding":""#;
        let tail_suffix = "\"}\n";
        let tail_bytes = FILE_BYTES
            .checked_sub(contents.len() + tail_prefix.len() + tail_suffix.len())
            .unwrap();
        contents.push_str(tail_prefix);
        contents.push_str(&"t".repeat(tail_bytes));
        contents.push_str(tail_suffix);
        assert_eq!(contents.len(), FILE_BYTES);
        contents
    }

    fn home_guard() -> crate::paths::test_env::EnvGuard {
        crate::paths::test_env::EnvGuard::capture(&["HOME"])
    }

    /// Point the message-cache root at a scratch directory for as long as the
    /// returned guard is alive.
    ///
    /// Redirecting `HOME` is enough on Unix and does nothing on Windows:
    /// `paths::get_config_dir` resolves the Windows root through
    /// `dirs::config_dir()`, a known-folder lookup that reads no environment
    /// variable at all. Every test in this module then shared one real
    /// `%APPDATA%\tokscale\cache` and loaded back the shards its neighbours had
    /// written, so the counts came out higher than the entries the test itself
    /// inserted — and which neighbours had run first decided by how much.
    ///
    /// `TOKSCALE_CONFIG_DIR` is the override `paths.rs` documents for exactly
    /// this ("CI sandbox, tests, isolated profile") and it is consulted first on
    /// every platform. On Unix it names the directory the `HOME` redirect
    /// already produced, so nothing moves there; it also pins the root against a
    /// globally-set `XDG_CONFIG_HOME`, which a `HOME`-only redirect leaks past
    /// on Linux runners.
    ///
    /// That reach is exactly why the restore has to be a `Drop` guard rather
    /// than a trailing call. The `HOME`-only redirect this replaced was inert
    /// on Windows, so leaking it past a panicking assertion cost nothing there;
    /// `TOKSCALE_CONFIG_DIR` is consulted first on *every* platform, so a leaked
    /// one points every later test in the binary at a `TempDir` that has already
    /// been dropped — the cross-test contamination this redirect exists to
    /// remove, reintroduced one layer down. `serial_test` does not help: it
    /// prevents overlap, not inheritance.
    #[must_use = "the redirect is undone as soon as the guard drops; bind it to a \
                  named variable that outlives the test body"]
    fn redirect_cache_home(home: &std::path::Path) -> crate::paths::test_env::EnvGuard {
        let mut env = crate::paths::test_env::EnvGuard::capture(&["HOME", "TOKSCALE_CONFIG_DIR"]);
        point_cache_home(&mut env, home);
        env
    }

    /// Re-aim a live [`redirect_cache_home`] at a different scratch directory.
    ///
    /// The tests that compare a warm cache against a cold one switch roots
    /// mid-body, and one switches back again to assert on the first root. They
    /// want a re-point, not a nested guard: the guard already holds the values
    /// from before the *first* redirect, and restoring those once at scope exit
    /// is the correct end state no matter how many times the root moved.
    fn point_cache_home(env: &mut crate::paths::test_env::EnvGuard, home: &std::path::Path) {
        env.set("HOME", home);
        env.set("TOKSCALE_CONFIG_DIR", home.join(".config").join("tokscale"));
    }

    /// A client's scan root under `home`, spelled the way a scan will spell it.
    ///
    /// `ClientDef::resolve_path` pushes each relative component with the
    /// platform separator (#1048), so on Windows a discovered file reads
    /// `C:\home\.claude\projects\demo\session.jsonl`. A fixture that builds the
    /// same file with `Path::join` gets a mixed spelling (`C:\home\.claude/projects\...`)
    /// — the same file, a different string.
    ///
    /// That difference is invisible until a test seeds the message cache by
    /// hand and expects the next scan to find it, because `CachedPath` keys on
    /// the OS string as written: two spellings are two keys, so the seeded
    /// entry is never read and the parse silently falls back to a cold parse.
    /// Seeding under the spelling the scan produces is what these tests mean.
    /// Whether the cache *ought* to fold the two spellings into one key is a
    /// separate question about the product; nothing here depends on the answer.
    fn client_scan_root(home: &std::path::Path, client: ClientId) -> std::path::PathBuf {
        std::path::PathBuf::from(
            client
                .data()
                .resolve_path_with_env_strategy(&home.to_string_lossy(), false),
        )
    }

    /// An explicit `--home` outranks every environment lookup. Pinned so the
    /// reordering that routed the fallback through `paths::home_dir` cannot
    /// quietly promote the resolver above the caller's own argument.
    #[test]
    #[serial]
    fn get_home_dir_string_prefers_the_explicit_option() {
        let mut env = home_guard();
        env.set("HOME", "/tmp/tokscale-env-home");
        assert_eq!(
            get_home_dir_string(&Some("/tmp/tokscale-explicit-home".to_string())),
            Ok("/tmp/tokscale-explicit-home".to_string())
        );
    }

    /// The bypass this test exists for: reading `$HOME` directly meant an
    /// exported-but-blank value won outright and produced `Ok("")`. Every
    /// consumer builds scan roots with `format!("{home}/...")`, so an empty
    /// home turns each of them into an absolute path from the filesystem root
    /// — `/.codex/sessions` rather than `~/.codex/sessions`.
    ///
    /// `paths::home_dir` delegates to `dirs`, which treats a blank `HOME` as
    /// unset and falls back to the passwd entry, so the empty string can no
    /// longer escape. Asserting "not `Ok("")`" rather than a concrete path
    /// keeps this honest on a runner with no passwd home, where the correct
    /// answer is the `Err` arm.
    #[test]
    #[serial]
    fn get_home_dir_string_never_returns_an_empty_home() {
        let mut env = home_guard();
        env.set("HOME", "");
        let resolved = get_home_dir_string(&None);
        assert_ne!(
            resolved,
            Ok(String::new()),
            "a blank HOME must not resolve to the empty string; \
             every caller joins it into a scan root"
        );
    }

    /// MSYS2, Cygwin and Git Bash export `HOME=/home/<user>` on Windows.
    /// Returning that verbatim points the model, monthly, hourly and local
    /// parsers at `C:\home\<user>` — `Path` reads the leading `/` as the root
    /// of the current drive — so a Git Bash user sees none of their own usage.
    /// `paths::home_dir` rejects the shape; this test pins that
    /// `get_home_dir_string` actually goes through it rather than around it.
    ///
    /// Windows-only by construction: `/home/runner` is a legitimate absolute
    /// path on macOS and the resolver rightly honors it there. It does run —
    /// on the `windows-latest` leg this PR adds.
    #[test]
    #[serial]
    #[cfg(windows)]
    fn get_home_dir_string_ignores_a_posix_shaped_home_on_windows() {
        let mut env = home_guard();
        env.set("HOME", "/home/runner");
        let resolved = get_home_dir_string(&None);
        assert_ne!(
            resolved,
            Ok("/home/runner".to_string()),
            "a POSIX-shaped HOME must not reach the scanners on Windows"
        );
    }

    #[test]
    fn token_total_saturates_on_overlarge_buckets() {
        // Multiple clamped (i64::MAX) buckets from a corrupt source must
        // saturate rather than overflow when summed.
        let t = TokenBreakdown {
            input: i64::MAX,
            output: i64::MAX,
            cache_read: i64::MAX,
            cache_write: 0,
            reasoning: 0,
        };
        assert_eq!(t.total(), i64::MAX);
        assert_eq!(super::positive_token_total(&t), i64::MAX);
    }

    #[test]
    fn model_aggregation_saturates_overflowing_token_folds() {
        // token_total_saturates_on_overlarge_buckets covers a single message's
        // grand total; the per-field CROSS-MESSAGE fold in
        // aggregate_model_usage_entries must saturate too. An antigravity-cli
        // row can carry an i64::MAX bucket after the untrusted-varint clamp
        // (sessions/antigravity_cli.rs to_i64), so two such rows folded into one
        // model group with plain `+=` overflow (debug panic / release wrap)
        // before the already-saturating grand total runs.
        let make = || {
            UnifiedMessage::new_with_dedup(
                "antigravity-cli",
                "gemini-3-pro",
                "antigravity",
                "session-overflow",
                1_733_011_200_000,
                TokenBreakdown {
                    input: i64::MAX,
                    output: 0,
                    cache_read: i64::MAX,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                None,
            )
        };

        let entries = aggregate_model_usage_entries(vec![make(), make()], &GroupBy::Model);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].input, i64::MAX);
        assert_eq!(entries[0].cache_read, i64::MAX);
    }

    #[test]
    fn model_report_totals_saturate_across_groups() {
        // aggregate_model_usage_entries saturates each entry's fields, so an
        // entry can be i64::MAX. get_model_report sums the entries into the
        // report-level totals via model_report_token_totals; two saturated
        // entries (two distinct models) must not overflow that sum either.
        let make = |model: &str| {
            UnifiedMessage::new_with_dedup(
                "antigravity-cli",
                model,
                "antigravity",
                "session-overflow",
                1_733_011_200_000,
                TokenBreakdown {
                    input: i64::MAX,
                    output: 0,
                    cache_read: i64::MAX,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                None,
            )
        };

        let entries = aggregate_model_usage_entries(
            vec![make("gemini-3-pro"), make("claude-opus-4-6")],
            &GroupBy::Model,
        );
        assert_eq!(entries.len(), 2);
        let (total_input, _total_output, total_cache_read, _total_cache_write) =
            super::model_report_token_totals(&entries);
        assert_eq!(total_input, i64::MAX);
        assert_eq!(total_cache_read, i64::MAX);
    }

    fn make_workspace_message(
        client: &str,
        model_id: &str,
        provider_id: &str,
        session_id: &str,
        cost: f64,
        workspace_key: Option<&str>,
        workspace_label: Option<&str>,
    ) -> UnifiedMessage {
        let mut msg = UnifiedMessage::new(
            client,
            model_id,
            provider_id,
            session_id,
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
        );
        msg.set_workspace(
            workspace_key.map(str::to_string),
            workspace_label.map(str::to_string),
        );
        msg
    }

    fn make_workbuddy_message(
        session_id: &str,
        timestamp: i64,
        input: i64,
        dedup_key: &str,
    ) -> UnifiedMessage {
        let mut msg = UnifiedMessage::new(
            "workbuddy",
            "glm-5.2",
            "zai",
            session_id,
            timestamp,
            TokenBreakdown {
                input,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );
        msg.dedup_key = Some(dedup_key.to_string());
        msg
    }

    fn make_trae_message(
        session_id: &str,
        timestamp: i64,
        dedup_key: Option<&str>,
        cost: f64,
    ) -> UnifiedMessage {
        UnifiedMessage::new_with_dedup(
            "trae",
            "gpt-5.2",
            "openai",
            session_id,
            timestamp,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            dedup_key.map(str::to_string),
        )
    }

    #[test]
    fn workbuddy_fallback_dedups_by_session_not_date() {
        const DAY1: i64 = 1_782_883_200_000;
        const DAY2: i64 = 1_782_969_600_000;

        // Session A has detailed coverage on DAY1.
        let detailed = vec![make_workbuddy_message(
            "sess-A",
            DAY1,
            100,
            "workbuddy:detailed-A",
        )];
        let fallback = vec![
            // Session A's cumulative SQLite aggregate is dated DAY2 (updated_at)
            // even though its detailed activity was DAY1. The old date-overlap
            // check kept it, double-counting the whole session on DAY2.
            make_workbuddy_message("sess-A", DAY2, 5000, "workbuddy:fallback-A"),
            // Session B has NO detailed coverage but its aggregate shares DAY1
            // with session A's detail. The old check dropped it, losing usage.
            make_workbuddy_message("sess-B", DAY1, 2000, "workbuddy:fallback-B"),
        ];

        let merged = super::merge_workbuddy_messages(detailed, fallback);

        // Detailed A kept; fallback A dropped (session covered); fallback B kept.
        assert_eq!(merged.len(), 2);
        assert!(merged
            .iter()
            .any(|message| message.dedup_key.as_deref() == Some("workbuddy:detailed-A")));
        assert!(merged
            .iter()
            .any(|message| message.dedup_key.as_deref() == Some("workbuddy:fallback-B")));
        assert!(!merged
            .iter()
            .any(|message| message.dedup_key.as_deref() == Some("workbuddy:fallback-A")));
    }

    #[test]
    fn workbuddy_fallback_kept_when_no_detailed_messages() {
        // With zero detailed coverage, every fallback session survives.
        let fallback = vec![
            make_workbuddy_message("sess-A", 1_782_883_200_000, 1000, "workbuddy:fallback-A"),
            make_workbuddy_message("sess-B", 1_782_969_600_000, 2000, "workbuddy:fallback-B"),
        ];

        let merged = super::merge_workbuddy_messages(Vec::new(), fallback);

        assert_eq!(merged.len(), 2);
    }

    #[allow(clippy::too_many_arguments)]
    fn build_opencode_sqlite_payload(
        created_ms: f64,
        completed_ms: f64,
        input: i64,
        output: i64,
        reasoning: i64,
        cache_read: i64,
        cache_write: i64,
        cost: f64,
    ) -> String {
        format!(
            r#"{{
                "role": "assistant",
                "modelID": "claude-sonnet-4",
                "providerID": "anthropic",
                "cost": {cost},
                "tokens": {{
                    "input": {input},
                    "output": {output},
                    "reasoning": {reasoning},
                    "cache": {{ "read": {cache_read}, "write": {cache_write} }}
                }},
                "time": {{ "created": {created_ms}, "completed": {completed_ms} }},
                "mode": "build"
            }}"#
        )
    }

    fn create_opencode_sqlite_db(db_path: &std::path::Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    fn create_hermes_sqlite_db(db_path: &std::path::Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                model TEXT,
                started_at REAL NOT NULL,
                message_count INTEGER DEFAULT 0,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                cache_read_tokens INTEGER DEFAULT 0,
                cache_write_tokens INTEGER DEFAULT 0,
                reasoning_tokens INTEGER DEFAULT 0,
                billing_provider TEXT,
                estimated_cost_usd REAL,
                actual_cost_usd REAL
            );
            CREATE TABLE session_model_usage (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                model TEXT NOT NULL,
                billing_provider TEXT NOT NULL DEFAULT '',
                billing_base_url TEXT NOT NULL DEFAULT '',
                billing_mode TEXT NOT NULL DEFAULT '',
                task TEXT NOT NULL DEFAULT '',
                api_call_count INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost_usd REAL NOT NULL DEFAULT 0,
                actual_cost_usd REAL NOT NULL DEFAULT 0,
                cost_status TEXT,
                cost_source TEXT,
                first_seen REAL,
                last_seen REAL,
                PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task)
            );",
        )
        .unwrap();
        conn
    }

    fn create_zed_sqlite_db(db_path: &std::path::Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                data_type TEXT NOT NULL,
                data BLOB NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    fn insert_zed_thread(conn: &rusqlite::Connection, id: &str, model: &str) {
        let payload = format!(
            r#"{{
                "version": "0.3.0",
                "title": "Test thread",
                "updated_at": "2026-05-01T12:30:00Z",
                "request_token_usage": {{
                    "turn-1": {{
                        "input_tokens": 42,
                        "output_tokens": 7,
                        "cache_creation_input_tokens": 3,
                        "cache_read_input_tokens": 5
                    }}
                }},
                "model": {{
                    "provider": "zed.dev",
                    "model": "{model}"
                }},
                "imported": false
            }}"#
        );
        conn.execute(
            "INSERT INTO threads (id, summary, updated_at, data_type, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, "Test thread", "2026-05-01T12:30:00Z", "json", payload.as_bytes()],
        )
        .unwrap();
    }

    fn insert_hermes_session(
        conn: &rusqlite::Connection,
        id: &str,
        model: &str,
        message_count: i64,
        input_tokens: i64,
        output_tokens: i64,
        actual_cost_usd: f64,
    ) {
        conn.execute(
            "INSERT INTO sessions (
                id, source, model, started_at, message_count,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
                billing_provider, estimated_cost_usd, actual_cost_usd
            ) VALUES (?1, 'cli', ?2, 1775001102.0, ?3, ?4, ?5, 0, 0, 0, 'anthropic', NULL, ?6)",
            rusqlite::params![
                id,
                model,
                message_count,
                input_tokens,
                output_tokens,
                actual_cost_usd
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_model_usage (
                session_id, model, billing_provider, billing_base_url, billing_mode, task,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
                estimated_cost_usd, actual_cost_usd
            ) VALUES (?1, ?2, 'anthropic', '', '', '', ?3, ?4, 0, 0, 0, 0, ?5)",
            rusqlite::params![
                id,
                model,
                input_tokens,
                output_tokens,
                actual_cost_usd
            ],
        )
        .unwrap();
    }

    #[test]
    fn test_normalize_model_for_grouping() {
        assert_eq!(
            normalize_model_for_grouping("claude-opus-4-5-20251101"),
            "claude-opus-4-5"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-sonnet-4-5-20250929"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-sonnet-4-20250514"),
            "claude-sonnet-4"
        );

        assert_eq!(
            normalize_model_for_grouping("claude-opus-4.5"),
            "claude-opus-4-5"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-sonnet-4.5"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-opus-4.6"),
            "claude-opus-4-6"
        );
        assert_eq!(
            normalize_model_for_grouping("anthropic/claude-4-6-sonnet"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            normalize_model_for_grouping("anthropic/claude-4-5-haiku"),
            "claude-haiku-4-5"
        );
        assert_eq!(
            normalize_model_for_grouping("anthropic/claude-4-6-opus"),
            "claude-opus-4-6"
        );

        assert_eq!(normalize_model_for_grouping("gpt-5.2"), "gpt-5.2");
        assert_eq!(normalize_model_for_grouping("gpt-5.4(xhigh)"), "gpt-5.4");
        assert_eq!(normalize_model_for_grouping("gpt-5.4(high)"), "gpt-5.4");
        assert_eq!(normalize_model_for_grouping("gpt-5.4(minimal)"), "gpt-5.4");
        assert_eq!(normalize_model_for_grouping("gpt-5.4(auto)"), "gpt-5.4");
        assert_eq!(normalize_model_for_grouping("gpt-5.4(none)"), "gpt-5.4");
        assert_eq!(
            normalize_model_for_grouping("gpt-5.4(weirdgarbage)"),
            "gpt-5.4(weirdgarbage)"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-sonnet-4.5(high)"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            normalize_model_for_grouping("gemini-3-pro(auto)"),
            "gemini-3-pro"
        );
        assert_eq!(
            normalize_model_for_grouping("gemini-2.5-pro"),
            "gemini-2.5-pro"
        );

        assert_eq!(
            normalize_model_for_grouping("claude-opus-4-5-high"),
            "claude-opus-4-5-high"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-opus-4-5-thinking-high"),
            "claude-opus-4-5-thinking-high"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-sonnet-4-5-high"),
            "claude-sonnet-4-5-high"
        );

        assert_eq!(
            normalize_model_for_grouping("claude-4-sonnet"),
            "claude-4-sonnet"
        );
        assert_eq!(
            normalize_model_for_grouping("claude-4-opus-thinking"),
            "claude-4-opus-thinking"
        );

        assert_eq!(normalize_model_for_grouping("big-pickle"), "big-pickle");
        assert_eq!(normalize_model_for_grouping("grok-code"), "grok-code");

        assert_eq!(
            normalize_model_for_grouping("claude-opus-4.5-20251101"),
            "claude-opus-4-5"
        );
    }

    #[test]
    fn test_group_by_from_str_valid_values() {
        assert_eq!(GroupBy::from_str("model").unwrap(), GroupBy::Model);
        assert_eq!(
            GroupBy::from_str("client,model").unwrap(),
            GroupBy::ClientModel
        );
        assert_eq!(
            GroupBy::from_str("client-model").unwrap(),
            GroupBy::ClientModel
        );
        assert_eq!(
            GroupBy::from_str("client,provider,model").unwrap(),
            GroupBy::ClientProviderModel
        );
        assert_eq!(
            GroupBy::from_str("client-provider-model").unwrap(),
            GroupBy::ClientProviderModel
        );
        assert_eq!(
            GroupBy::from_str("workspace,model").unwrap(),
            GroupBy::WorkspaceModel
        );
        assert_eq!(
            GroupBy::from_str("workspace-model").unwrap(),
            GroupBy::WorkspaceModel
        );
        assert_eq!(GroupBy::from_str("session").unwrap(), GroupBy::Session);
        assert_eq!(
            GroupBy::from_str("session,model").unwrap(),
            GroupBy::Session
        );
        assert_eq!(
            GroupBy::from_str("session-model").unwrap(),
            GroupBy::Session
        );
        assert_eq!(
            GroupBy::from_str("client,session").unwrap(),
            GroupBy::ClientSession
        );
        assert_eq!(
            GroupBy::from_str("client,session,model").unwrap(),
            GroupBy::ClientSession
        );
        assert_eq!(
            GroupBy::from_str("client-session-model").unwrap(),
            GroupBy::ClientSession
        );
        assert!(GroupBy::from_str("unknown").is_err());
    }

    #[test]
    fn test_group_by_default_is_client_model() {
        assert_eq!(GroupBy::default(), GroupBy::ClientModel);
    }

    #[test]
    fn test_group_by_display_round_trips_with_from_str() {
        let variants = [
            GroupBy::Model,
            GroupBy::ClientModel,
            GroupBy::ClientProviderModel,
            GroupBy::WorkspaceModel,
            GroupBy::Session,
            GroupBy::ClientSession,
        ];

        for variant in variants {
            let rendered = variant.to_string();
            let parsed = GroupBy::from_str(&rendered).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn test_group_by_from_str_whitespace_handling() {
        assert_eq!(
            GroupBy::from_str("client, model").unwrap(),
            GroupBy::ClientModel
        );
        assert_eq!(GroupBy::from_str(" model ").unwrap(), GroupBy::Model);
        assert_eq!(
            GroupBy::from_str("client , provider , model").unwrap(),
            GroupBy::ClientProviderModel
        );
        assert_eq!(
            GroupBy::from_str("workspace, model").unwrap(),
            GroupBy::WorkspaceModel
        );
    }

    #[test]
    fn test_model_usage_performance_uses_only_timed_positive_token_messages() {
        let mut timed = make_workspace_message(
            "opencode",
            "gpt-5.4",
            "openai",
            "session-1",
            0.0,
            None,
            None,
        );
        timed.tokens = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 25,
            cache_write: 0,
            reasoning: 25,
        };
        timed.duration_ms = Some(400);

        let mut untimed = make_workspace_message(
            "opencode",
            "gpt-5.4",
            "openai",
            "session-2",
            0.0,
            None,
            None,
        );
        untimed.tokens = TokenBreakdown {
            input: 300,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };

        let entries = aggregate_model_usage_entries(vec![timed, untimed], &GroupBy::ClientModel);

        assert_eq!(entries.len(), 1);
        let performance = &entries[0].performance;
        assert_eq!(performance.total_duration_ms, 400);
        assert_eq!(performance.timed_tokens, 200);
        assert_eq!(performance.sample_count, 1);
        assert_eq!(performance.ms_per_1k_tokens, Some(2000.0));
        assert!((performance.token_coverage - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_model_usage_performance_is_null_without_duration_samples() {
        let entries = aggregate_model_usage_entries(
            vec![make_workspace_message(
                "claude",
                "claude-sonnet-4-5",
                "anthropic",
                "session-1",
                0.0,
                None,
                None,
            )],
            &GroupBy::ClientModel,
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].performance.ms_per_1k_tokens, None);
        assert_eq!(entries[0].performance.total_duration_ms, 0);
        assert_eq!(entries[0].performance.timed_tokens, 0);
        assert_eq!(entries[0].performance.token_coverage, 0.0);
    }

    #[test]
    fn test_workspace_model_grouping_merges_same_workspace_and_model() {
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.25,
                    Some("/repo-a"),
                    Some("repo-a"),
                ),
                make_workspace_message(
                    "qwen",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.75,
                    Some("/repo-a"),
                    Some("repo-a"),
                ),
            ],
            &GroupBy::WorkspaceModel,
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model, "claude-sonnet-4-5");
        assert_eq!(entries[0].workspace_key.as_deref(), Some("/repo-a"));
        assert_eq!(entries[0].workspace_label.as_deref(), Some("repo-a"));
        assert_eq!(entries[0].cost, 4.0);
        assert_eq!(entries[0].message_count, 2);
        assert_eq!(entries[0].merged_clients.as_deref(), Some("claude, qwen"));
    }

    #[test]
    fn test_model_grouping_merges_anthropic_prefixed_claude_variant_with_canonical_model() {
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message(
                    "claude",
                    "anthropic/claude-4-6-sonnet",
                    "anthropic",
                    "session-1",
                    1.25,
                    Some("/repo-a"),
                    Some("repo-a"),
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-6",
                    "anthropic",
                    "session-2",
                    2.75,
                    Some("/repo-b"),
                    Some("repo-b"),
                ),
            ],
            &GroupBy::ClientModel,
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model, "claude-sonnet-4-6");
        assert_eq!(entries[0].input, 20);
        assert_eq!(entries[0].output, 10);
        assert_eq!(entries[0].cost, 4.0);
        assert_eq!(entries[0].message_count, 2);
    }

    #[test]
    fn worktree_rollup_merges_worktrees_of_one_repo_into_a_single_row() {
        let messages = vec![
            make_workspace_message(
                "claude",
                "claude-sonnet-4-5-20250929",
                "anthropic",
                "session-1",
                1.0,
                Some("/repo-a/.claude/worktrees/feature-x"),
                None,
            ),
            make_workspace_message(
                "claude",
                "claude-sonnet-4-5-20250929",
                "anthropic",
                "session-2",
                2.0,
                Some("/repo-a/.claude/worktrees/feature-y"),
                None,
            ),
            make_workspace_message(
                "claude",
                "claude-sonnet-4-5-20250929",
                "anthropic",
                "session-3",
                4.0,
                Some("/repo-a"),
                None,
            ),
        ];

        let separate = aggregate_model_usage_entries_with_rollup(
            messages.clone(),
            &GroupBy::WorkspaceModel,
            WorktreeRollup::Separate,
        );
        assert_eq!(separate.len(), 3, "each worktree stays its own row");

        let merged = aggregate_model_usage_entries_with_rollup(
            messages,
            &GroupBy::WorkspaceModel,
            WorktreeRollup::MergeIntoRepo,
        );
        assert_eq!(merged.len(), 1, "every worktree folds into the repo row");
        assert_eq!(merged[0].workspace_key.as_deref(), Some("/repo-a"));
        assert_eq!(merged[0].workspace_label.as_deref(), Some("repo-a"));
        // No usage may be lost or double counted by the rollup.
        assert_eq!(merged[0].cost, 7.0);
        assert_eq!(merged[0].message_count, 3);
    }

    #[test]
    fn worktree_rollup_labels_name_the_repo_and_the_worktree() {
        let entries = aggregate_model_usage_entries_with_rollup(
            vec![make_workspace_message(
                "claude",
                "claude-sonnet-4-5-20250929",
                "anthropic",
                "session-1",
                1.0,
                Some("/repo-a/.claude/worktrees/feature-x"),
                None,
            )],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::Separate,
        );

        // Without rollup the row must still say WHICH worktree it is -- the bug
        // was a label that truncated to a shared, indistinguishable prefix.
        assert_eq!(
            entries[0].workspace_label.as_deref(),
            Some("repo-a ⑃ feature-x")
        );
    }

    #[test]
    fn worktree_rollup_merges_a_slug_key_with_the_same_repos_real_path() {
        // Claude Code writes a dash-mangled slug and Codex/OpenCode write real
        // paths for the SAME directory. Under rollup both must resolve to one
        // identity, or "one row per repo" silently still yields two.
        // Spell the fixture the way `read_dir` reports it -- see
        // `sessions::canonical_tempdir` for why that is not just `tempdir()`.
        let (_temp, temp_root) = crate::sessions::canonical_tempdir();
        let repo = temp_root.join("devpro/ing/claude-witness");
        std::fs::create_dir_all(repo.join(".claude/worktrees/feature-x")).unwrap();

        let real_path = crate::sessions::normalize_workspace_key(&repo.to_string_lossy()).unwrap();
        let worktree_path = crate::sessions::normalize_workspace_key(
            &repo.join(".claude/worktrees/feature-x").to_string_lossy(),
        )
        .unwrap();
        // How Claude Code would name that worktree's project directory.
        let slug: String = worktree_path
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();

        let entries = aggregate_model_usage_entries_with_rollup(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some(&slug),
                    None,
                ),
                make_workspace_message(
                    "opencode",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some(&real_path),
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::MergeIntoRepo,
        );

        assert_eq!(entries.len(), 1, "slug and real path must share one row");
        assert_eq!(entries[0].cost, 3.0);
        assert_eq!(
            entries[0].workspace_label.as_deref(),
            Some("claude-witness")
        );
    }

    /// The same directory recorded twice — Claude Code's slug and another
    /// client's real path — stays two rows without rollup, and those two rows
    /// must still be tellable apart. No parent segment can do it (both resolve
    /// to one directory), so the key does; escalating the parent qualifier
    /// first would only push that key off a narrow row.
    #[test]
    fn same_directory_under_two_key_formats_is_separated_by_the_key() {
        let (_temp, temp_root) = crate::sessions::canonical_tempdir();
        let repo = temp_root.join("devpro/ing/claude-witness");
        std::fs::create_dir_all(&repo).unwrap();

        let real_path = crate::sessions::normalize_workspace_key(&repo.to_string_lossy()).unwrap();
        let slug: String = real_path
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();

        let entries = aggregate_model_usage_entries_with_rollup(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some(&slug),
                    None,
                ),
                make_workspace_message(
                    "opencode",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some(&real_path),
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::Separate,
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries.iter().map(|entry| entry.cost).sum::<f64>(), 3.0);
        let labels: HashSet<&str> = entries
            .iter()
            .map(|entry| entry.workspace_label.as_deref().unwrap())
            .collect();
        assert_eq!(labels.len(), 2, "rows must be tellable apart: {labels:?}");
        for label in labels {
            assert!(
                label.starts_with("claude-witness ("),
                "the key qualifies the base label, not a longer path: {label}"
            );
        }
    }

    #[test]
    fn workspace_rows_keep_parser_supplied_labels_for_non_path_keys() {
        // Warp keys a workspace by opaque UUID; only the parser can name it, so
        // relabeling must not clobber that.
        let entries = aggregate_model_usage_entries_with_rollup(
            vec![make_workspace_message(
                "warp",
                "claude-sonnet-4-5-20250929",
                "anthropic",
                "session-1",
                1.0,
                Some("9f2c1a04-1e4b-4c3f-a0d1-77b2e5c9aa10"),
                Some("Ing's Team"),
            )],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::MergeIntoRepo,
        );

        assert_eq!(entries[0].workspace_label.as_deref(), Some("Ing's Team"));
        assert_eq!(
            entries[0].workspace_key.as_deref(),
            Some("9f2c1a04-1e4b-4c3f-a0d1-77b2e5c9aa10")
        );
    }

    /// Resolving one workspace key used to walk the filesystem three times —
    /// once each for the label, the path and the repo root — so a slug that took
    /// seconds to decode cost three times that per row. The decode is memoized
    /// on the labeler and shared by all three.
    #[test]
    fn workspace_labeler_decodes_each_key_once() {
        let mut labeler = crate::WorkspaceLabeler::default();
        let key = "-nonexistent-tokscale-decode-probe";

        let label = labeler.label(key);
        let path = labeler.path(key);
        let root = labeler.repo_root(key);
        assert_eq!(
            labeler.decoded_key_count(),
            1,
            "label/path/repo_root must share one decode"
        );

        // Repeating every call adds no decodes and changes no answers.
        assert_eq!(labeler.label(key), label);
        assert_eq!(labeler.path(key), path);
        assert_eq!(labeler.repo_root(key), root);
        assert_eq!(labeler.decoded_key_count(), 1);
    }

    /// Two directories that share a basename produced the same row text, which
    /// made `--group-by workspace,model` unreadable exactly where it matters:
    /// the rows are distinct and correctly separated, but nothing on screen said
    /// which repo each one was.
    #[test]
    fn workspace_rows_disambiguate_colliding_basenames() {
        let entries = aggregate_model_usage_entries_with_rollup(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some("/work/proj"),
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some("/oss/proj"),
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::Separate,
        );

        let labels: HashMap<&str, &str> = entries
            .iter()
            .map(|entry| {
                (
                    entry.workspace_key.as_deref().unwrap(),
                    entry.workspace_label.as_deref().unwrap(),
                )
            })
            .collect();
        assert_eq!(labels.get("/work/proj"), Some(&"work/proj"));
        assert_eq!(labels.get("/oss/proj"), Some(&"oss/proj"));
        // Grouping is untouched: only the display string changed.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.iter().map(|entry| entry.cost).sum::<f64>(), 3.0);
    }

    /// One parent segment is not always enough. The qualifier has to keep
    /// walking up until the rows actually differ, and stop as soon as they do.
    #[test]
    fn workspace_label_qualifier_walks_up_until_the_rows_differ() {
        let entries = aggregate_model_usage_entries_with_rollup(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some("/home/x/shared/api"),
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some("/home/z/shared/api"),
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-3",
                    4.0,
                    Some("/home/x/other/api"),
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::Separate,
        );

        let labels: HashMap<&str, &str> = entries
            .iter()
            .map(|entry| {
                (
                    entry.workspace_key.as_deref().unwrap(),
                    entry.workspace_label.as_deref().unwrap(),
                )
            })
            .collect();
        // `shared/api` collides, so the group escalates one more segment -- and
        // every member of the group escalates together, so the labels stay
        // comparable to each other.
        assert_eq!(labels.get("/home/x/shared/api"), Some(&"x/shared/api"));
        assert_eq!(labels.get("/home/z/shared/api"), Some(&"z/shared/api"));
        assert_eq!(labels.get("/home/x/other/api"), Some(&"x/other/api"));
    }

    /// Non-git directories are workspaces too: a plain folder must disambiguate
    /// the same way, and a folder that merely LOOKS like git metadata
    /// (`notes.git/worktrees/...`) must not be mistaken for a worktree.
    #[test]
    fn workspace_rows_disambiguate_non_git_paths() {
        let entries = aggregate_model_usage_entries_with_rollup(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some("/home/me/Documents/notes"),
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some("/home/me/Dropbox/notes"),
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-3",
                    4.0,
                    Some("/home/me/notes.git/worktrees/notes"),
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::MergeIntoRepo,
        );

        let labels: HashMap<&str, &str> = entries
            .iter()
            .map(|entry| {
                (
                    entry.workspace_key.as_deref().unwrap(),
                    entry.workspace_label.as_deref().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            labels.get("/home/me/Documents/notes"),
            Some(&"Documents/notes")
        );
        assert_eq!(labels.get("/home/me/Dropbox/notes"), Some(&"Dropbox/notes"));
        // `notes.git` is a directory name, not git metadata: the row keeps its
        // own key even under rollup, and is labeled from its real parent.
        assert_eq!(
            labels.get("/home/me/notes.git/worktrees/notes"),
            Some(&"worktrees/notes")
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.iter().map(|entry| entry.cost).sum::<f64>(), 7.0);
    }

    /// Windows keys arrive with backslashes from clients that never normalized
    /// them. Splitting on `/` alone made the whole path the label, which is the
    /// unreadable row this labeling exists to prevent.
    #[test]
    fn workspace_rows_label_windows_style_paths() {
        let entries = aggregate_model_usage_entries_with_rollup(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some(r"C:\work\api"),
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some(r"D:\work\api"),
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-3",
                    4.0,
                    Some(r"C:\work\api\.claude\worktrees\feature-x"),
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::Separate,
        );

        let labels: HashMap<&str, &str> = entries
            .iter()
            .map(|entry| {
                (
                    entry.workspace_key.as_deref().unwrap(),
                    entry.workspace_label.as_deref().unwrap(),
                )
            })
            .collect();
        assert_eq!(labels.get(r"C:\work\api"), Some(&"C:/work/api"));
        assert_eq!(labels.get(r"D:\work\api"), Some(&"D:/work/api"));
        // The worktree row names its repo and its worktree, and does not collide
        // with either repo row, so it needs no qualifier.
        assert_eq!(
            labels.get(r"C:\work\api\.claude\worktrees\feature-x"),
            Some(&"api ⑃ feature-x")
        );
        // Keys are never rewritten -- a Windows key still groups as it arrived.
        assert_eq!(entries.len(), 3);
    }

    /// Nothing on disk can separate two opaque client ids that were given the
    /// same name, so the row falls back to the grouping key, which is unique by
    /// construction. Distinguishable beats pretty here.
    #[test]
    fn workspace_rows_fall_back_to_the_key_when_nothing_else_separates_them() {
        let entries = aggregate_model_usage_entries_with_rollup(
            vec![
                make_workspace_message(
                    "warp",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some("9f2c1a04-1e4b-4c3f-a0d1-77b2e5c9aa10"),
                    Some("Platform"),
                ),
                make_workspace_message(
                    "warp",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some("0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9"),
                    Some("Platform"),
                ),
            ],
            &GroupBy::WorkspaceModel,
            WorktreeRollup::Separate,
        );

        let labels: HashSet<&str> = entries
            .iter()
            .map(|entry| entry.workspace_label.as_deref().unwrap())
            .collect();
        assert_eq!(
            labels.len(),
            2,
            "every row must be tellable apart: {labels:?}"
        );
        assert!(labels
            .iter()
            .all(|label| label.starts_with("Platform (") && label.ends_with(')')));
    }

    #[test]
    fn test_workspace_model_grouping_separates_different_workspaces() {
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some("/repo-a"),
                    Some("repo-a"),
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    Some("/repo-b"),
                    Some("repo-b"),
                ),
            ],
            &GroupBy::WorkspaceModel,
        );

        assert_eq!(entries.len(), 2);
        let labels: HashSet<_> = entries
            .iter()
            .map(|entry| entry.workspace_label.as_deref().unwrap())
            .collect();
        assert_eq!(labels, HashSet::from(["repo-a", "repo-b"]));
    }

    #[test]
    fn test_workspace_model_grouping_uses_unknown_bucket_without_workspace_metadata() {
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    None,
                    None,
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    "2.0".parse().unwrap(),
                    None,
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].workspace_key, None);
        assert_eq!(
            entries[0].workspace_label.as_deref(),
            Some(UNKNOWN_WORKSPACE_LABEL)
        );
        assert_eq!(entries[0].message_count, 2);
        assert_eq!(entries[0].cost, 3.0);
    }

    #[test]
    fn test_parsed_round_trip_preserves_workspace_metadata() {
        let mut unified = UnifiedMessage::new(
            "qwen",
            "qwen3.5-plus",
            "qwen",
            "session-1",
            1_742_390_400_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 2,
                cache_write: 0,
                reasoning: 1,
            },
            1.25,
        );
        unified.set_workspace(
            Some("//server/share/demo-workspace".to_string()),
            Some("demo-workspace".to_string()),
        );
        unified.duration_ms = Some(2500);

        let parsed = unified_to_parsed(&unified);
        let round_tripped = parsed_to_unified(&parsed, 2.5);

        assert_eq!(
            round_tripped.workspace_key.as_deref(),
            Some("//server/share/demo-workspace")
        );
        assert_eq!(
            round_tripped.workspace_label.as_deref(),
            Some("demo-workspace")
        );
        assert_eq!(round_tripped.cost, 2.5);
        assert_eq!(round_tripped.duration_ms, Some(2500));
    }

    #[test]
    fn test_workspace_model_grouping_keeps_real_unknown_workspace_separate() {
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-1",
                    1.0,
                    Some("unknown-workspace"),
                    Some("unknown-workspace"),
                ),
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-2",
                    2.0,
                    None,
                    None,
                ),
            ],
            &GroupBy::WorkspaceModel,
        );

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| {
            entry.workspace_key.as_deref() == Some("unknown-workspace")
                && entry.workspace_label.as_deref() == Some("unknown-workspace")
                && (entry.cost - 1.0).abs() < f64::EPSILON
        }));
        assert!(entries.iter().any(|entry| {
            entry.workspace_key.is_none()
                && entry.workspace_label.as_deref() == Some(UNKNOWN_WORKSPACE_LABEL)
                && (entry.cost - 2.0).abs() < f64::EPSILON
        }));
    }

    #[test]
    fn test_session_grouping_merges_same_session_and_model() {
        // Two messages with the same session_id + same model — should collapse
        // into one row regardless of the client that produced them, because
        // GroupBy::Session keys on (session_id, model) only.
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-shared",
                    1.25,
                    None,
                    None,
                ),
                make_workspace_message(
                    "amp",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-shared",
                    2.75,
                    None,
                    None,
                ),
            ],
            &GroupBy::Session,
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session_id.as_deref(), Some("session-shared"));
        assert_eq!(entries[0].model, "claude-sonnet-4-5");
        assert!((entries[0].cost - 4.0).abs() < f64::EPSILON);
        assert_eq!(entries[0].message_count, 2);
        assert!(entries[0].workspace_key.is_none());
        assert!(entries[0].workspace_label.is_none());
        // Session grouping does not merge_clients into a comma list.
        assert!(entries[0].merged_clients.is_none());
    }

    #[test]
    fn test_session_grouping_separates_different_sessions() {
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message("codex", "gpt-5", "openai", "session-a", 1.0, None, None),
                make_workspace_message("codex", "gpt-5", "openai", "session-b", 2.0, None, None),
            ],
            &GroupBy::Session,
        );

        assert_eq!(entries.len(), 2);
        let session_ids: HashSet<_> = entries
            .iter()
            .map(|e| e.session_id.as_deref().unwrap())
            .collect();
        assert_eq!(session_ids, HashSet::from(["session-a", "session-b"]));
    }

    #[test]
    fn test_client_session_grouping_keeps_clients_separate() {
        // Same session_id seen by two different clients (unusual in practice
        // but possible if parsers collide on an id space). ClientSession
        // must yield two rows; Session would yield one (covered above).
        let entries = aggregate_model_usage_entries(
            vec![
                make_workspace_message(
                    "claude",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-shared",
                    1.0,
                    None,
                    None,
                ),
                make_workspace_message(
                    "amp",
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "session-shared",
                    3.0,
                    None,
                    None,
                ),
            ],
            &GroupBy::ClientSession,
        );

        assert_eq!(entries.len(), 2);
        for entry in &entries {
            assert_eq!(entry.session_id.as_deref(), Some("session-shared"));
            assert!(entry.merged_clients.is_none());
        }
        let by_client: HashSet<_> = entries.iter().map(|e| e.client.as_str()).collect();
        assert_eq!(by_client, HashSet::from(["claude", "amp"]));
    }

    #[test]
    fn test_non_session_grouping_does_not_populate_session_id() {
        // Defensive: only Session/ClientSession variants should set the
        // session_id field on ModelUsage — every other group_by must leave
        // it None so the camelCase JSON output omits it via
        // `skip_serializing_if = "Option::is_none"`.
        for group_by in &[
            GroupBy::Model,
            GroupBy::ClientModel,
            GroupBy::ClientProviderModel,
            GroupBy::WorkspaceModel,
        ] {
            let entries = aggregate_model_usage_entries(
                vec![make_workspace_message(
                    "codex",
                    "gpt-5",
                    "openai",
                    "session-x",
                    1.0,
                    None,
                    None,
                )],
                group_by,
            );
            assert_eq!(entries.len(), 1);
            assert!(
                entries[0].session_id.is_none(),
                "session_id leaked into {:?} grouping",
                group_by
            );
        }
    }

    #[test]
    fn test_retain_for_requested_clients_keeps_original_client_matches() {
        let requested: HashSet<&str> = HashSet::from(["opencode"]);
        assert!(retain_for_requested_clients(
            "opencode",
            "gpt-4o",
            "anthropic",
            &requested
        ));
        assert!(!retain_for_requested_clients(
            "claude",
            "gpt-4o",
            "anthropic",
            &requested
        ));
    }

    #[test]
    fn test_retain_for_requested_clients_accepts_synthetic_gateway_traffic() {
        let requested: HashSet<&str> = HashSet::from(["synthetic"]);
        assert!(retain_for_requested_clients(
            "opencode",
            "hf:deepseek-ai/DeepSeek-V3-0324",
            "unknown",
            &requested
        ));
        assert!(retain_for_requested_clients(
            "synthetic",
            "deepseek-v3-0324",
            "synthetic",
            &requested
        ));
        assert!(!retain_for_requested_clients(
            "opencode",
            "gpt-4o",
            "anthropic",
            &requested
        ));
    }

    #[test]
    fn test_retain_for_requested_clients_preserves_kilo_split() {
        let kilocode_only: HashSet<&str> = HashSet::from(["kilocode"]);
        assert!(retain_for_requested_clients(
            "kilocode",
            "gpt-5",
            "openai",
            &kilocode_only
        ));
        assert!(!retain_for_requested_clients(
            "kilo",
            "gpt-5",
            "openai",
            &kilocode_only
        ));

        let kilo_only: HashSet<&str> = HashSet::from(["kilo"]);
        assert!(retain_for_requested_clients(
            "kilo", "gpt-5", "openai", &kilo_only
        ));
        assert!(!retain_for_requested_clients(
            "kilocode", "gpt-5", "openai", &kilo_only
        ));
    }

    #[test]
    #[serial_test::serial]
    fn test_cursor_parse_path_reprices_missing_cost_composer_1_5_rows() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let temp_dir = tempfile::TempDir::new().unwrap();
        let cursor_cache_dir = temp_dir.path().join(".config/tokscale/cursor-cache");
        std::fs::create_dir_all(&cursor_cache_dir).unwrap();

        let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2026-03-04T12:00:00.000Z","Included","Composer 1.5","No","1200","1000","5000","2000","8000","Included""#;
        std::fs::write(cursor_cache_dir.join("usage.csv"), csv).unwrap();

        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());
        let messages = parse_all_messages_with_pricing(
            temp_dir.path().to_str().unwrap(),
            &["cursor".to_string()],
            Some(&pricing),
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "cursor");
        assert_eq!(messages[0].model_id, "Composer 1.5");
        assert!(messages[0].cost > 0.0);
        assert!(!messages[0].has_authoritative_cost());
    }

    #[test]
    #[serial_test::serial]
    fn test_cursor_cached_lane_matches_cold_parse_on_warm_hit() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let cursor_cache_dir = source_home.path().join(".config/tokscale/cursor-cache");
        std::fs::create_dir_all(&cursor_cache_dir).unwrap();
        let usage_path = cursor_cache_dir.join("usage.csv");

        let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2026-03-04T12:00:00.000Z","Included","Composer 1.5","No","1200","1000","5000","2000","8000","0""#;
        std::fs::write(&usage_path, csv).unwrap();

        let mut litellm = HashMap::new();
        litellm.insert(
            "composer-1.5".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                cache_read_input_token_cost: Some(0.0001),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let _parse_counter =
            sessions::cursor::register_parse_cursor_file_counter(source_home.path());

        let cold = parse_all_messages_with_pricing(
            source_home.path().to_str().unwrap(),
            &["cursor".to_string()],
            Some(&pricing),
        );
        assert_eq!(cold.len(), 1);
        assert_eq!(cold[0].cost, 0.0);
        assert!(cold[0].has_authoritative_cost());
        assert_eq!(
            sessions::cursor::parse_cursor_file_call_count(source_home.path()),
            1,
            "cold parse should invoke the Cursor parser once"
        );

        let persisted = message_cache::SourceMessageCache::load();
        let cached = persisted
            .get(
                message_cache::CacheIdentity::for_client(ClientId::Cursor),
                &usage_path,
            )
            .expect("cold parse should persist the Cursor source entry");
        assert_eq!(cached.messages.len(), 1);

        let warm = parse_all_messages_with_pricing(
            source_home.path().to_str().unwrap(),
            &["cursor".to_string()],
            Some(&pricing),
        );
        assert_eq!(warm.len(), 1);
        assert_eq!(warm[0].cost, 0.0);
        assert!(warm[0].has_authoritative_cost());
        assert_eq!(warm, cold);
        assert_eq!(
            sessions::cursor::parse_cursor_file_call_count(source_home.path()),
            1,
            "warm cache hit must not invoke the Cursor parser again"
        );
    }

    /// MiMo Code records carry an authoritative per-message cost. The micode
    /// lane must NOT reprice a record that already has a cost, even when the
    /// model has a market price that would compute a different (non-zero) value.
    /// This must hold on the first parse AND on a subsequent cache hit, since
    /// the previous bug repriced and persisted the inflated cost to the cache.
    #[test]
    #[serial_test::serial]
    fn test_micode_authoritative_cost_is_not_repriced_on_first_parse_or_cache_hit() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let micode_dir = source_home.path().join(".local/share/mimocode");
            std::fs::create_dir_all(&micode_dir).unwrap();
            let db_path = micode_dir.join("mimocode.db");

            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE message (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    data TEXT NOT NULL
                );",
            )
            .unwrap();
            // Authoritative cost 0.05 with 1000 input / 500 output tokens.
            let data_json = r#"{
                "role": "assistant",
                "modelID": "mimo-v2.5-pro",
                "providerID": "mimo",
                "cost": 0.05,
                "tokens": { "input": 1000, "output": 500, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
                "time": { "created": 1700000000000.0 }
            }"#;
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["msg_auth_cost", "ses_1", data_json],
            )
            .unwrap();
            drop(conn);

            // Pricing that WOULD reprice mimo-v2.5-pro to a different non-zero
            // value (1000 * 0.001 + 500 * 0.002 = 2.0) if the guard were absent.
            let mut litellm = HashMap::new();
            litellm.insert(
                "mimo-v2.5-pro".into(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(0.001),
                    output_cost_per_token: Some(0.002),
                    ..Default::default()
                },
            );
            let pricing = pricing::PricingService::new(litellm, HashMap::new());

            let first = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["micode".to_string()],
                Some(&pricing),
            );
            assert_eq!(first.len(), 1);
            assert!(
                (first[0].cost - 0.05).abs() < 1e-9,
                "authoritative cost must survive the first parse, got {}",
                first[0].cost
            );

            // Second run hits the source cache; the persisted entry must still
            // carry the authoritative cost rather than a repriced value.
            let second = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["micode".to_string()],
                Some(&pricing),
            );
            assert_eq!(second.len(), 1);
            assert!(
                (second[0].cost - 0.05).abs() < 1e-9,
                "authoritative cost must survive the cache hit, got {}",
                second[0].cost
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_micode_cross_database_dedup_prefers_explicit_zero_cost() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let micode_dir = source_home.path().join(".local/share/mimocode");
            std::fs::create_dir_all(&micode_dir).unwrap();
            let without_cost = r#"{
                "id": "shared-message",
                "role": "assistant",
                "modelID": "unknown-model",
                "providerID": "mimo",
                "tokens": { "input": 10, "output": 5 },
                "time": { "created": 1700000000000.0 }
            }"#;
            let with_zero_cost = r#"{
                "id": "shared-message",
                "role": "assistant",
                "modelID": "unknown-model",
                "providerID": "mimo",
                "cost": 0,
                "tokens": { "input": 10, "output": 5 },
                "time": { "created": 1700000000000.0 }
            }"#;
            for (name, data) in [
                ("mimocode-alpha.db", without_cost),
                ("mimocode-beta.db", with_zero_cost),
            ] {
                let db_path = micode_dir.join(name);
                let conn = rusqlite::Connection::open(db_path).unwrap();
                conn.execute_batch(
                    "CREATE TABLE message (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL,
                        data TEXT NOT NULL
                    );",
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                    rusqlite::params![name, "session", data],
                )
                .unwrap();
            }

            let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());
            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["micode".to_string()],
                Some(&pricing),
            );

            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].cost, 0.0);
            assert!(messages[0].has_authoritative_cost());
            assert!(validate_priced_messages(&messages, Some(&pricing)).is_ok());
        }
    }

    /// Claude Code rewrites a session transcript in place on resume/compact:
    /// the file keeps its path and session id but loses already-written
    /// assistant turns. Because the source cache tracks live file content, a
    /// rescan after such a rewrite used to drop those turns from history for
    /// good — and `tokscale submit` feeds the leaderboard from the same
    /// recompute. See https://github.com/junhoyeo/tokscale/issues/994.
    #[test]
    #[serial_test::serial]
    fn test_claude_in_place_rewrite_preserves_previously_seen_messages() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let claude_dir = source_home.path().join(".claude/projects/myproject");
            std::fs::create_dir_all(&claude_dir).unwrap();
            let transcript = claude_dir.join("conversation.jsonl");

            let turn_one = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":7,"cache_creation_input_tokens":3}}}"#;
            let turn_two = r#"{"type":"assistant","timestamp":"2024-12-01T10:05:00.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":60,"cache_read_input_tokens":11,"cache_creation_input_tokens":5}}}"#;
            let turn_three = r#"{"type":"assistant","timestamp":"2024-12-01T10:10:00.000Z","requestId":"req_003","message":{"id":"msg_003","model":"claude-3-5-sonnet","usage":{"input_tokens":300,"output_tokens":70,"cache_read_input_tokens":13,"cache_creation_input_tokens":17}}}"#;

            std::fs::write(
                &transcript,
                format!("{turn_one}\n{turn_two}\n{turn_three}\n"),
            )
            .unwrap();

            let before = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(before.len(), 3, "cold scan must see all three turns");
            let before_output: i64 = before.iter().map(|m| m.tokens.output).sum();
            let before_cache_read: i64 = before.iter().map(|m| m.tokens.cache_read).sum();
            let before_cache_write: i64 = before.iter().map(|m| m.tokens.cache_write).sum();
            assert_eq!(before_output, 180);

            // The rewrite: same path, same session, two assistant turns gone.
            std::fs::write(&transcript, format!("{turn_three}\n")).unwrap();

            let after = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );

            assert_eq!(
                after.len(),
                3,
                "an in-place rewrite must not retire messages the cache already observed"
            );
            assert_eq!(
                after.iter().map(|m| m.tokens.output).sum::<i64>(),
                before_output
            );
            assert_eq!(
                after.iter().map(|m| m.tokens.cache_read).sum::<i64>(),
                before_cache_read
            );
            assert_eq!(
                after.iter().map(|m| m.tokens.cache_write).sum::<i64>(),
                before_cache_write
            );

            // Retention has to survive its own round trip through the cache:
            // the rewritten entry is what the NEXT scan reads back, so a
            // union that is computed but not persisted drifts one run later.
            let third = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(
                third.len(),
                3,
                "the retained turns must be written back to the cache, not just returned once"
            );
            assert_eq!(
                third.iter().map(|m| m.tokens.output).sum::<i64>(),
                before_output
            );
        }
    }

    #[test]
    fn test_claude_cross_file_merge_preserves_provider_reported_cost() {
        let mut retained = UnifiedMessage::new_with_dedup(
            "claude",
            "claude-3-5-haiku",
            "bedrock",
            "retained-session",
            1_733_050_000_000,
            TokenBreakdown {
                input: 500,
                output: 60,
                ..Default::default()
            },
            9.0,
            Some("msg_shared:req_shared".to_string()),
        );
        retained.mark_provider_reported_cost();
        let live = UnifiedMessage::new_with_dedup(
            "claude",
            "claude-3-5-sonnet",
            "anthropic",
            "live-session",
            1_733_050_001_000,
            TokenBreakdown {
                input: 200,
                output: 999,
                ..Default::default()
            },
            0.0,
            Some("msg_shared:req_shared".to_string()),
        );
        let mut retained_flag = true;

        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-3-5-sonnet".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        merge_claude_cross_file_duplicate(
            &mut retained,
            &mut retained_flag,
            live,
            false,
            Some(&pricing),
        );

        assert!(!retained_flag);
        assert_eq!(retained.model_id, "claude-3-5-sonnet");
        assert_eq!(retained.provider_id, "anthropic");
        assert_eq!(retained.session_id, "live-session");
        assert_eq!(retained.tokens.input, 500);
        assert_eq!(retained.tokens.output, 999);
        assert_eq!(retained.cost, 9.0);
        assert_eq!(retained.cost_source, sessions::CostSource::ProviderReported);
    }

    /// Drive a retained/live collision through cold parse, compaction,
    /// cross-file replay, and a warm cache hit.
    ///
    /// The retained copy is seeded into cache from a transcript that is then
    /// rewritten empty; the live copy arrives in a second transcript named by
    /// the caller, so file order cannot quietly become the tiebreaker. Both
    /// copies describe the same response, one observed mid-stream and one
    /// completed, and they lead on different token fields — so any resolution
    /// that keeps one whole message under-reports the other field.
    ///
    /// Every direction reconciles to the same row: input 500, output 999,
    /// attributed to the live observation's Sonnet metadata and repriced from
    /// the merged tokens. That is what makes the two directions comparable —
    /// which side happened to be retained must not change the result.
    fn assert_claude_retained_live_merge(
        retained_record: &str,
        live_record: &str,
        live_file_name: &str,
    ) {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        let claude_dir = client_scan_root(source_home.path(), ClientId::Claude).join("myproject");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let retained_path = claude_dir.join("mmm-retained.jsonl");

        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-3-5-sonnet".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        litellm.insert(
            "claude-3-5-haiku".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.0001),
                output_cost_per_token: Some(0.0002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        std::fs::write(&retained_path, format!("{retained_record}\n")).unwrap();
        let seeded = parse_all_messages_with_pricing_with_env_strategy(
            source_home.path().to_str().unwrap(),
            &["claude".to_string()],
            Some(&pricing),
            false,
            &scanner::ScannerSettings::default(),
        );
        assert_eq!(seeded.len(), 1, "cold scan must seed retained history");

        // Claude rewrites the first transcript, and the same response shows up
        // in a fork/resume transcript that is still on disk.
        std::fs::write(&retained_path, "").unwrap();
        std::fs::write(claude_dir.join(live_file_name), format!("{live_record}\n")).unwrap();

        let assert_merged = |messages: &[UnifiedMessage]| {
            assert_eq!(
                messages.len(),
                1,
                "the shared response must be counted once"
            );
            let merged = &messages[0];
            assert_eq!(merged.tokens.input, 500, "take the larger input");
            assert_eq!(merged.tokens.output, 999, "take the completed output");
            assert_eq!(merged.model_id, "claude-3-5-sonnet");
            assert_eq!(merged.provider_id, "anthropic");
            assert_eq!(merged.session_id, live_file_name.trim_end_matches(".jsonl"));
            assert_eq!(merged.workspace_key.as_deref(), Some("myproject"));
            assert_eq!(merged.workspace_label.as_deref(), Some("myproject"));
            assert_eq!(merged.cost_source, sessions::CostSource::Estimated);
            assert!(
                (merged.cost - 2.498).abs() < 1e-9,
                "Sonnet price must be recomputed from merged tokens; got {}",
                merged.cost
            );
        };

        let merged = parse_all_messages_with_pricing_with_env_strategy(
            source_home.path().to_str().unwrap(),
            &["claude".to_string()],
            Some(&pricing),
            false,
            &scanner::ScannerSettings::default(),
        );
        assert_merged(&merged);

        let warm = parse_all_messages_with_pricing_with_env_strategy(
            source_home.path().to_str().unwrap(),
            &["claude".to_string()],
            Some(&pricing),
            false,
            &scanner::ScannerSettings::default(),
        );
        assert_merged(&warm);
    }

    /// A partial observed before the transcript recorded the model the turn
    /// billed against. Used as the retained copy, its stale Haiku attribution
    /// must not outlive the live observation.
    const CLAUDE_MERGE_PARTIAL_STALE_MODEL: &str = r#"{"type":"assistant","timestamp":"2024-12-01T10:05:00.000Z","requestId":"req_shared","provider":"bedrock","message":{"id":"msg_shared","model":"claude-3-5-haiku","provider":"bedrock","usage":{"input_tokens":500,"output_tokens":60}}}"#;
    /// The same partial, already carrying the model the completed copy names.
    /// Used as the live copy, where model attribution is not what is under
    /// test and a divergence would only restate the pair above.
    const CLAUDE_MERGE_PARTIAL: &str = r#"{"type":"assistant","timestamp":"2024-12-01T10:05:00.000Z","requestId":"req_shared","provider":"anthropic","message":{"id":"msg_shared","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input_tokens":500,"output_tokens":60}}}"#;
    /// The completed observation of that same response.
    const CLAUDE_MERGE_COMPLETED: &str = r#"{"type":"assistant","timestamp":"2024-12-01T10:05:01.000Z","requestId":"req_shared","provider":"anthropic","message":{"id":"msg_shared","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input_tokens":200,"output_tokens":999}}}"#;

    #[test]
    #[serial_test::serial]
    fn test_claude_retained_partial_merges_completed_live_that_sorts_later() {
        assert_claude_retained_live_merge(
            CLAUDE_MERGE_PARTIAL_STALE_MODEL,
            CLAUDE_MERGE_COMPLETED,
            "zzz-live.jsonl",
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_claude_retained_partial_merges_completed_live_that_sorts_earlier() {
        assert_claude_retained_live_merge(
            CLAUDE_MERGE_PARTIAL_STALE_MODEL,
            CLAUDE_MERGE_COMPLETED,
            "aaa-live.jsonl",
        );
    }

    /// The mirror direction: the *retained* copy is the completed observation
    /// and the live transcript carries only the partial. Completeness has to
    /// be monotonic here too — a merge that keeps whichever copy is live, or
    /// whichever file sorts first, discards the completed output that only the
    /// retained copy still holds.
    #[test]
    #[serial_test::serial]
    fn test_claude_retained_completed_merges_live_partial_that_sorts_later() {
        assert_claude_retained_live_merge(
            CLAUDE_MERGE_COMPLETED,
            CLAUDE_MERGE_PARTIAL,
            "zzz-live.jsonl",
        );
    }

    /// Mirror direction, opposite file order.
    #[test]
    #[serial_test::serial]
    fn test_claude_retained_completed_merges_live_partial_that_sorts_earlier() {
        assert_claude_retained_live_merge(
            CLAUDE_MERGE_COMPLETED,
            CLAUDE_MERGE_PARTIAL,
            "aaa-live.jsonl",
        );
    }

    /// A cache entry written before retention provenance existed carries the
    /// retained turns but no record of *which* rows they are. Reading such an
    /// entry as if every row were live lets the stale, path-first copy of a
    /// response outrank the completed live replay of the same response — and
    /// the model attribution rides along with it, so the priced cost goes
    /// stale too. Every existing user upgrades with a populated cache, so the
    /// first warm scan after the upgrade has to rebuild that provenance.
    ///
    /// #1011: a pre-#1037 cache entry carries a `ceil(chars/4)` estimate that
    /// the API-reported `input_tokens` of the next turn already counted. The
    /// parser stopped minting those in #1037, but that alone does not clean an
    /// entry already on disk — the reporter measured an upgrade changing
    /// nothing, and asked for a migration.
    ///
    /// No migration is needed, and this pins why. Every such entry predates
    /// retention provenance (#1037 merged 2026-08-06, #1085 on 2026-08-11), so
    /// it is markerless, and `needs_retention_provenance_migration` already
    /// routes markerless Claude entries through a full re-parse. The estimate
    /// is re-derived away as a side effect of a mechanism built for something
    /// else.
    ///
    /// The fixture is arranged so a fix that merely *kept* the cached rows
    /// would fail: the seeded estimate is a row the current parser cannot
    /// produce, so if the warm scan still reports it, the entry was served
    /// rather than rebuilt.
    #[test]
    #[serial_test::serial]
    fn test_claude_legacy_char_estimate_is_dropped_by_the_provenance_rebuild() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _env = redirect_cache_home(cache_home.path());

        let claude_dir = client_scan_root(source_home.path(), ClientId::Claude).join("myproject");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let transcript = claude_dir.join("estimate-session.jsonl");

        // One assistant turn, and a tool_result with content but no token
        // metadata — the shape Claude Code always writes, which is what made
        // the old fallback fire on every one of them.
        let assistant = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let tool_result = r#"{"type":"user","timestamp":"2024-12-01T10:00:05.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_001","content":"0123456789012345678901234567890123456789"}]}}"#;
        std::fs::write(&transcript, format!("{assistant}\n{tool_result}\n")).unwrap();

        let scan = || {
            parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            )
        };
        let total_input =
            |messages: &[UnifiedMessage]| messages.iter().map(|m| m.tokens.input).sum::<i64>();

        let clean = scan();
        assert_eq!(
            total_input(&clean),
            100,
            "current parser must report only the API number"
        );

        // Rewrite the entry the way a pre-#1037 release left it: the estimate
        // as its own path-scoped row, and no provenance marker.
        // Exactly what `estimate_tokens_from_chars` would have produced for the
        // 40-character tool_result above.
        let estimate = 40_usize.div_ceil(4) as i64;
        {
            let mut cache = message_cache::SourceMessageCache::load();
            let mut entries: Vec<message_cache::CachedSourceEntry> = cache
                .all_entries()
                .into_iter()
                .filter(message_cache::CachedSourceEntry::is_claude_namespace)
                .collect();
            assert_eq!(entries.len(), 1, "the transcript must be cached");
            let entry = &mut entries[0];
            entry.messages.push(UnifiedMessage::new_with_dedup(
                "claude",
                "claude-3-5-sonnet",
                "anthropic",
                "estimate-session",
                1_733_047_205_000,
                TokenBreakdown {
                    input: estimate,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                Some("claude:tool_result:estimate-session:tool_result:toolu_001".to_string()),
            ));
            entry.fallback_timestamp_indices.clear();
            cache.insert(entries.pop().unwrap());
            cache.save_if_dirty();
        }

        let poisoned = message_cache::SourceMessageCache::load()
            .all_entries()
            .into_iter()
            .filter(message_cache::CachedSourceEntry::is_claude_namespace)
            .flat_map(|entry| entry.messages)
            .map(|message| message.tokens.input)
            .sum::<i64>();
        assert_eq!(
            poisoned,
            100 + estimate,
            "the seeded cache must actually be inflated, or this test proves nothing"
        );

        let warm = scan();
        assert_eq!(
            total_input(&warm),
            100,
            "a markerless entry must be rebuilt, dropping the stale char estimate"
        );
        assert!(
            !warm.iter().any(|m| m
                .dedup_key
                .as_deref()
                .is_some_and(|k| k.contains(":tool_result:"))),
            "the phantom tool_result row must not survive the rebuild"
        );
    }

    /// The strongest statement of "not stale" is that the warm scan agrees
    /// with a cold scan of the same bytes, cost included.
    #[test]
    #[serial_test::serial]
    fn test_claude_legacy_cache_entry_rebuilds_retention_provenance() {
        use crate::RETENTION_PROVENANCE_REBUILDS;
        use std::sync::atomic::Ordering::Relaxed;

        let cache_home = tempfile::TempDir::new().unwrap();
        let cold_cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let mut env = redirect_cache_home(cache_home.path());

        {
            let claude_dir =
                client_scan_root(source_home.path(), ClientId::Claude).join("myproject");
            std::fs::create_dir_all(&claude_dir).unwrap();
            let original = claude_dir.join("aaa-original.jsonl");

            let turn_one = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
            // The partial was observed mid-stream, before the transcript
            // recorded the model the turn actually billed against.
            let turn_two_partial = r#"{"type":"assistant","timestamp":"2024-12-01T10:05:00.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-haiku","usage":{"input_tokens":200,"output_tokens":60}}}"#;
            let turn_two_complete = r#"{"type":"assistant","timestamp":"2024-12-01T10:05:00.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":2000,"output_tokens":999}}}"#;

            let mut litellm = HashMap::new();
            litellm.insert(
                "claude-3-5-sonnet".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(0.000_003),
                    output_cost_per_token: Some(0.000_015),
                    ..Default::default()
                },
            );
            litellm.insert(
                "claude-3-5-haiku".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(0.000_000_8),
                    output_cost_per_token: Some(0.000_004),
                    ..Default::default()
                },
            );
            let pricing = pricing::PricingService::new(litellm, HashMap::new());

            let scan = |pricing: &pricing::PricingService| {
                let mut messages = parse_all_messages_with_pricing_with_env_strategy(
                    source_home.path().to_str().unwrap(),
                    &["claude".to_string()],
                    Some(pricing),
                    false,
                    &scanner::ScannerSettings::default(),
                );
                messages.sort_by(|left, right| left.dedup_key.cmp(&right.dedup_key));
                messages
            };
            let summary = |messages: &[UnifiedMessage]| {
                messages
                    .iter()
                    .map(|message| {
                        (
                            message.dedup_key.clone(),
                            message.model_id.clone(),
                            message.tokens.input,
                            message.tokens.output,
                            format!("{:.10}", message.cost),
                        )
                    })
                    .collect::<Vec<_>>()
            };

            std::fs::write(&original, format!("{turn_one}\n{turn_two_partial}\n")).unwrap();
            assert_eq!(scan(&pricing).len(), 2, "seed scan");

            // The session forks: the original transcript keeps only turn one,
            // and the fork replays turn two with the completed response.
            std::fs::write(&original, format!("{turn_one}\n")).unwrap();
            std::fs::write(
                claude_dir.join("zzz-fork.jsonl"),
                format!("{turn_two_complete}\n"),
            )
            .unwrap();
            assert_eq!(scan(&pricing).len(), 2, "fork scan");

            // Rewrite every Claude entry in the pre-provenance shape a release
            // before this one would have left on disk.
            let mut cache = message_cache::SourceMessageCache::load();
            let legacy: Vec<message_cache::CachedSourceEntry> = cache
                .all_entries()
                .into_iter()
                .filter(message_cache::CachedSourceEntry::is_claude_namespace)
                .collect();
            assert_eq!(legacy.len(), 2, "both transcripts must be cached");
            assert!(
                legacy
                    .iter()
                    .any(|entry| !entry.fallback_timestamp_indices.is_empty()),
                "the retaining entry must have recorded provenance before it is stripped"
            );
            for mut entry in legacy {
                entry.fallback_timestamp_indices.clear();
                cache.insert(entry);
            }
            cache.save_if_dirty();
            drop(cache);

            let rebuilds_before = RETENTION_PROVENANCE_REBUILDS.load(Relaxed);
            let warm = scan(&pricing);
            let rebuilds_after_first = RETENTION_PROVENANCE_REBUILDS.load(Relaxed);

            point_cache_home(&mut env, cold_cache_home.path());
            let cold = scan(&pricing);
            point_cache_home(&mut env, cache_home.path());

            assert_eq!(cold.len(), 2, "cold scan sees turn one and the replay");
            assert_eq!(
                summary(&warm),
                summary(&cold),
                "the first warm scan over a pre-provenance cache must agree with a cold parse"
            );
            let warm_two = warm
                .iter()
                .find(|message| message.dedup_key.as_deref() == Some("msg_002:req_002"))
                .expect("the replayed turn must survive");
            assert_eq!(warm_two.model_id, "claude-3-5-sonnet");
            assert!(warm_two.cost > 0.0, "the replayed turn must be priced");

            // The rebuild is an upgrade cost, not a per-scan one. Both entries
            // are rebuilt on the first warm scan and none on the second.
            assert_eq!(
                rebuilds_after_first - rebuilds_before,
                2,
                "both pre-provenance entries are rebuilt on the first warm scan"
            );
            let warm_again = scan(&pricing);
            assert_eq!(
                RETENTION_PROVENANCE_REBUILDS.load(Relaxed),
                rebuilds_after_first,
                "a second warm scan must not re-parse the transcripts again"
            );
            assert_eq!(
                summary(&warm_again),
                summary(&cold),
                "and it must keep reporting the rebuilt result"
            );

            let migrated = message_cache::SourceMessageCache::load();
            let claude_entries: Vec<message_cache::CachedSourceEntry> = migrated
                .all_entries()
                .into_iter()
                .filter(message_cache::CachedSourceEntry::is_claude_namespace)
                .collect();
            assert_eq!(claude_entries.len(), 2);
            assert!(
                claude_entries
                    .iter()
                    .all(|entry| !entry.needs_retention_provenance_migration()),
                "the rebuild has to be persisted, or every scan pays for it again"
            );
            let retained: HashSet<String> = claude_entries
                .iter()
                .flat_map(|entry| entry.retained_message_keys())
                .collect();
            assert_eq!(
                retained,
                HashSet::from(["msg_002:req_002".to_string()]),
                "only the turn the original transcript no longer carries is retained"
            );
        }
    }

    /// A Claude tool-result key embeds the session id, which is the
    /// transcript's file stem. A retained tool result therefore could never
    /// collapse against the same tool result replayed under a fork's filename
    /// — both would count — so retention has to leave those records behind
    /// even though it means a compaction still retires their input tokens.
    #[test]
    #[serial_test::serial]
    fn test_claude_path_scoped_tool_result_is_not_retained_across_a_rewrite() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let claude_dir = source_home.path().join(".claude/projects/myproject");
            std::fs::create_dir_all(&claude_dir).unwrap();
            let transcript = claude_dir.join("conversation.jsonl");

            let assistant_turn = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
            let tool_result = r#"{"type":"user","timestamp":"2024-12-01T10:01:00.000Z","message":{"model":"claude-3-5-sonnet","content":[{"type":"tool_result","tool_use_id":"toolu_1","tool_output":{"input_tokens":40,"output":"result"}}]}}"#;

            std::fs::write(&transcript, format!("{assistant_turn}\n{tool_result}\n")).unwrap();
            let before = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(
                before.len(),
                2,
                "cold scan sees the turn and the tool result"
            );
            assert_eq!(before.iter().map(|m| m.tokens.input).sum::<i64>(), 140);

            // The rewrite drops both records; only the assistant turn is
            // re-added, so the tool result is a candidate for retention.
            std::fs::write(&transcript, format!("{assistant_turn}\n")).unwrap();

            let after = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(
                after.len(),
                1,
                "a path-scoped key must not be carried across the rewrite"
            );
            assert_eq!(after.iter().map(|m| m.tokens.input).sum::<i64>(), 100);
        }
    }

    /// The retention above must not resurrect a session the user deleted:
    /// `prune_missing_files` drops the entry when the file is gone, which is
    /// the behavior `d9df8c9c` (local session cleanup) depends on.
    ///
    /// The transcript is compacted first, and retention is asserted, so the
    /// deletion runs against an entry that really is holding a turn the live
    /// file no longer has. Deleting straight after a cold scan would prove
    /// nothing about retention.
    #[test]
    #[serial_test::serial]
    fn test_claude_deleted_transcript_is_not_resurrected_by_retention() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let claude_dir = source_home.path().join(".claude/projects/myproject");
            std::fs::create_dir_all(&claude_dir).unwrap();
            let transcript = claude_dir.join("conversation.jsonl");

            let turn_one = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
            let turn_two = r#"{"type":"assistant","timestamp":"2024-12-01T10:05:00.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":60}}}"#;

            std::fs::write(&transcript, format!("{turn_one}\n{turn_two}\n")).unwrap();
            let before = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(before.len(), 2);

            std::fs::write(&transcript, format!("{turn_two}\n")).unwrap();
            let retained = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(
                retained.len(),
                2,
                "the entry must actually be holding a retained turn before the delete"
            );

            std::fs::remove_file(&transcript).unwrap();

            let after = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
                false,
                &scanner::ScannerSettings::default(),
            );
            assert!(
                after.is_empty(),
                "a deleted transcript stays deleted, retained turns and all; local disk remains the source of truth"
            );
        }
    }

    fn write_kimi_repeated_status_fixture(source_home: &std::path::Path) {
        let session_dir = source_home.join(".kimi/sessions/group-1/session-1");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("wire.jsonl"),
            r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 10, "output": 1, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-progressive"}}}
{"timestamp": 1770983420.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 20, "output": 2, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-progressive"}}}
{"timestamp": 1770983430.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 5, "output": 1, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-distinct"}}}
{"timestamp": 1770983440.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 7, "output": 1, "input_cache_read": 0, "input_cache_creation": 0}}}}
{"timestamp": 1770983450.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 8, "output": 1, "input_cache_read": 0, "input_cache_creation": 0}}}}"#,
        )
        .unwrap();
    }

    fn write_kimchi_fixture(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            r#"{"type":"session","id":"kimchi-session","timestamp":"2026-08-01T00:00:00.000Z","cwd":"/tmp/kimchi-project"}
{"type":"message","id":"kimchi-message","timestamp":"2026-08-01T00:00:01.000Z","message":{"role":"assistant","model":"kimi-k2.6","provider":"kimchi-dev","usage":{"input":100,"output":10,"cacheRead":5,"cacheWrite":2,"totalTokens":117}}}"#,
        )
        .unwrap();
    }
    fn write_cline_cli_fixture(path: &std::path::Path, messages: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(r#"{{"sessionId":"cline-dedup-session","messages":[{messages}]}}"#),
        )
        .unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_cline_cli_deduplicates_duplicate_records_in_cached_and_local_paths() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let duplicate = r#"{"id":"duplicate","role":"assistant","ts":1785320475705,"modelInfo":{"id":"cline-free/glm-5.2","provider":"cline-pass"},"metrics":{"inputTokens":100,"outputTokens":10}}"#;
            let distinct_a = r#"{"id":"distinct-a","role":"assistant","ts":1785320476705,"metrics":{"inputTokens":200,"outputTokens":20}}"#;
            let distinct_b = r#"{"id":"distinct-b","role":"assistant","ts":1785320477705,"metrics":{"inputTokens":300,"outputTokens":30}}"#;
            write_cline_cli_fixture(
                &source_home
                    .path()
                    .join(".cline/data/sessions/first/first.messages.json"),
                &format!("{duplicate},{distinct_a}"),
            );
            write_cline_cli_fixture(
                &source_home
                    .path()
                    .join(".cline/data/sessions/second/second.messages.json"),
                &format!("{duplicate},{distinct_b}"),
            );

            let clients = ["cline".to_string()];
            let scanner_settings = scanner::ScannerSettings::default();
            let cached = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &clients,
                None,
                false,
                &scanner_settings,
            );
            let mut cached_inputs = cached
                .iter()
                .map(|message| message.tokens.input)
                .collect::<Vec<_>>();
            cached_inputs.sort_unstable();
            assert_eq!(cached_inputs, vec![100, 200, 300]);
            let cached_again = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &clients,
                None,
                false,
                &scanner_settings,
            );
            let mut cached_again_inputs = cached_again
                .iter()
                .map(|message| message.tokens.input)
                .collect::<Vec<_>>();
            cached_again_inputs.sort_unstable();
            assert_eq!(cached_again_inputs, vec![100, 200, 300]);

            let parsed = parse_local_clients(LocalParseOptions {
                home_dir: Some(source_home.path().to_str().unwrap().to_string()),
                use_env_roots: false,
                clients: Some(clients.to_vec()),
                since: None,
                until: None,
                year: None,
                scanner_settings,
            })
            .unwrap();
            let mut local_inputs = parsed
                .messages
                .iter()
                .map(|message| message.input)
                .collect::<Vec<_>>();
            local_inputs.sort_unstable();
            assert_eq!(local_inputs, vec![100, 200, 300]);
            assert_eq!(parsed.counts.get(ClientId::Cline), 3);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_prefers_grok_unified_log() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = source_home
                .path()
                .join(".grok/sessions/%2Ftmp%2Fproject/session-1");
            std::fs::create_dir_all(&session_dir).unwrap();
            std::fs::write(
                session_dir.join("updates.jsonl"),
                r#"{"method":"session/update","params":{"sessionId":"session-1","_meta":{"totalTokens":999,"agentTimestampMs":1700000000000}}}"#,
            )
            .unwrap();

            let logs_dir = source_home.path().join(".grok/logs");
            std::fs::create_dir_all(&logs_dir).unwrap();
            std::fs::write(
                logs_dir.join("unified.jsonl"),
                r#"{"ts":"2023-11-14T22:13:20Z","pid":7,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"cached_prompt_tokens":60,"completion_tokens":25,"reasoning_tokens":5}}"#,
            )
            .unwrap();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["grok".to_string()],
                None,
            );

            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].tokens.input, 40);
            assert_eq!(messages[0].tokens.cache_read, 60);
            assert_eq!(messages[0].tokens.output, 20);
            assert_eq!(messages[0].tokens.reasoning, 5);
            assert_eq!(messages[0].tokens.total(), 125);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_reprices_grok_after_legacy_model_attribution() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = source_home
                .path()
                .join(".grok/sessions/%2Ftmp%2Fproject/session-1");
            std::fs::create_dir_all(&session_dir).unwrap();
            std::fs::write(
                session_dir.join("updates.jsonl"),
                r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk","_meta":{"modelId":"grok-code"}},"_meta":{"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":999,"agentTimestampMs":1700000000000}}}"#,
            )
            .unwrap();

            let logs_dir = source_home.path().join(".grok/logs");
            std::fs::create_dir_all(&logs_dir).unwrap();
            std::fs::write(
                logs_dir.join("unified.jsonl"),
                r#"{"ts":"2023-11-14T22:13:20Z","pid":7,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"cached_prompt_tokens":60,"completion_tokens":25,"reasoning_tokens":5}}"#,
            )
            .unwrap();

            let mut litellm = HashMap::new();
            litellm.insert(
                "grok-code".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(0.001),
                    output_cost_per_token: Some(0.002),
                    ..Default::default()
                },
            );
            let pricing = pricing::PricingService::new(litellm, HashMap::new());

            let first = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["grok".to_string()],
                Some(&pricing),
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].model_id, "grok-code");
            assert!(first[0].cost > 0.0);

            let second = parse_all_messages_with_pricing_with_env_strategy(
                source_home.path().to_str().unwrap(),
                &["grok".to_string()],
                Some(&pricing),
                false,
                &scanner::ScannerSettings::default(),
            );
            assert_eq!(second.len(), 1);
            assert_eq!(second[0].model_id, "grok-code");
            assert!(second[0].cost > 0.0);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_keeps_conflicted_grok_scoped_model_change_unpriced_cold_and_warm() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let logs_dir = source_home.path().join(".grok/logs");
            std::fs::create_dir_all(&logs_dir).unwrap();
            std::fs::write(
                logs_dir.join("unified.jsonl"),
                r#"{"ts":"2026-07-31T00:00:01Z","pid":19,"msg":"subagent spawn credentials","ctx":{"subagent_id":"child","effective_model":"grok-4.8"}}
{"ts":"2026-07-31T00:00:02Z","pid":19,"sid":"child","msg":"model changed","ctx":{"model":"grok-code"}}
{"ts":"2026-07-31T00:00:03Z","pid":19,"msg":"subagent failed","ctx":{"subagent_id":"child","effective_model":"grok-4.9"}}
{"ts":"2026-07-31T00:00:04Z","pid":19,"sid":"child","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"cached_prompt_tokens":60,"completion_tokens":25,"reasoning_tokens":5}}"#,
            )
            .unwrap();

            let mut litellm = HashMap::new();
            litellm.insert(
                "grok-code".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(0.001),
                    output_cost_per_token: Some(0.002),
                    ..Default::default()
                },
            );
            let pricing = pricing::PricingService::new(litellm, HashMap::new());

            for scan in ["cold", "warm"] {
                let messages = parse_all_messages_with_pricing_with_env_strategy(
                    source_home.path().to_str().unwrap(),
                    &["grok".to_string()],
                    Some(&pricing),
                    false,
                    &scanner::ScannerSettings::default(),
                );
                assert_eq!(messages.len(), 1, "{scan} scan message count");
                assert_eq!(messages[0].model_id, "grok-unknown", "{scan} scan");
                assert!(messages[0].model_attribution_conflicted, "{scan} scan");
                assert_eq!(messages[0].cost, 0.0, "{scan} scan");
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_kimi_deduplicates_repeated_status_updates() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_kimi_repeated_status_fixture(source_home.path());

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["kimi".to_string()],
                None,
            );

            assert_eq!(messages.len(), 4);
            assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 40);
            assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 5);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_local_clients_kimi_deduplicates_repeated_status_updates() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_kimi_repeated_status_fixture(source_home.path());

            let parsed = parse_local_clients(LocalParseOptions {
                home_dir: Some(source_home.path().to_str().unwrap().to_string()),
                use_env_roots: false,
                clients: Some(vec!["kimi".to_string()]),
                since: None,
                until: None,
                year: None,
                scanner_settings: scanner::ScannerSettings::default(),
            })
            .unwrap();

            assert_eq!(parsed.counts.get(ClientId::Kimi), 4);
            assert_eq!(parsed.messages.len(), 4);
            assert_eq!(parsed.messages.iter().map(|m| m.input).sum::<i64>(), 40);
            assert_eq!(parsed.messages.iter().map(|m| m.output).sum::<i64>(), 5);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_kimchi_deduplicates_same_message_across_scan_roots() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        let default_path = source_home
            .path()
            .join(".config/kimchi/harness/sessions/workspace/session.jsonl");
        let extra_path = source_home
            .path()
            .join("kimchi-extra/workspace/session.jsonl");
        write_kimchi_fixture(&default_path);
        write_kimchi_fixture(&extra_path);

        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert(
            "kimchi".to_string(),
            vec![source_home.path().join("kimchi-extra")],
        );
        let scanner_settings = scanner::ScannerSettings {
            extra_scan_paths,
            ..Default::default()
        };

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(source_home.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["kimchi".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner_settings.clone(),
        })
        .unwrap();
        assert_eq!(parsed.counts.get(ClientId::Kimchi), 1);
        assert_eq!(parsed.messages.len(), 1);

        let messages = parse_all_messages_with_pricing_with_env_strategy(
            source_home.path().to_str().unwrap(),
            &["kimchi".to_string()],
            None,
            false,
            &scanner_settings,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("kimchi:kimchi-session:kimchi-message")
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_local_clients_codebuff_freebuff_filters_stay_isolated() {
        // Freebuff and Codebuff share the manicode scan bucket (parser
        // partition the same file set). A single-client filter must not pick
        // up the other product's rows: codebuff-only must produce clean code
        // rows/zero freebuff count, and vice versa.
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        let manicode = source_home.path().join(".config").join("manicode");
        // An authoritative Codebuff chat: assistant message carries usage.
        let codebuff_chat = manicode
            .join("projects")
            .join("proj")
            .join("chats")
            .join("2026-08-07T05-21-00.000Z");
        std::fs::create_dir_all(&codebuff_chat).unwrap();
        std::fs::write(
            codebuff_chat.join("chat-messages.json"),
            r#"[
                { "variant": "user", "content": "hi", "timestamp": "2026-08-07T05:21:00.000Z" },
                { "variant": "ai", "timestamp": "2026-08-07T05:22:00.000Z",
                  "metadata": { "model": "claude-sonnet-4-20250514",
                                "usage": { "inputTokens": 500, "outputTokens": 200 } } }
            ]"#,
        )
        .unwrap();
        // A Freebuff chat: marked by its `base2-free*` root agent id, with no
        // authoritative usage — only estimated text.
        let freebuff_chat = manicode
            .join("projects")
            .join("proj")
            .join("chats")
            .join("2026-08-07T13-00-00.000Z");
        std::fs::create_dir_all(&freebuff_chat).unwrap();
        std::fs::write(
            freebuff_chat.join("chat-messages.json"),
            r#"[
                { "variant": "user", "content": "hello world", "timestamp": "2026-08-07T13:00:00.000Z" },
                { "variant": "ai", "timestamp": "2026-08-07T13:01:00.000Z", "blocks": [ { "content": "Hello!" } ],
                  "metadata": { "runState": { "sessionState": { "mainAgentState": {
                      "agentType": "base2-free-deepseek-flash" } } } } }
            ]"#,
        )
        .unwrap();

        let options_for = |clients: Vec<String>| LocalParseOptions {
            home_dir: Some(source_home.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(clients),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        };

        // codebuff-only: authoritative Codebuff row, zero estimated Freebuff rows.
        let codebuff_only = parse_local_clients(options_for(vec!["codebuff".to_string()])).unwrap();
        assert_eq!(codebuff_only.counts.get(ClientId::Codebuff), 1);
        assert_eq!(codebuff_only.counts.get(ClientId::Freebuff), 0);
        assert!(
            codebuff_only
                .messages
                .iter()
                .all(|m| m.client == "codebuff"),
            "all reported rows must be codebuff, got {:?}",
            codebuff_only
                .messages
                .iter()
                .map(|m| &m.client)
                .collect::<Vec<_>>()
        );

        // freebuff-only → estimated Freebuff rows, zero Codebuff rows.
        let free_only = parse_local_clients(options_for(vec!["freebuff".to_string()])).unwrap();
        assert_eq!(free_only.counts.get(ClientId::Freebuff), 1);
        assert_eq!(free_only.counts.get(ClientId::Codebuff), 0);
        assert!(
            free_only.messages.iter().all(|m| m.client == "freebuff"),
            "all reported rows must be freebuff, got {:?}",
            free_only
                .messages
                .iter()
                .map(|m| &m.client)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_includes_opencodereview() {
        // Regression: opencodereview declares submit_default, and
        // parse_local_clients has always parsed it, so `tokscale report`
        // showed the usage. But the submit path
        // (parse_all_messages_with_pricing_with_env_strategy) had no
        // opencodereview block at all, so none of that usage was ever
        // uploaded. Pin the submit path specifically — a green
        // parse_local_clients test cannot catch this.
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = source_home
                .path()
                .join(".opencodereview/sessions/-home-user-project");
            std::fs::create_dir_all(&session_dir).unwrap();
            std::fs::write(
                session_dir.join("session-1.jsonl"),
                r#"{"type":"session_start","sessionId":"session-1","timestamp":"2026-01-15T10:00:00Z","cwd":"/home/user/project","model":"claude-sonnet-4-20250514"}
{"type":"llm_response","sessionId":"session-1","timestamp":"2026-01-15T10:00:05Z","model":"claude-sonnet-4-20250514","duration_ms":1500,"usage":{"prompt_tokens":1000,"completion_tokens":200,"cache_read_tokens":500,"cache_write_tokens":100}}
{"type":"llm_response","sessionId":"session-1","timestamp":"2026-01-15T10:01:00Z","model":"gpt-4o","duration_ms":900,"usage":{"prompt_tokens":300,"completion_tokens":50,"cache_read_tokens":0,"cache_write_tokens":0}}"#,
            )
            .unwrap();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencodereview".to_string()],
                None,
            );

            assert_eq!(messages.len(), 2);
            assert!(messages.iter().all(|m| m.client == "opencodereview"));
            assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1300);
            assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 250);
            assert_eq!(
                messages.iter().map(|m| m.tokens.cache_read).sum::<i64>(),
                500
            );
            assert_eq!(
                messages.iter().map(|m| m.tokens.cache_write).sum::<i64>(),
                100
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_source_cache_refreshes_stale_date_on_cache_hit() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let message_dir = source_home
                .path()
                .join(".local/share/opencode/storage/message/project-1");
            std::fs::create_dir_all(&message_dir).unwrap();
            let path = message_dir.join("msg_001.json");
            std::fs::write(
                &path,
                r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"accounts/fireworks/models/deepseek-v3-0324","providerID":"fireworks","cost":0,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
            )
            .unwrap();

            let fingerprint = message_cache::SourceFingerprint::from_path(&path).unwrap();
            let mut stale_message = UnifiedMessage::new(
                "opencode",
                "accounts/fireworks/models/deepseek-v3-0324",
                "fireworks",
                "session-1",
                1_733_011_200_000,
                TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
            );
            stale_message.date = "1900-01-01".to_string();

            let mut cache = message_cache::SourceMessageCache::default();
            cache.insert(message_cache::CachedSourceEntry::new(
                message_cache::CacheIdentity::for_client(ClientId::OpenCode),
                &path,
                fingerprint,
                vec![stale_message],
                Vec::new(),
                None,
            ));
            cache.save_if_dirty();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );

            assert_eq!(messages.len(), 1);
            assert_ne!(messages[0].date, "1900-01-01");
            assert_eq!(
                messages[0].date,
                UnifiedMessage::new(
                    "opencode",
                    "accounts/fireworks/models/deepseek-v3-0324",
                    "fireworks",
                    "session-1",
                    1_733_011_200_000,
                    TokenBreakdown {
                        input: 10,
                        output: 5,
                        cache_read: 0,
                        cache_write: 0,
                        reasoning: 0,
                    },
                    0.0,
                )
                .date
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_claude_warm_cache_removes_synthetic_placeholder_before_submit_validation() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let claude_dir = client_scan_root(source_home.path(), ClientId::Claude).join("demo");
            std::fs::create_dir_all(&claude_dir).unwrap();
            let transcript = claude_dir.join("session.jsonl");
            std::fs::write(
                &transcript,
                r#"{"type":"assistant","timestamp":"2026-06-24T01:00:00.000Z","requestId":"req_live","message":{"id":"live","model":"claude-3-5-sonnet","usage":{"input_tokens":1,"output_tokens":1}}}"#,
            )
            .unwrap();

            let identity = message_cache::CacheIdentity::for_client(ClientId::Claude);
            let fingerprint = message_cache::SourceFingerprint::from_claude_code_path_with_home(
                &transcript,
                Some(source_home.path()),
            )
            .unwrap();
            let retained = UnifiedMessage::new_with_dedup(
                "claude",
                "claude-3-5-sonnet",
                "anthropic",
                "session",
                1_782_259_200_000,
                TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                Some("old:req_old".to_string()),
            );
            let poisoned = UnifiedMessage::new_with_dedup(
                "claude",
                "<synthetic>",
                "unknown",
                "session",
                1_782_259_201_000,
                TokenBreakdown {
                    input: 100,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                Some("claude:tool_result:session:tool_result:toolu_1".to_string()),
            );
            let mut cache = message_cache::SourceMessageCache::default();
            // Seeded the way a scan writes it: `old:req_old` is history the
            // live transcript no longer carries, and the entry records that.
            // An entry without the provenance is a pre-upgrade one and gets
            // rebuilt from the live bytes instead of served warm, which is a
            // different path than this test is about.
            cache.insert(
                message_cache::CachedSourceEntry::new_with_retained_message_keys(
                    identity,
                    &transcript,
                    fingerprint,
                    vec![retained, poisoned],
                    &HashSet::from(["old:req_old".to_string()]),
                ),
            );
            cache.save_if_dirty();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
            );

            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
            assert_eq!(messages[0].tokens.input, 10);

            let repaired = message_cache::SourceMessageCache::load();
            let cached = repaired
                .get(identity, &transcript)
                .expect("the retained Claude cache entry should remain");
            assert_eq!(cached.messages.len(), 1);
            assert_eq!(cached.messages[0].dedup_key.as_deref(), Some("old:req_old"));
        }
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_empty_parse_results_are_not_cached_for_optional_file_sources() {
        use std::os::unix::fs::PermissionsExt;

        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let message_dir = source_home
                .path()
                .join(".local/share/opencode/storage/message/project-1");
            std::fs::create_dir_all(&message_dir).unwrap();
            let path = message_dir.join("msg_001.json");
            std::fs::write(
                &path,
                r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"accounts/fireworks/models/deepseek-v3-0324","providerID":"fireworks","cost":0,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
            )
            .unwrap();

            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o000);
            std::fs::set_permissions(&path, permissions).unwrap();

            let first_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );
            assert!(first_messages.is_empty());

            let cache = message_cache::SourceMessageCache::load();
            assert!(cache
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::OpenCode),
                    &path,
                )
                .is_none());

            let mut readable_permissions = std::fs::metadata(&path).unwrap().permissions();
            readable_permissions.set_mode(0o644);
            std::fs::set_permissions(&path, readable_permissions).unwrap();

            let second_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );
            assert_eq!(second_messages.len(), 1);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_empty_cache_hits_are_reparsed_for_optional_file_sources() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let message_dir =
                client_scan_root(source_home.path(), ClientId::OpenCode).join("project-1");
            std::fs::create_dir_all(&message_dir).unwrap();
            let path = message_dir.join("msg_001.json");
            std::fs::write(
                &path,
                r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"accounts/fireworks/models/deepseek-v3-0324","providerID":"fireworks","cost":0,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
            )
            .unwrap();

            let fingerprint = message_cache::SourceFingerprint::from_path(&path).unwrap();
            let mut cache = message_cache::SourceMessageCache::default();
            cache.insert(message_cache::CachedSourceEntry::new(
                message_cache::CacheIdentity::for_client(ClientId::OpenCode),
                &path,
                fingerprint,
                Vec::new(),
                Vec::new(),
                None,
            ));
            cache.save_if_dirty();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );
            assert_eq!(messages.len(), 1);

            let loaded = message_cache::SourceMessageCache::load();
            let repaired_entry = loaded
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::OpenCode),
                    &path,
                )
                .unwrap();
            assert_eq!(repaired_entry.messages.len(), 1);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_sqlite_source_cache_invalidates_on_wal_change() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let db_dir = source_home.path().join(".local/share/opencode");
            std::fs::create_dir_all(&db_dir).unwrap();
            let db_path = db_dir.join("opencode.db");

            let conn = rusqlite::Connection::open(&db_path).unwrap();
            let journal_mode: String = conn
                .query_row("PRAGMA journal_mode=WAL;", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal_mode.to_lowercase(), "wal");
            conn.execute_batch(
                "PRAGMA wal_autocheckpoint=0;
                 CREATE TABLE message (
                     id TEXT PRIMARY KEY,
                     session_id TEXT NOT NULL,
                     data TEXT NOT NULL
                 );",
            )
            .unwrap();

            let row_one = r#"{
                "role": "assistant",
                "modelID": "claude-sonnet-4",
                "providerID": "anthropic",
                "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
                "time": { "created": 1700000000000.0 }
            }"#;
            let row_two = r#"{
                "role": "assistant",
                "modelID": "claude-sonnet-4",
                "providerID": "anthropic",
                "tokens": { "input": 120, "output": 60, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
                "time": { "created": 1700000001000.0 }
            }"#;

            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["msg-1", "session-1", row_one],
            )
            .unwrap();

            let first_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );
            assert_eq!(first_messages.len(), 1);

            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["msg-2", "session-1", row_two],
            )
            .unwrap();
            assert!(db_path.with_extension("db-wal").exists());

            let refreshed_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );
            assert_eq!(refreshed_messages.len(), 2);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_dedups_across_channel_suffixed_opencode_dbs() {
        // Regression guard: a session that appears in both `opencode.db` and
        // `opencode-<channel>.db` (e.g. the user switches channels mid-session)
        // must only be counted once.
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let db_dir = source_home.path().join(".local/share/opencode");
            std::fs::create_dir_all(&db_dir).unwrap();

            let schema = "PRAGMA journal_mode=WAL;
                 PRAGMA wal_autocheckpoint=0;
                 CREATE TABLE message (
                     id TEXT PRIMARY KEY,
                     session_id TEXT NOT NULL,
                     data TEXT NOT NULL
                 );";
            let row = |input: u64, ts: u64| {
                format!(
                    r#"{{
                        "role": "assistant",
                        "modelID": "claude-sonnet-4",
                        "providerID": "anthropic",
                        "tokens": {{ "input": {input}, "output": 10, "reasoning": 0, "cache": {{ "read": 0, "write": 0 }} }},
                        "time": {{ "created": {ts}.0 }}
                    }}"#
                )
            };

            let default_db = db_dir.join("opencode.db");
            let conn = rusqlite::Connection::open(&default_db).unwrap();
            conn.execute_batch(schema).unwrap();
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    "shared-msg",
                    "session-shared",
                    row(100, 1_700_000_000_000u64)
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    "latest-only",
                    "session-latest",
                    row(200, 1_700_000_001_000u64)
                ],
            )
            .unwrap();
            drop(conn);

            let stable_db = db_dir.join("opencode-stable.db");
            let conn = rusqlite::Connection::open(&stable_db).unwrap();
            conn.execute_batch(schema).unwrap();
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    "shared-msg",
                    "session-shared",
                    row(100, 1_700_000_000_000u64)
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    "stable-only",
                    "session-stable",
                    row(300, 1_700_000_002_000u64)
                ],
            )
            .unwrap();
            drop(conn);

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );
            assert_eq!(
                messages.len(),
                3,
                "expected 3 unique messages (shared + latest-only + stable-only), got {}",
                messages.len()
            );
            let mut ids: Vec<String> = messages
                .iter()
                .filter_map(|m| m.dedup_key.clone())
                .collect();
            ids.sort();
            assert_eq!(ids, vec!["latest-only", "shared-msg", "stable-only"]);

            let messages_warm = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );
            assert_eq!(
                messages_warm.len(),
                3,
                "warm cache must also dedup shared message across channel dbs"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_opencode_sqlite_deduplicates_forked_history() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let db_dir = source_home.path().join(".local/share/opencode");
            std::fs::create_dir_all(&db_dir).unwrap();
            let db_path = db_dir.join("opencode.db");
            let conn = create_opencode_sqlite_db(&db_path);

            let msg_a = build_opencode_sqlite_payload(
                1_700_000_000_000.0,
                1_700_000_000_500.0,
                100,
                50,
                0,
                10,
                5,
                0.01,
            );
            let msg_b = build_opencode_sqlite_payload(
                1_700_000_001_000.0,
                1_700_000_001_500.0,
                200,
                80,
                10,
                20,
                0,
                0.02,
            );
            let msg_c = build_opencode_sqlite_payload(
                1_700_000_002_000.0,
                1_700_000_002_500.0,
                300,
                120,
                15,
                0,
                0,
                0.03,
            );

            for (id, session_id, payload) in [
                ("root_a", "root", msg_a.as_str()),
                ("root_b", "root", msg_b.as_str()),
                ("fork_a_copy", "fork", msg_a.as_str()),
                ("fork_b_copy", "fork", msg_b.as_str()),
                ("fork_c_new", "fork", msg_c.as_str()),
            ] {
                conn.execute(
                    "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, session_id, payload],
                )
                .unwrap();
            }
            drop(conn);

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["opencode".to_string()],
                None,
            );

            assert_eq!(messages.len(), 3);
            assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 600);
            assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 250);
            assert_eq!(messages.iter().map(|m| m.cost).sum::<f64>(), 0.06);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_local_clients_opencode_sqlite_counts_deduplicated_forked_history() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let db_dir = source_home.path().join(".local/share/opencode");
            std::fs::create_dir_all(&db_dir).unwrap();
            let db_path = db_dir.join("opencode.db");
            let conn = create_opencode_sqlite_db(&db_path);

            let msg_a = build_opencode_sqlite_payload(
                1_700_000_000_000.0,
                1_700_000_000_500.0,
                100,
                50,
                0,
                10,
                5,
                0.01,
            );
            let msg_b = build_opencode_sqlite_payload(
                1_700_000_001_000.0,
                1_700_000_001_500.0,
                200,
                80,
                10,
                20,
                0,
                0.02,
            );
            let msg_c = build_opencode_sqlite_payload(
                1_700_000_002_000.0,
                1_700_000_002_500.0,
                300,
                120,
                15,
                0,
                0,
                0.03,
            );

            for (id, session_id, payload) in [
                ("root_a", "root", msg_a.as_str()),
                ("root_b", "root", msg_b.as_str()),
                ("fork_a_copy", "fork", msg_a.as_str()),
                ("fork_b_copy", "fork", msg_b.as_str()),
                ("fork_c_new", "fork", msg_c.as_str()),
            ] {
                conn.execute(
                    "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, session_id, payload],
                )
                .unwrap();
            }
            drop(conn);

            let parsed = parse_local_clients(LocalParseOptions {
                home_dir: Some(source_home.path().to_str().unwrap().to_string()),
                use_env_roots: false,
                clients: Some(vec!["opencode".to_string()]),
                since: None,
                until: None,
                year: None,
                scanner_settings: scanner::ScannerSettings::default(),
            })
            .unwrap();

            assert_eq!(parsed.counts.get(ClientId::OpenCode), 3);
            assert_eq!(parsed.messages.len(), 3);
            assert_eq!(parsed.messages.iter().map(|m| m.input).sum::<i64>(), 600);
            assert_eq!(parsed.messages.iter().map(|m| m.output).sum::<i64>(), 250);
        }
    }

    fn write_codex_forked_history_fixture(source_home: &std::path::Path) {
        let codex_dir = source_home.join(".codex/sessions");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("parent.jsonl"),
            concat!(
                r#"{"timestamp":"2026-04-30T10:00:00Z","type":"session_meta","payload":{"id":"parent-session","source":"interactive","model_provider":"openai","cwd":"/Users/alice/root"}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"total_tokens":65},"last_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"total_tokens":65}}}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"total_tokens":130},"last_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"total_tokens":65}}}}"#,
                "\n"
            ),
        )
        .unwrap();
        std::fs::write(
            codex_dir.join("fork.jsonl"),
            concat!(
                r#"{"timestamp":"2026-04-30T10:01:00Z","type":"session_meta","payload":{"id":"fork-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","cwd":"/Users/alice/root-worktree"}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:01:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"total_tokens":130},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"total_tokens":130}}}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:01:02Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:01:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"total_tokens":65},"last_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"total_tokens":65}}}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:01:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"total_tokens":130},"last_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"total_tokens":65}}}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T10:01:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"total_tokens":143},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"total_tokens":13}}}}"#,
                "\n"
            ),
        )
        .unwrap();
    }

    fn write_codex_parent_replay_fixture(source_home: &std::path::Path) {
        let codex_dir = source_home.join(".codex/sessions");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("parent.jsonl"),
            concat!(
                r#"{"timestamp":"2026-05-24T20:00:00Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-05-24T20:00:01Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-05-24T20:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":10,"total_tokens":110},"last_token_usage":{"input_tokens":100,"output_tokens":10,"total_tokens":110}}}}"#,
                "\n",
                r#"{"timestamp":"2026-05-24T20:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":130,"output_tokens":13,"total_tokens":143},"last_token_usage":{"input_tokens":30,"output_tokens":3,"total_tokens":33}}}}"#,
                "\n"
            ),
        )
        .unwrap();

        for (filename, child_id, child_turn_id, timestamp) in [
            (
                "child-a.jsonl",
                "019e5c03-1e99-7000-8000-000000000001",
                "019e5c03-6425-7000-8000-000000000001",
                "2026-05-24T21:00:00Z",
            ),
            (
                "child-b.jsonl",
                "019e5c04-1e99-7000-8000-000000000001",
                "019e5c04-6425-7000-8000-000000000001",
                "2026-05-24T22:00:00Z",
            ),
        ] {
            std::fs::write(
                codex_dir.join(filename),
                format!(
                    concat!(
                        r#"{{"timestamp":"{timestamp}","type":"session_meta","payload":{{"id":"{child_id}","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{{"subagent":{{"thread_spawn":{{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}}}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}}}"#,
                        "\n",
                        r#"{{"timestamp":"{timestamp}","type":"session_meta","payload":{{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}}}"#,
                        "\n",
                        r#"{{"timestamp":"{timestamp}","type":"turn_context","payload":{{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}}}"#,
                        "\n",
                        r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":100,"output_tokens":10,"total_tokens":110}},"last_token_usage":{{"input_tokens":100,"output_tokens":10,"total_tokens":110}}}}}}}}"#,
                        "\n",
                        r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":130,"output_tokens":13,"total_tokens":143}},"last_token_usage":{{"input_tokens":30,"output_tokens":3,"total_tokens":33}}}}}}}}"#,
                        "\n",
                        r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"task_started","turn_id":"{child_turn_id}"}}}}"#,
                        "\n",
                        r#"{{"timestamp":"{timestamp}","type":"turn_context","payload":{{"turn_id":"{child_turn_id}","model":"gpt-5.5","cwd":"/repo"}}}}"#,
                        "\n",
                        r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":140,"output_tokens":14,"total_tokens":154}},"last_token_usage":{{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}}}}}"#,
                        "\n",
                    ),
                    timestamp = timestamp,
                    child_id = child_id,
                    child_turn_id = child_turn_id,
                ),
            )
            .unwrap();
        }
    }

    fn write_codex_user_fork_replay_fixture(source_home: &std::path::Path) {
        let sessions_dir = source_home.join(".codex/sessions/2026/01/02");
        let archived_dir = source_home.join(".codex/archived_sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&archived_dir).unwrap();

        std::fs::write(
            archived_dir.join("rollout-2026-01-02T03-04-05-11111111-1111-7111-8111-111111111111.jsonl"),
            concat!(
                r#"{"timestamp":"2026-01-02T03:04:05Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:04:06Z","type":"turn_context","payload":{"turn_id":"11111111-3333-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:04:07Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:04:08Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200,"cached_input_tokens":450,"output_tokens":120,"total_tokens":1320},"last_token_usage":{"input_tokens":200,"cached_input_tokens":50,"output_tokens":20,"total_tokens":220}}}}"#,
                "\n"
            ),
        )
        .unwrap();

        std::fs::write(
            sessions_dir.join("rollout-2026-01-02T03-10-00-22222222-2222-7222-8222-222222222222.jsonl"),
            concat!(
                r#"{"timestamp":"2026-01-02T03:10:00Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:10:00Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:10:00Z","type":"turn_context","payload":{"turn_id":"11111111-3333-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:10:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:10:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200,"cached_input_tokens":450,"output_tokens":120,"total_tokens":1320},"last_token_usage":{"input_tokens":200,"cached_input_tokens":50,"output_tokens":20,"total_tokens":220}}}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:10:30Z","type":"turn_context","payload":{"turn_id":"22222222-4444-7444-8444-444444444444","model":"gpt-5.5","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:10:30Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-01-02T03:10:53Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":500,"output_tokens":150,"total_tokens":1650},"last_token_usage":{"input_tokens":300,"cached_input_tokens":50,"output_tokens":30,"total_tokens":330}}}}"#,
                "\n"
            ),
        )
        .unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_codex_deduplicates_forked_history() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_codex_forked_history_fixture(source_home.path());

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(messages.len(), 3);
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                88
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.cache_read)
                    .sum::<i64>(),
                22
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.output)
                    .sum::<i64>(),
                33
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_codex_keeps_user_fork_own_turn() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_codex_user_fork_replay_fixture(source_home.path());

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            let session_ids: HashSet<_> = messages
                .iter()
                .map(|message| message.session_id.as_str())
                .collect();
            assert!(session_ids
                .contains("rollout-2026-01-02T03-10-00-22222222-2222-7222-8222-222222222222"));
            assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1000);
            assert_eq!(
                messages.iter().map(|m| m.tokens.cache_read).sum::<i64>(),
                500
            );
            assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 150);
        }
    }

    /// Regression fixture for issue #779: Codex CLI moves aged sessions from
    /// `~/.codex/sessions/` into a sibling `~/.codex/archived_sessions/`
    /// directory. Three distinct scenarios are covered here:
    /// - `live-only`: a session that only ever lived in `sessions/`.
    /// - `archived-only`: a session that only exists in `archived_sessions/`
    ///   (the case the collector was previously blind to, causing the
    ///   undercount reported in #779).
    /// - `shared`: the same upstream session content present in *both*
    ///   directories at once (e.g. mid-archive), which must be counted once,
    ///   not twice.
    fn write_codex_sessions_and_archived_sessions_fixture(source_home: &std::path::Path) {
        let sessions_dir = source_home.join(".codex/sessions");
        let archived_dir = source_home.join(".codex/archived_sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&archived_dir).unwrap();

        std::fs::write(
            sessions_dir.join("live-only.jsonl"),
            concat!(
                r#"{"timestamp":"2026-06-25T10:00:00Z","type":"session_meta","payload":{"id":"33333333-3333-7333-8333-333333333333","source":"interactive","model_provider":"openai","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-06-25T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
                "\n",
                r#"{"timestamp":"2026-06-25T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55},"last_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55}}}}"#,
                "\n"
            ),
        )
        .unwrap();

        std::fs::write(
            archived_dir.join("archived-only.jsonl"),
            concat!(
                r#"{"timestamp":"2026-06-20T09:00:00Z","type":"session_meta","payload":{"id":"44444444-4444-7444-8444-444444444444","source":"interactive","model_provider":"openai","cwd":"/repo"}}"#,
                "\n",
                r#"{"timestamp":"2026-06-20T09:00:01Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
                "\n",
                r#"{"timestamp":"2026-06-20T09:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":70,"output_tokens":7,"total_tokens":77},"last_token_usage":{"input_tokens":70,"output_tokens":7,"total_tokens":77}}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let shared_content = concat!(
            r#"{"timestamp":"2026-06-22T08:00:00Z","type":"session_meta","payload":{"id":"55555555-5555-7555-8555-555555555555","source":"interactive","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-06-22T08:00:01Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"2026-06-22T08:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":30,"output_tokens":3,"total_tokens":33},"last_token_usage":{"input_tokens":30,"output_tokens":3,"total_tokens":33}}}}"#,
            "\n"
        );
        std::fs::write(
            sessions_dir.join("shared-in-sessions.jsonl"),
            shared_content,
        )
        .unwrap();
        std::fs::write(
            archived_dir.join("shared-in-archived.jsonl"),
            shared_content,
        )
        .unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_codex_scans_archived_sessions_without_double_counting()
    {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_codex_sessions_and_archived_sessions_fixture(source_home.path());

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            // live-only + archived-only + shared (counted once, not twice).
            assert_eq!(
                messages.len(),
                3,
                "archived_sessions must be scanned (live-only + archived-only), and a session \
                 present in both sessions/ and archived_sessions/ must be deduplicated to one \
                 message, not counted twice"
            );

            let session_ids: HashSet<_> = messages
                .iter()
                .map(|message| message.session_id.as_str())
                .collect();
            assert!(session_ids.contains("live-only"));
            assert!(
                session_ids.contains("archived-only"),
                "archived_sessions/archived-only.jsonl must be scanned and parsed"
            );

            // 50 (live-only) + 70 (archived-only) + 30 (shared, once) = 150.
            assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 150);
            // 5 (live-only) + 7 (archived-only) + 3 (shared, once) = 15.
            assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 15);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_codex_deduplicates_parent_replay_across_forks() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_codex_parent_replay_fixture(source_home.path());

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            // Parent contributes its two turns. The two forks each replay the
            // parent history (skipped) and then emit one own turn that lands on
            // the identical cumulative total (140/14). Sibling forks sharing a
            // cumulative total is the signature of a replayed row, so the
            // fork-parent-scoped dedup key collapses them into one. Real fork
            // fan-out replays the same upstream totals into 10-100+ siblings;
            // two distinct turns reaching a byte-identical cumulative vector by
            // chance does not happen in practice because the cumulative encodes
            // each fork's divergent context size.
            assert_eq!(messages.len(), 3);
            assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 140);
            assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 14);
        }
    }

    fn write_codex_twin_token_count_fixture(source_home: &std::path::Path) {
        // Single session with two turns whose `last_token_usage` deltas are
        // byte-identical but emitted at different timestamps. The fork-dedup
        // key includes the cumulative total, so both turns must survive even
        // when a user happens to send two turns producing the same per-turn
        // delta.
        let codex_dir = source_home.join(".codex/sessions");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("twin-deltas.jsonl"),
            concat!(
                r#"{"timestamp":"2026-04-30T11:00:00Z","type":"session_meta","payload":{"id":"twin-session","source":"interactive","model_provider":"openai","cwd":"/Users/alice/root"}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T11:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T11:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                "\n",
                r#"{"timestamp":"2026-04-30T11:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":4,"output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                "\n"
            ),
        )
        .unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_with_pricing_codex_keeps_twin_token_counts_at_distinct_timestamps() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_codex_twin_token_count_fixture(source_home.path());

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(
                messages.len(),
                2,
                "two turns with identical token deltas at distinct timestamps must both survive dedup",
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                16,
                "input tokens normalize cache_read out of input: 2 turns × (10 - 2) = 16",
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.output)
                    .sum::<i64>(),
                6,
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.cache_read)
                    .sum::<i64>(),
                4,
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_local_clients_codex_counts_deduplicated_forked_history() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            write_codex_forked_history_fixture(source_home.path());

            let parsed = parse_local_clients(LocalParseOptions {
                home_dir: Some(source_home.path().to_str().unwrap().to_string()),
                use_env_roots: false,
                clients: Some(vec!["codex".to_string()]),
                since: None,
                until: None,
                year: None,
                scanner_settings: scanner::ScannerSettings::default(),
            })
            .unwrap();

            assert_eq!(parsed.counts.get(ClientId::Codex), 3);
            assert_eq!(parsed.messages.len(), 3);
            assert_eq!(
                parsed
                    .messages
                    .iter()
                    .map(|message| message.input)
                    .sum::<i64>(),
                88
            );
            assert_eq!(
                parsed
                    .messages
                    .iter()
                    .map(|message| message.cache_read)
                    .sum::<i64>(),
                22
            );
            assert_eq!(
                parsed
                    .messages
                    .iter()
                    .map(|message| message.output)
                    .sum::<i64>(),
                33
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_codex_cache_reparses_from_zero_when_incremental_prefix_is_stale() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let fresh_cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let mut cache_env = redirect_cache_home(cache_home.path());

        {
            let codex_dir = client_scan_root(source_home.path(), ClientId::Codex);
            std::fs::create_dir_all(&codex_dir).unwrap();
            let path = codex_dir.join("session.jsonl");
            std::fs::write(
                &path,
                concat!(
                    r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                    "\n"
                ),
            )
            .unwrap();

            let initial_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert_eq!(initial_messages.len(), 1);
            assert_eq!(initial_messages[0].model_id, "gpt-5.4");
            assert!(message_cache::SourceMessageCache::load()
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::Codex),
                    &path,
                )
                .and_then(|entry| entry.codex_incremental)
                .is_some());

            std::thread::sleep(std::time::Duration::from_millis(5));
            std::fs::write(
                &path,
                concat!(
                    r#"{"type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
                    "\n"
                ),
            )
            .unwrap();

            let warm_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            point_cache_home(&mut cache_env, fresh_cache_home.path());
            let fresh_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(warm_messages, fresh_messages);
            assert_eq!(warm_messages.len(), 2);
            assert!(warm_messages
                .iter()
                .all(|message| message.model_id == "gpt-5.5"));
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_source_cache_keeps_untimestamped_rows_in_sync_after_append() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let fresh_cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let mut cache_env = redirect_cache_home(cache_home.path());

        {
            let codex_dir = source_home.path().join(".codex/sessions");
            std::fs::create_dir_all(&codex_dir).unwrap();
            let path = codex_dir.join("session.jsonl");
            std::fs::write(
                &path,
                concat!(
                    r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                    "\n"
                ),
            )
            .unwrap();

            let first_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert_eq!(first_messages.len(), 1);

            std::thread::sleep(std::time::Duration::from_millis(5));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(
                concat!(
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
                    "\n"
                )
                .as_bytes(),
            )
            .unwrap();
            file.flush().unwrap();

            let warm_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            point_cache_home(&mut cache_env, fresh_cache_home.path());
            let fresh_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(warm_messages, fresh_messages);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_source_cache_matches_cold_parse_after_malformed_json_append() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let fresh_cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let mut cache_env = redirect_cache_home(cache_home.path());

        {
            let codex_dir = source_home.path().join(".codex/sessions");
            std::fs::create_dir_all(&codex_dir).unwrap();
            let path = codex_dir.join("session.jsonl");
            std::fs::write(
                &path,
                concat!(
                    r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":999""#,
                    "\n"
                ),
            )
            .unwrap();

            let initial_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert_eq!(initial_messages.len(), 1);

            std::thread::sleep(std::time::Duration::from_millis(5));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(
                concat!(
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
                    "\n"
                )
                .as_bytes(),
            )
            .unwrap();
            file.flush().unwrap();

            let warm_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert!(message_cache::SourceMessageCache::load()
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::Codex),
                    &path,
                )
                .is_none());

            point_cache_home(&mut cache_env, fresh_cache_home.path());
            let fresh_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(warm_messages, fresh_messages);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_exact_hit_codex_cache_repairs_fallback_timestamps_without_incremental_state() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = source_home.path().join(".codex/sessions");
            std::fs::create_dir_all(&session_dir).unwrap();
            let path = session_dir.join("session.jsonl");
            std::fs::write(
                &path,
                concat!(
                    r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                    "\n"
                ),
            )
            .unwrap();

            let expected = crate::sessions::codex::parse_codex_file(&path);
            assert_eq!(expected.len(), 1);

            let fingerprint = message_cache::SourceFingerprint::from_path(&path).unwrap();
            let mut stale_message = expected[0].clone();
            stale_message.timestamp = 0;
            stale_message.date = "1900-01-01".to_string();

            let mut cache = message_cache::SourceMessageCache::default();
            cache.insert(message_cache::CachedSourceEntry::new(
                message_cache::CacheIdentity::for_client(ClientId::Codex),
                &path,
                fingerprint,
                vec![stale_message],
                vec![0],
                None,
            ));
            cache.save_if_dirty();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(messages, expected);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_codex_cache_repairs_fallback_timestamps_after_source_mtime_change() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let fresh_cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let mut cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = source_home.path().join(".codex/sessions");
            std::fs::create_dir_all(&session_dir).unwrap();
            let path = session_dir.join("session.jsonl");
            let contents = concat!(
                r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                "\n"
            );
            std::fs::write(&path, contents).unwrap();

            let initial_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert_eq!(initial_messages.len(), 1);

            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::write(&path, contents).unwrap();

            let warm_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            point_cache_home(&mut cache_env, fresh_cache_home.path());
            let fresh_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(warm_messages, fresh_messages);
            assert_ne!(warm_messages[0].timestamp, initial_messages[0].timestamp);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_full_log_parse_preserves_valid_messages_before_invalid_line_error() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = source_home.path().join(".codex/sessions");
            std::fs::create_dir_all(&session_dir).unwrap();
            let path = session_dir.join("session.jsonl");

            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(
                concat!(
                    r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                    "\n"
                )
                .as_bytes(),
            )
            .unwrap();
            file.write_all(&[0xff, b'\n']).unwrap();
            file.flush().unwrap();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].model_id, "gpt-5.4");

            let cache = message_cache::SourceMessageCache::load();
            assert!(cache
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::Codex),
                    &path,
                )
                .is_none());
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_codex_cache_does_not_persist_unknown_before_later_turn_context() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let fresh_cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let mut cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = client_scan_root(source_home.path(), ClientId::Codex);
            std::fs::create_dir_all(&session_dir).unwrap();
            let path = session_dir.join("session.jsonl");
            std::fs::write(
                &path,
                concat!(
                    r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
                    "\n",
                    r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                    "\n"
                ),
            )
            .unwrap();

            let initial_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert_eq!(initial_messages.len(), 1);
            assert_eq!(initial_messages[0].model_id, "unknown");
            assert!(message_cache::SourceMessageCache::load()
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::Codex),
                    &path,
                )
                .is_none());

            std::thread::sleep(std::time::Duration::from_millis(5));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(
                concat!(
                    r#"{"timestamp":"2026-04-27T10:00:04Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
                    "\n"
                )
                .as_bytes(),
            )
            .unwrap();
            file.flush().unwrap();

            let resumed_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            point_cache_home(&mut cache_env, fresh_cache_home.path());
            let fresh_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(resumed_messages, fresh_messages);
            assert_eq!(resumed_messages.len(), 1);
            assert_eq!(resumed_messages[0].model_id, "gpt-5.5");

            point_cache_home(&mut cache_env, cache_home.path());
            assert!(message_cache::SourceMessageCache::load()
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::Codex),
                    &path,
                )
                .is_some());
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_codex_cache_skips_non_newline_terminated_resume_prefix() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let fresh_cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let mut cache_env = redirect_cache_home(cache_home.path());

        {
            let session_dir = source_home.path().join(".codex/sessions");
            std::fs::create_dir_all(&session_dir).unwrap();
            let path = session_dir.join("session.jsonl");
            std::fs::write(
                &path,
                concat!(
                    r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#
                ),
            )
            .unwrap();

            let initial_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );
            assert_eq!(initial_messages.len(), 1);
            assert!(message_cache::SourceMessageCache::load()
                .get(
                    message_cache::CacheIdentity::for_client(ClientId::Codex),
                    &path,
                )
                .is_none());

            std::thread::sleep(std::time::Duration::from_millis(5));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(
                concat!(
                    "\n",
                    r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
                    "\n"
                )
                .as_bytes(),
            )
            .unwrap();
            file.flush().unwrap();

            let warm_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            point_cache_home(&mut cache_env, fresh_cache_home.path());
            let fresh_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["codex".to_string()],
                None,
            );

            assert_eq!(warm_messages, fresh_messages);
            assert_eq!(warm_messages.len(), 2);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_source_cache_does_not_reuse_priced_cost_without_pricing_service() {
        let temp_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(temp_home.path());
        {
            let cursor_cache_dir = source_home.path().join(".config/tokscale/cursor-cache");
            std::fs::create_dir_all(&cursor_cache_dir).unwrap();

            let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2026-03-04T12:00:00.000Z","Included","Composer 1.5","No","1200","1000","5000","2000","8000","Included""#;
            std::fs::write(cursor_cache_dir.join("usage.csv"), csv).unwrap();

            let mut litellm = HashMap::new();
            litellm.insert(
                "Composer 1.5".into(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(0.001),
                    output_cost_per_token: Some(0.002),
                    cache_read_input_token_cost: Some(0.0005),
                    ..Default::default()
                },
            );
            let pricing = pricing::PricingService::new(litellm, HashMap::new());

            let repriced_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["cursor".to_string()],
                Some(&pricing),
            );
            assert_eq!(repriced_messages.len(), 1);
            assert!(repriced_messages[0].cost > 0.0);

            let cached_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["cursor".to_string()],
                None,
            );

            assert_eq!(cached_messages.len(), 1);
            assert_eq!(cached_messages[0].cost, 0.0);
        }
    }

    #[test]
    fn test_apply_pricing_if_available_keeps_existing_cost_without_pricing() {
        let mut msg = UnifiedMessage::new_with_agent(
            "roocode",
            "gpt-4o",
            "provider",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.42,
            Some("planner".to_string()),
        );

        apply_pricing_if_available(&mut msg, None);

        assert_eq!(msg.cost, 0.42);
    }

    #[test]
    fn strict_pricing_validation_accepts_covered_and_provider_reported_usage() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "covered-model".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let covered = UnifiedMessage::new(
            "synthetic",
            "covered-model",
            "openai",
            "covered",
            1_733_011_200_000,
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            0.0,
        );
        let mut reported = UnifiedMessage::new(
            "synthetic",
            "unlisted-model",
            "provider",
            "reported",
            1_733_011_200_000,
            TokenBreakdown {
                output: 1,
                ..Default::default()
            },
            0.0,
        );
        reported.mark_provider_reported_cost();

        assert!(validate_priced_messages(&[covered, reported], Some(&pricing)).is_ok());
    }

    #[test]
    fn strict_pricing_validation_rejects_unpriced_token_usage() {
        let message = UnifiedMessage::new(
            "synthetic",
            "unlisted-model",
            "provider",
            "unpriced",
            1_733_011_200_000,
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            0.0,
        );
        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());

        let error = validate_priced_messages(&[message], Some(&pricing)).unwrap_err();
        assert!(error.contains("provider/unlisted-model"));
    }

    // Regression: #1013. The message used to repeat one entry per affected
    // message, so a real submission produced a ~290KB error that scrolled the
    // actionable model ids off screen.
    #[test]
    fn strict_pricing_validation_error_deduplicates_models_with_counts() {
        let unpriced = |model: &str, session: &str| {
            UnifiedMessage::new(
                "synthetic",
                model,
                "provider",
                session,
                1_733_011_200_000,
                TokenBreakdown {
                    input: 1,
                    ..Default::default()
                },
                0.0,
            )
        };
        let messages = vec![
            unpriced("repeated-model", "a"),
            unpriced("repeated-model", "b"),
            unpriced("repeated-model", "c"),
            unpriced("single-model", "d"),
        ];
        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());

        let error = validate_priced_messages(&messages, Some(&pricing)).unwrap_err();

        assert_eq!(error.matches("provider/repeated-model").count(), 1);
        assert_eq!(error.matches("provider/single-model").count(), 1);
        assert!(
            error.contains("provider/repeated-model (x3)"),
            "repeated ids must carry an occurrence count: {error}"
        );
        assert!(
            !error.contains("provider/single-model (x"),
            "single occurrences must not be annotated: {error}"
        );
        assert!(
            error.find("provider/repeated-model") < error.find("provider/single-model"),
            "ids must keep first-seen order: {error}"
        );
    }

    #[test]
    fn graph_pricing_policy_excludes_unpriced_only_from_submission() {
        let message = UnifiedMessage::new(
            "opencode",
            "genuinely-unpriced-model",
            "unknown-provider",
            "unpriced",
            1_736_510_400_000,
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            0.0,
        );
        // Populated but not covering this model: an empty service would instead
        // trip the "no pricing dataset loaded" guard, which is a different case.
        let pricing = pricing::PricingService::new(unrelated_litellm_dataset(), HashMap::new());

        let report = build_graph_from_messages(
            vec![message.clone()],
            Some(&pricing),
            GraphPricingRequirement::Lenient,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("reporting graphs should retain unpriced usage");
        let submission = build_graph_from_messages(
            vec![message],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("submission graphs should exclude unpriced usage");

        assert_eq!(report.summary.total_tokens, 1);
        assert_eq!(submission.summary.total_tokens, 0);
        assert_eq!(submission.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            submission.unpriced_submission_exclusions[0].model_id,
            "genuinely-unpriced-model"
        );
    }

    /// A dataset that loaded successfully but prices an unrelated model.
    ///
    /// Tests asserting "this model is unpriced" must use this rather than an
    /// empty service: an empty service means *no dataset loaded*, which is a
    /// separate, fatal condition on the submission path.
    fn unrelated_litellm_dataset() -> HashMap<String, pricing::ModelPricing> {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4o".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        );
        litellm
    }

    #[test]
    fn submission_without_any_pricing_data_still_fails() {
        let message = UnifiedMessage::new(
            "opencode",
            "gpt-4o",
            "openai",
            "priced-if-data-loaded",
            1_736_510_400_000,
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            0.0,
        );

        // `None` is unreachable from `generate_submission_graph`, which always
        // passes `Some(..)` because `PricingService::get_or_init` degrades every
        // failed source to an empty map rather than erroring. The reachable
        // shape of "no pricing dataset loaded" is a populated-with-nothing
        // service, so both must fail identically.
        let empty = pricing::PricingService::new(HashMap::new(), HashMap::new());
        for (label, pricing) in [
            ("no service at all", None),
            ("a service with no dataset", Some(&empty)),
        ] {
            let Err(error) = build_graph_from_messages(
                vec![message.clone()],
                pricing,
                GraphPricingRequirement::Submission,
                std::time::Instant::now(),
                &crate::bucket_tz::BucketTimezone::Local,
            ) else {
                panic!("submission must fail with {label}");
            };

            assert_eq!(error, "pricing data is unavailable for submission");
        }
    }

    /// A missing pricing dataset only matters if something needed pricing.
    /// Provider-reported costs are authoritative, so a batch made entirely of
    /// them must still submit during a total upstream outage.
    #[test]
    fn submission_without_pricing_data_still_accepts_provider_reported_usage() {
        let mut message = UnifiedMessage::new(
            "opencode",
            "some-model",
            "anthropic",
            "provider-reported",
            1_736_510_400_000,
            TokenBreakdown {
                input: 1_000,
                output: 500,
                ..Default::default()
            },
            0.05,
        );
        message.mark_provider_reported_cost();
        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());

        let graph = build_graph_from_messages(
            vec![message],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("authoritative costs must not need a pricing dataset");

        assert_eq!(graph.summary.total_tokens, 1_500);
        assert!(graph.unpriced_submission_exclusions.is_empty());
    }

    #[test]
    fn submission_excludes_unpriced_generic_gemini_default_but_keeps_priceable_usage() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4o".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(1e-6),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let mut generic = UnifiedMessage::new(
            "antigravity-cli",
            "gemini-default",
            "google",
            "generic",
            1_736_510_400_000,
            TokenBreakdown {
                input: 7,
                cache_read: 11,
                ..Default::default()
            },
            0.0,
        );
        generic.message_count = 7;
        let concrete = UnifiedMessage::new(
            "synthetic",
            "gpt-4o",
            "openai",
            "concrete",
            1_736_510_400_000,
            TokenBreakdown {
                input: 13,
                ..Default::default()
            },
            0.0,
        );

        let graph = build_graph_from_messages(
            vec![generic, concrete],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("generic routing label must not block fully priced submission usage");

        assert_eq!(graph.summary.total_tokens, 13);
        assert_eq!(graph.contributions[0].clients.len(), 1);
        assert_eq!(graph.contributions[0].clients[0].model_id, "gpt-4o");
        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0],
            UnpricedSubmissionExclusion {
                provider_id: "google".to_string(),
                model_id: "gemini-default".to_string(),
                message_count: 7,
                total_tokens: 18,
                reason: ROUTING_LABEL_UNPRICED_REASON,
            }
        );
    }

    #[test]
    fn submission_excludes_unpriced_auto_routing_label() {
        // `auto` is the unknown-model label Kiro emits and the default-model
        // label Cursor/Copilot record in usage rows. A models.dev `morph/auto`
        // paid row exists, so before the resolver refused routing labels the
        // bare label resolved to it and slipped through submission at morph's
        // rates (#1062). The fixture quotes a cache-read rate precisely so the
        // pre-fix fallback covers all three populated buckets (7 input, 11
        // cache-read, 0 output) and submits the row — without it, the row
        // would fail coverage on the missing cache rate and the test would
        // pass even with the bug. The label must instead be excluded with the
        // routing-label reason.
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "morph/auto".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(8.5e-7),
                output_cost_per_token: Some(1.55e-6),
                cache_read_input_token_cost: Some(1.6e-7),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new_with_custom_and_models_dev(
            pricing::custom::CustomPricing::default(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );
        let mut auto = UnifiedMessage::new(
            "kiro",
            "auto",
            "amazon-bedrock",
            "generic",
            1_736_510_400_000,
            TokenBreakdown {
                input: 7,
                cache_read: 11,
                ..Default::default()
            },
            0.0,
        );
        auto.message_count = 7;

        let graph = build_graph_from_messages(
            vec![auto],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("routing label must not abort submission");

        assert_eq!(graph.summary.total_tokens, 0);
        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0],
            UnpricedSubmissionExclusion {
                provider_id: "amazon-bedrock".to_string(),
                model_id: "auto".to_string(),
                message_count: 7,
                total_tokens: 18,
                reason: ROUTING_LABEL_UNPRICED_REASON,
            }
        );
    }

    #[test]
    fn whitespace_padded_routing_label_is_classified_the_same_by_resolver_and_reason() {
        // `lookup::is_routing_label` trims before comparing, so the resolver
        // refuses to price ` auto `. The exclusion reason has to agree, or the
        // row is reported as having no model-to-price mapping while the reason
        // it is unpriced is that it names a router. Both paths now read the
        // same list, so a label added to `lookup::ROUTING_LABELS` cannot drift
        // out of the reason.
        assert_eq!(
            crate::pricing::lookup::is_routing_label(" auto "),
            is_generic_routing_label("amazon-bedrock", " auto "),
            "resolver and exclusion reason must classify a padded routing label alike"
        );

        // The models.dev `morph/auto` row is fully priced, so if the resolver
        // did not refuse the padded label the row would submit at Morph rates
        // (#1062) instead of reaching the exclusion path at all.
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "morph/auto".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(8.5e-7),
                output_cost_per_token: Some(1.55e-6),
                cache_read_input_token_cost: Some(1.6e-7),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new_with_custom_and_models_dev(
            pricing::custom::CustomPricing::default(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );
        let mut padded = UnifiedMessage::new(
            "kiro",
            " auto ",
            "amazon-bedrock",
            "generic",
            1_736_510_400_000,
            TokenBreakdown {
                input: 7,
                cache_read: 11,
                ..Default::default()
            },
            0.0,
        );
        padded.message_count = 7;

        let graph = build_graph_from_messages(
            vec![padded],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("routing label must not abort submission");

        assert_eq!(graph.summary.total_tokens, 0);
        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0].reason, ROUTING_LABEL_UNPRICED_REASON,
            "padded routing label must report the routing-label reason"
        );
    }

    #[test]
    fn custom_priced_routing_label_reports_incomplete_pricing_not_missing_mapping() {
        // A `custom-pricing.json` entry for a routing label is the user
        // stating what their router actually costs them — the escape hatch
        // `ROUTING_LABELS` documents. Telling that user the label "has no
        // authoritative model-to-price mapping" contradicts the mapping they
        // just wrote. Here the custom entry quotes an input rate but no cache
        // rate, so the row still fails coverage; the reason must name the gap
        // that is actually fixable (the missing cache-read rate), not deny the
        // mapping exists.
        let mut custom_models = HashMap::new();
        custom_models.insert(
            "auto".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(3e-6),
                output_cost_per_token: Some(1.5e-5),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new_with_custom(
            pricing::custom::CustomPricing::from_models(custom_models),
            HashMap::new(),
            HashMap::new(),
        );
        let mut auto = UnifiedMessage::new(
            "kiro",
            "auto",
            "amazon-bedrock",
            "generic",
            1_736_510_400_000,
            TokenBreakdown {
                input: 7,
                cache_read: 11,
                ..Default::default()
            },
            0.0,
        );
        auto.message_count = 7;

        let graph = build_graph_from_messages(
            vec![auto],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("routing label must not abort submission");

        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0],
            UnpricedSubmissionExclusion {
                provider_id: "amazon-bedrock".to_string(),
                model_id: "auto".to_string(),
                message_count: 7,
                total_tokens: 18,
                reason: INCOMPLETE_MODEL_PRICING_REASON,
            }
        );
    }

    #[test]
    fn submission_excludes_unpriced_concrete_models() {
        let concrete = UnifiedMessage::new(
            "synthetic",
            "gemini-3.5-pro",
            "google",
            "concrete",
            1_736_510_400_000,
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            0.0,
        );
        // Populated but not covering this model — see `unrelated_litellm_dataset`.
        let pricing = pricing::PricingService::new(unrelated_litellm_dataset(), HashMap::new());

        let graph = build_graph_from_messages(
            vec![concrete],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("one unpriced model must not block the submission");

        assert_eq!(graph.summary.total_tokens, 0);
        assert!(graph.contributions.is_empty());
        assert_eq!(
            graph.unpriced_submission_exclusions,
            vec![UnpricedSubmissionExclusion {
                provider_id: "google".to_string(),
                model_id: "gemini-3.5-pro".to_string(),
                message_count: 1,
                total_tokens: 1,
                reason: MISSING_MODEL_PRICING_REASON,
            }]
        );
    }

    /// An unscoped model-part fallback proves only the model spelling, not
    /// which provider served it. The estimate remains available locally, while
    /// submission excludes it with the provider-specific evidence gap.
    #[test]
    fn submission_excludes_cross_provider_model_part_estimate() {
        let openrouter = HashMap::from([(
            "vendor/atlas-chat".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        )]);
        let pricing = pricing::PricingService::new(HashMap::new(), openrouter);
        let message = UnifiedMessage::new(
            "synthetic",
            "atlas-chat",
            "unknown",
            "cross-provider-model-part",
            1_736_510_400_000,
            TokenBreakdown {
                input: 100,
                output: 50,
                ..Default::default()
            },
            0.0,
        );

        let estimate = pricing
            .lookup_with_source("atlas-chat", None)
            .expect("the model-part estimate remains available for reporting");
        assert_eq!(estimate.matched_key, "vendor/atlas-chat");
        assert_eq!(
            estimate.evidence.kind,
            pricing::lookup::ResolutionKind::ModelPart
        );

        let graph = build_graph_from_messages(
            vec![message],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("the unsafe estimate must be excluded, not abort the graph");

        assert!(graph.contributions.is_empty());
        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0].reason,
            UNVERIFIED_PROVIDER_IDENTITY_REASON
        );
    }

    /// Prefix probing and provider-tag aliases are useful lookup fallbacks,
    /// not proof that the recorded provider used the candidate's billing
    /// endpoint. Both stay estimate-only at the submission boundary.
    #[test]
    fn submission_excludes_provider_prefix_and_cross_endpoint_alias_estimates() {
        let litellm = HashMap::from([
            (
                "anthropic/atlas-chat".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(1e-6),
                    output_cost_per_token: Some(2e-6),
                    ..Default::default()
                },
            ),
            (
                "vertex_ai/vertex-chat".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(3e-6),
                    output_cost_per_token: Some(6e-6),
                    ..Default::default()
                },
            ),
            (
                "vertex_ai/accounts/anthropic/models/vertex-chat".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(3e-6),
                    output_cost_per_token: Some(6e-6),
                    ..Default::default()
                },
            ),
        ]);
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let usage = TokenBreakdown {
            input: 100,
            output: 50,
            ..Default::default()
        };
        let messages = vec![
            UnifiedMessage::new(
                "synthetic",
                "atlas-chat",
                "synthetic",
                "provider-prefix",
                1_736_510_400_000,
                usage.clone(),
                0.0,
            ),
            UnifiedMessage::new(
                "synthetic",
                "vertex-chat",
                "anthropic",
                "cross-endpoint-alias",
                1_736_510_400_001,
                usage.clone(),
                0.0,
            ),
            UnifiedMessage::new(
                "synthetic",
                "accounts/anthropic/models/vertex-chat",
                "anthropic",
                "scoped-cross-endpoint-alias",
                1_736_510_400_002,
                usage,
                0.0,
            ),
        ];

        let graph = build_graph_from_messages(
            messages,
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("unsafe estimates must be excluded, not abort the graph");

        assert!(graph.contributions.is_empty());
        assert_eq!(graph.unpriced_submission_exclusions.len(), 3);
        for exclusion in graph.unpriced_submission_exclusions {
            assert_eq!(exclusion.reason, UNVERIFIED_PROVIDER_IDENTITY_REASON);
        }

        // Source-constrained OpenRouter uses a separate scoped wrapper. Keep a
        // graph-level guard with only that source loaded so it cannot regress
        // independently from the auto/LiteLLM path above.
        let openrouter_pricing = pricing::PricingService::new(
            HashMap::new(),
            HashMap::from([(
                "vertex_ai/accounts/anthropic/models/vertex-chat".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(3e-6),
                    output_cost_per_token: Some(6e-6),
                    ..Default::default()
                },
            )]),
        );
        let openrouter_graph = build_graph_from_messages(
            vec![UnifiedMessage::new(
                "synthetic",
                "accounts/anthropic/models/vertex-chat",
                "anthropic",
                "openrouter-scoped-cross-endpoint-alias",
                1_736_510_400_003,
                TokenBreakdown {
                    input: 100,
                    output: 50,
                    ..Default::default()
                },
                0.0,
            )],
            Some(&openrouter_pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("the OpenRouter alias estimate must be excluded");
        assert!(openrouter_graph.contributions.is_empty());
        assert_eq!(
            openrouter_graph.unpriced_submission_exclusions[0].reason,
            UNVERIFIED_PROVIDER_IDENTITY_REASON
        );
    }

    /// A fuzzy lookup with one candidate is excluded because nothing proves
    /// the priced key names the model that was used — not because candidates
    /// disagreed. There is only one candidate, so it cannot disagree with
    /// anything, and reporting a disagreement would send audit and submission
    /// diagnostics after a conflict that does not exist.
    #[test]
    fn submission_excludes_single_candidate_fuzzy_price_for_unverified_identity() {
        let litellm = HashMap::from([(
            "vendor-a/atlas-chat-preview".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        )]);
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let message = UnifiedMessage::new(
            "synthetic",
            "atlas-chat",
            "unknown",
            "single-candidate",
            1_736_510_400_000,
            TokenBreakdown {
                input: 100,
                output: 50,
                ..Default::default()
            },
            0.0,
        );

        let resolution = pricing
            .lookup_with_source_and_provider("atlas-chat", None, Some("unknown"))
            .expect("the estimate still resolves for reporting");
        assert_eq!(resolution.evidence.candidate_count, 1);
        assert!(
            resolution.evidence.price_consensus,
            "a lone candidate agrees with itself"
        );
        assert!(!resolution.evidence.exact_model_identity);

        let graph = build_graph_from_messages(
            vec![message],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("an unverified estimate must be excluded, not abort the graph");

        assert!(graph.contributions.is_empty());
        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0].reason,
            UNVERIFIED_MODEL_IDENTITY_REASON
        );
    }

    #[test]
    fn submission_excludes_ambiguous_fuzzy_price_with_specific_reason() {
        let litellm = HashMap::from([
            (
                "vendor-a/atlas-chat-preview".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(1e-6),
                    output_cost_per_token: Some(2e-6),
                    ..Default::default()
                },
            ),
            (
                "vendor-b/atlas-chat-beta".to_string(),
                pricing::ModelPricing {
                    input_cost_per_token: Some(3e-6),
                    output_cost_per_token: Some(6e-6),
                    ..Default::default()
                },
            ),
        ]);
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let message = UnifiedMessage::new(
            "synthetic",
            "atlas-chat",
            "unknown",
            "ambiguous",
            1_736_510_400_000,
            TokenBreakdown {
                input: 100,
                output: 50,
                ..Default::default()
            },
            0.0,
        );

        let graph = build_graph_from_messages(
            vec![message],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("an ambiguous estimate must be excluded, not abort the graph");

        assert!(graph.contributions.is_empty());
        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0].reason,
            AMBIGUOUS_MODEL_PRICING_REASON
        );
    }

    #[test]
    fn submission_reports_ambiguous_evidence_from_a_borrowed_bucket_rate() {
        let disputed_cache_row = |cache_read: f64| pricing::ModelPricing {
            input_cost_per_token: Some(1e-6),
            output_cost_per_token: Some(2e-6),
            cache_read_input_token_cost: Some(cache_read),
            ..Default::default()
        };
        let litellm = HashMap::from([
            (
                "azure_ai/atlas-chat".to_string(),
                pricing::ModelPricing {
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
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let message = UnifiedMessage::new(
            "synthetic",
            "atlas-chat",
            "azure",
            "borrowed-ambiguous-cache-rate",
            1_736_510_400_000,
            TokenBreakdown {
                input: 100,
                output: 50,
                cache_read: 20,
                ..Default::default()
            },
            0.0,
        );

        let resolution = pricing
            .resolve_for_usage_with_provider(
                &message.model_id,
                Some(&message.provider_id),
                &message.tokens,
            )
            .expect("the estimate should remain visible");
        assert_eq!(
            resolution.evidence.submission_safety_gap(),
            Some(pricing::lookup::SubmissionSafetyGap::PriceDisagreement)
        );

        let graph = build_graph_from_messages(
            vec![message],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("ambiguous borrowed pricing must be excluded, not abort the graph");

        assert!(graph.contributions.is_empty());
        assert_eq!(graph.unpriced_submission_exclusions.len(), 1);
        assert_eq!(
            graph.unpriced_submission_exclusions[0].reason,
            AMBIGUOUS_MODEL_PRICING_REASON
        );
    }

    #[test]
    fn submission_excludes_usage_with_an_unpriced_cache_write_bucket() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-5.5".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(5e-6),
                output_cost_per_token: Some(30e-6),
                cache_read_input_token_cost: Some(0.5e-6),
                ..Default::default()
            },
        );
        litellm.insert(
            "gpt-4o".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(1e-6),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let incomplete = UnifiedMessage::new(
            "hermes",
            "gpt-5.5",
            "custom",
            "incomplete",
            1_736_510_400_000,
            TokenBreakdown {
                input: 10,
                cache_read: 20,
                cache_write: 30,
                ..Default::default()
            },
            0.0,
        );
        let covered = UnifiedMessage::new(
            "synthetic",
            "gpt-4o",
            "openai",
            "covered",
            1_736_510_400_000,
            TokenBreakdown {
                input: 40,
                ..Default::default()
            },
            0.0,
        );

        let graph = build_graph_from_messages(
            vec![incomplete, covered],
            Some(&pricing),
            GraphPricingRequirement::Submission,
            std::time::Instant::now(),
            &crate::bucket_tz::BucketTimezone::Local,
        )
        .expect("an incomplete cache rate must not block covered usage");

        assert_eq!(graph.summary.total_tokens, 40);
        assert_eq!(graph.contributions[0].clients[0].model_id, "gpt-4o");
        assert_eq!(
            graph.unpriced_submission_exclusions,
            vec![UnpricedSubmissionExclusion {
                provider_id: "custom".to_string(),
                model_id: "gpt-5.5".to_string(),
                message_count: 1,
                total_tokens: 60,
                reason: INCOMPLETE_MODEL_PRICING_REASON,
            }]
        );
    }

    #[test]
    fn strict_pricing_validation_accepts_bundled_pricing() {
        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());
        let message = UnifiedMessage::new(
            "cursor",
            "composer-2.5",
            "cursor",
            "bundled",
            1_733_011_200_000,
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            0.0,
        );

        assert!(validate_priced_messages(&[message], Some(&pricing)).is_ok());
    }

    #[test]
    fn strict_pricing_validation_ignores_filtered_out_unpriced_usage() {
        let mut old = UnifiedMessage::new(
            "synthetic",
            "unlisted-model",
            "provider",
            "old",
            1_733_011_200_000,
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            0.0,
        );
        old.date = "2020-01-01".to_string();
        let filtered = filter_messages_for_report(
            vec![old],
            &ReportOptions {
                since: Some("2021-01-01".to_string()),
                ..Default::default()
            },
        );

        assert!(validate_priced_messages(
            &filtered,
            Some(&pricing::PricingService::new(
                HashMap::new(),
                HashMap::new()
            ))
        )
        .is_ok());
    }

    #[test]
    fn strict_pricing_validation_requires_each_populated_bucket_to_have_a_base_rate() {
        let mut custom = HashMap::new();
        custom.insert(
            "input-only".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        custom.insert(
            "output-only".to_string(),
            pricing::ModelPricing {
                output_cost_per_token: Some(1e-6),
                ..Default::default()
            },
        );
        custom.insert(
            "tier-only".to_string(),
            pricing::ModelPricing {
                input_cost_per_token_above_272k_tokens: Some(1e-6),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new_with_custom(
            pricing::custom::CustomPricing::from_models(custom),
            HashMap::new(),
            HashMap::new(),
        );
        let usage = |model, input, output, reasoning, cache_read, cache_write| {
            UnifiedMessage::new(
                "synthetic",
                model,
                "provider",
                model,
                1_733_011_200_000,
                TokenBreakdown {
                    input,
                    output,
                    reasoning,
                    cache_read,
                    cache_write,
                },
                0.0,
            )
        };

        assert!(
            validate_priced_messages(&[usage("input-only", 1, 0, 0, 0, 0)], Some(&pricing)).is_ok()
        );
        assert!(
            validate_priced_messages(&[usage("input-only", 0, 1, 0, 0, 0)], Some(&pricing))
                .is_err()
        );
        assert!(
            validate_priced_messages(&[usage("output-only", 0, 1, 1, 0, 0)], Some(&pricing))
                .is_ok()
        );
        assert!(
            validate_priced_messages(&[usage("output-only", 1, 0, 0, 0, 0)], Some(&pricing))
                .is_err()
        );
        assert!(
            validate_priced_messages(&[usage("output-only", 0, 0, 0, 1, 0)], Some(&pricing))
                .is_err()
        );
        assert!(
            validate_priced_messages(&[usage("output-only", 0, 0, 0, 0, 1)], Some(&pricing))
                .is_err()
        );
        assert!(validate_priced_messages(
            &[usage("tier-only", 300_000, 0, 0, 0, 0)],
            Some(&pricing)
        )
        .is_err());
    }

    #[test]
    fn test_apply_pricing_if_available_overrides_cost_when_pricing_exists() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4o".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "codex",
            "gpt-4o",
            "provider",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.02);
    }

    #[test]
    fn test_apply_pricing_if_available_preserves_cursor_provider_reported_cost() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4o".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "cursor",
            "gpt-4o",
            "openai",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.42,
        );
        msg.mark_provider_reported_cost();

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.42);
    }

    #[test]
    fn test_apply_pricing_if_available_applies_zed_hosted_markup() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4-5".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "zed",
            "claude-sonnet-4-5",
            crate::sessions::zed::ZED_HOSTED_PROVIDER,
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert!((msg.cost - 0.022).abs() < 1e-12);
    }

    #[test]
    fn test_apply_pricing_if_available_skips_zed_markup_for_non_zed_client() {
        // Non-zed client with provider_id "zed.dev" must not receive the +10%
        // markup. The multiplier is gated on (client == "zed" AND provider).
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4-5".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "claudecode",
            "claude-sonnet-4-5",
            crate::sessions::zed::ZED_HOSTED_PROVIDER,
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        // 10 * 0.001 + 5 * 0.002 = 0.020, no markup.
        assert!((msg.cost - 0.020).abs() < 1e-12);
    }

    #[test]
    fn test_apply_pricing_if_available_skips_zed_markup_for_byok_provider() {
        // A Zed message whose provider_id is the upstream provider directly
        // (BYOK / non-hosted path) must not be marked up — the user is paying
        // the upstream API directly, not through Zed.
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4-5".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "zed",
            "claude-sonnet-4-5",
            "anthropic",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert!((msg.cost - 0.020).abs() < 1e-12);
    }

    #[test]
    fn test_apply_pricing_if_available_uses_reasoning_for_gemini() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gemini-2.5-pro".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "gemini",
            "gemini-2.5-pro",
            "google",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 7,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.034);
    }

    #[test]
    fn test_apply_pricing_if_available_uses_cache_read_pricing_for_gemini() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gemini-2.5-pro".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                cache_read_input_token_cost: Some(0.0001),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "gemini",
            "gemini-2.5-pro",
            "google",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 7,
                cache_write: 0,
                reasoning: 3,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.0267);
    }

    #[test]
    fn test_apply_pricing_if_available_uses_market_rate_for_free_variant() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "z-ai/glm-4.7".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(HashMap::new(), openrouter);

        let mut msg = UnifiedMessage::new(
            "opencode",
            "glm-4.7-free",
            "modal",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.02);
    }

    #[test]
    fn test_apply_pricing_if_available_prefers_provider_aware_match() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "xai/grok-code-fast-1-0825".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        litellm.insert(
            "azure_ai/grok-code-fast-1".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "opencode",
            "grok-code",
            "azure",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.2);
    }

    #[test]
    fn test_apply_pricing_if_available_uses_nested_reseller_exact_match() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-4".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        litellm.insert(
            "azure/openai/gpt-4".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "opencode",
            "gpt-4",
            "azure",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.2);
    }

    #[test]
    fn test_apply_pricing_if_available_keeps_scoped_fireworks_cost_without_exact_pricing() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "fireworks_ai/accounts/fireworks/models/deepseek-r1-0528-distill-qwen3-8b".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.0000002),
                output_cost_per_token: Some(0.0000002),
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "deepseek/deepseek-v4-pro".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.000001),
                output_cost_per_token: Some(0.000002),
                ..Default::default()
            },
        );

        let pricing = pricing::PricingService::new(litellm, openrouter);
        let mut msg = UnifiedMessage::new(
            "opencode",
            "accounts/fireworks/models/deepseek-v4-pro",
            "fireworks",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.123,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.123);
    }

    #[test]
    fn test_apply_pricing_if_available_prefers_provider_specific_exact_match_over_plain_exact() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gemini-2.5-pro".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                cache_creation_input_token_cost: None,
                ..Default::default()
            },
        );

        let mut openrouter = HashMap::new();
        openrouter.insert(
            "google/gemini-2.5-pro".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                cache_creation_input_token_cost: Some(0.01),
                ..Default::default()
            },
        );

        let pricing = pricing::PricingService::new(litellm, openrouter);

        let mut msg = UnifiedMessage::new(
            "opencode",
            "gemini-2.5-pro",
            "google",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 3,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.05);
    }

    #[test]
    fn test_apply_pricing_if_available_normalizes_openai_codex_provider() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-5.2-preview".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );
        litellm.insert(
            "google/gpt-5.2-preview-max".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.1),
                output_cost_per_token: Some(0.2),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "openclaw",
            "gpt-5.2",
            "openai-codex",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.2);
    }

    #[test]
    fn test_apply_pricing_if_available_prices_claude_code_gpt_5_3_codex() {
        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());

        let mut msg = UnifiedMessage::new(
            "claude",
            "gpt-5.3-codex",
            "openai",
            "session-1",
            1_776_000_000_000,
            TokenBreakdown {
                input: 1_000_000,
                output: 100_000,
                cache_read: 50_000,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        let expected = 1.75 + 1.4 + 0.00875;
        assert!((msg.cost - expected).abs() < 1e-12);
    }

    #[test]
    fn test_apply_pricing_if_available_prices_minimax_m3_bare_id_via_alias() {
        // #935: routers report MiniMax M3 as the bare lowercase id `minimax-m3`,
        // which is not a key in any dataset. When the session record carries no
        // usable provider hint — parsers emit `unknown` for an absent provider
        // and `normalize_provider_hint` drops it — nothing pins the lookup to
        // MiniMax's catalog, so the bare id falls through to model-part/fuzzy
        // matching over every row whose model part is `minimax-m3`.
        //
        // models.dev publishes that model part under dozens of third parties,
        // several of them at 0.0/0.0 (`kenari/minimax-m3` and
        // `nvidia/minimaxai/minimax-m3` both do today). Electing one of those
        // prices real usage at exactly $0 — which is what "pricing missing"
        // in #935 looks like from the user's side, since a row of explicit
        // zeros still counts as "priced" downstream. The alias must pin the
        // canonical first-party `minimax/MiniMax-M3` key instead.
        let mut litellm = HashMap::new();
        // Real first-party rates: $0.60 input / $2.40 output / $0.12 cache read
        // per million tokens. The cache-read rate is part of the published
        // first-party row, so the fixture carries it too — otherwise the guard
        // below would pass while cached input silently priced at $0.
        litellm.insert(
            "minimax/MiniMax-M3".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(6e-7),
                output_cost_per_token: Some(2.4e-6),
                cache_read_input_token_cost: Some(1.2e-7),
                ..Default::default()
            },
        );
        // The hosted reseller row that ships alongside it, at a deliberately
        // far-apart rate so electing it could not be mistaken for the
        // first-party result.
        litellm.insert(
            "fireworks_ai/minimax-m3".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(3e-5),
                output_cost_per_token: Some(1.2e-4),
                ..Default::default()
            },
        );
        // The zero-cost third-party row that the bare id actually elects
        // without the alias.
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "kenari/minimax-m3".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new_with_custom_and_models_dev(
            Default::default(),
            litellm,
            HashMap::new(),
            models_dev,
        );

        // Fixture guards: both competing rows must really be in the dataset and
        // resolvable, or this test would pass for the wrong reason.
        let competing_zero = pricing
            .lookup_with_source_and_provider("kenari/minimax-m3", None, None)
            .expect("competing zero-cost models.dev row must be present");
        assert_eq!(competing_zero.matched_key, "kenari/minimax-m3");
        assert_eq!(competing_zero.pricing.input_cost_per_token, Some(0.0));
        let competing_hosted = pricing
            .lookup_with_source_and_provider("minimax-m3", None, Some("fireworks_ai"))
            .expect("competing fireworks_ai row must resolve under its own hint");
        assert_eq!(competing_hosted.matched_key, "fireworks_ai/minimax-m3");

        // The behavior the alias exists to guarantee: the bare id resolves to
        // the canonical first-party key, not to either competitor.
        let resolved = pricing
            .lookup_with_source_and_provider("minimax-m3", None, Some("unknown"))
            .expect("bare `minimax-m3` must resolve");
        assert_eq!(resolved.matched_key, "minimax/MiniMax-M3");
        assert_eq!(resolved.source, "LiteLLM");

        let mut msg = UnifiedMessage::new(
            "ollama",
            "minimax-m3",
            "unknown",
            "session-1",
            1_776_000_000_000,
            TokenBreakdown {
                input: 1_000_000,
                output: 100_000,
                cache_read: 500_000,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        // First-party: 1_000_000 * 6e-7 + 100_000 * 2.4e-6 + 500_000 * 1.2e-7
        // = 0.9. The zero-cost row would give 0.0; the fireworks row, which
        // publishes no cache-read rate, would give 42.0.
        let expected = 1_000_000.0 * 6e-7 + 100_000.0 * 2.4e-6 + 500_000.0 * 1.2e-7;
        assert!(
            (msg.cost - expected).abs() < 1e-12,
            "expected first-party minimax/MiniMax-M3 cost {expected}, got {}",
            msg.cost
        );
    }

    #[test]
    fn test_apply_pricing_if_available_prices_claude_code_minimax_model() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "minimax/minimax-m2.1".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());

        let mut msg = UnifiedMessage::new(
            "claude",
            "MiniMax-M2.1",
            "minimax",
            "session-1",
            1_776_000_000_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        assert_eq!(msg.cost, 0.2);
    }

    #[test]
    fn test_apply_pricing_if_available_prices_kimi_k2p6_alias() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "moonshotai/kimi-k2.6".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(9.5e-7),
                output_cost_per_token: Some(0.000004),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(HashMap::new(), openrouter);

        let mut msg = UnifiedMessage::new(
            "kimi",
            "k2p6",
            "kimi-for-coding",
            "session-1",
            1_776_000_000_000,
            TokenBreakdown {
                input: 1_000_000,
                output: 250_000,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(&pricing));

        let expected = 1_000_000.0 * 9.5e-7 + 250_000.0 * 0.000004;
        assert!((msg.cost - expected).abs() < 1e-12);
        assert!(msg.cost > 0.0);
    }

    #[test]
    fn test_select_local_parse_pricing_prefers_fresh_service_for_new_models() {
        let mut fresh_litellm = HashMap::new();
        fresh_litellm.insert(
            "gpt-5.4".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.000002),
                output_cost_per_token: Some(0.00001),
                ..Default::default()
            },
        );
        let fresh = Arc::new(pricing::PricingService::new(fresh_litellm, HashMap::new()));
        let stale = pricing::PricingService::new(HashMap::new(), HashMap::new());
        let selected = select_local_parse_pricing(Ok(Arc::clone(&fresh)), || Some(stale)).unwrap();

        let mut msg = UnifiedMessage::new(
            "opencode",
            "gpt-5.4",
            "openai",
            "session-1",
            1_733_011_200_000,
            TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );

        apply_pricing_if_available(&mut msg, Some(selected.as_ref()));

        assert!(msg.cost > 0.0);
    }

    #[test]
    fn test_select_local_parse_pricing_falls_back_to_stale_cache_on_fetch_error() {
        let mut stale_litellm = HashMap::new();
        stale_litellm.insert(
            "gpt-5.2".into(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.00000175),
                output_cost_per_token: Some(0.000014),
                ..Default::default()
            },
        );
        let stale = pricing::PricingService::new(stale_litellm, HashMap::new());

        let selected =
            select_local_parse_pricing(Err("network failed".to_string()), || Some(stale)).unwrap();

        assert!(selected.lookup_with_source("gpt-5.2", None).is_some());
    }

    #[test]
    fn test_select_local_parse_pricing_does_not_evaluate_stale_fallback_on_fresh_success() {
        let fresh = Arc::new(pricing::PricingService::new(HashMap::new(), HashMap::new()));
        let mut stale_called = false;

        let selected = select_local_parse_pricing(Ok(Arc::clone(&fresh)), || {
            stale_called = true;
            None
        })
        .unwrap();

        assert!(Arc::ptr_eq(&selected, &fresh));
        assert!(!stale_called);
    }

    #[test]
    fn test_dedupe_latest_trae_messages_keeps_latest_timestamp_for_session() {
        let messages = vec![
            make_trae_message(
                "session-stable",
                1_700_000_002_000,
                Some("trae:session-stable:1_700_000_002"),
                0.2,
            ),
            make_trae_message(
                "session-stable",
                1_700_000_003_000,
                Some("trae:session-stable:1_700_000_003"),
                0.3,
            ),
            make_trae_message(
                "session-other",
                1_700_000_001_000,
                Some("trae:session-other:1_700_000_001"),
                0.1,
            ),
        ];

        let deduped = dedupe_latest_trae_messages(messages);

        assert_eq!(deduped.len(), 2);
        let stable = deduped
            .iter()
            .find(|msg| msg.session_id == "session-stable")
            .expect("session-stable should remain after dedupe");
        assert_eq!(stable.timestamp, 1_700_000_003_000);
        assert_eq!(stable.cost, 0.3);
        assert_eq!(
            stable.dedup_key.as_deref(),
            Some("trae:session-stable:1_700_000_003")
        );
    }

    #[test]
    fn test_dedupe_latest_trae_messages_tiebreaks_by_dedup_key() {
        let messages = vec![
            make_trae_message(
                "session-stable",
                1_700_000_010_000,
                Some("dedupe-key-a"),
                0.2,
            ),
            make_trae_message(
                "session-stable",
                1_700_000_010_000,
                Some("dedupe-key-z"),
                0.4,
            ),
            make_trae_message(
                "session-stable",
                1_700_000_009_000,
                Some("dedupe-key-m"),
                0.1,
            ),
        ];

        let deduped = dedupe_latest_trae_messages(messages);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].timestamp, 1_700_000_010_000);
        assert_eq!(deduped[0].dedup_key.as_deref(), Some("dedupe-key-z"));
        assert_eq!(deduped[0].cost, 0.4);
    }

    #[test]
    fn test_parse_all_messages_with_pricing_keeps_gateway_message_under_synthetic_filter() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let message_dir = temp_dir
            .path()
            .join(".local/share/opencode/storage/message/project-1");
        std::fs::create_dir_all(&message_dir).unwrap();
        std::fs::write(
            message_dir.join("msg_001.json"),
            r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"hf:deepseek-ai/DeepSeek-V3-0324","providerID":"unknown","cost":0,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
        )
        .unwrap();

        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());
        let messages = parse_all_messages_with_pricing_with_cache_policy(
            temp_dir.path().to_str().unwrap(),
            &["synthetic".to_string()],
            Some(&pricing),
            false,
            &scanner::ScannerSettings::default(),
            SourceCachePolicy::InMemory,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "opencode");
        assert_eq!(messages[0].model_id, "deepseek-v3-0324");
        assert_eq!(messages[0].provider_id, "synthetic");
    }

    #[test]
    fn test_parse_local_clients_preserves_gateway_message_client_counts() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let message_dir = temp_dir
            .path()
            .join(".local/share/opencode/storage/message/project-1");
        std::fs::create_dir_all(&message_dir).unwrap();
        std::fs::write(
            message_dir.join("msg_001.json"),
            r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"accounts/fireworks/models/deepseek-v3-0324","providerID":"fireworks","cost":0,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
        )
        .unwrap();

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["opencode".to_string(), "synthetic".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::OpenCode), 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].client, "opencode");
        assert_eq!(parsed.messages[0].model_id, "deepseek-v3-0324");
        // opencode now canonicalizes the provider segment like every other
        // session parser, so the raw "fireworks" gateway id resolves to its
        // canonical "fireworks_ai" tag.
        assert_eq!(parsed.messages[0].provider_id, "fireworks_ai");
    }

    #[test]
    fn test_parse_all_messages_fireworks_provider_kept_under_synthetic_only_filter() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let message_dir = temp_dir
            .path()
            .join(".local/share/opencode/storage/message/project-1");
        std::fs::create_dir_all(&message_dir).unwrap();
        std::fs::write(
            message_dir.join("msg_001.json"),
            r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"accounts/fireworks/models/deepseek-v3-0324","providerID":"fireworks","cost":0.1,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
        )
        .unwrap();

        let pricing = pricing::PricingService::new(HashMap::new(), HashMap::new());
        let messages = parse_all_messages_with_pricing_with_cache_policy(
            temp_dir.path().to_str().unwrap(),
            &["synthetic".to_string()],
            Some(&pricing),
            false,
            &scanner::ScannerSettings::default(),
            SourceCachePolicy::InMemory,
        );

        assert_eq!(
            messages.len(),
            1,
            "fireworks gateway message must not be dropped when filtering for synthetic"
        );
        assert_eq!(messages[0].client, "opencode");
        assert_eq!(messages[0].model_id, "deepseek-v3-0324");
        // Provider is canonicalized by the opencode parser (fireworks -> fireworks_ai).
        assert_eq!(messages[0].provider_id, "fireworks_ai");
    }

    #[test]
    fn test_parse_local_clients_fireworks_provider_kept_under_synthetic_only_filter() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let message_dir = temp_dir
            .path()
            .join(".local/share/opencode/storage/message/project-1");
        std::fs::create_dir_all(&message_dir).unwrap();
        std::fs::write(
            message_dir.join("msg_001.json"),
            r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"accounts/fireworks/models/deepseek-v3-0324","providerID":"fireworks","cost":0.1,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
        )
        .unwrap();

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["synthetic".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();

        assert_eq!(
            parsed.messages.len(),
            1,
            "fireworks gateway message must not be dropped when filtering for synthetic only"
        );
        assert_eq!(parsed.messages[0].client, "opencode");
        assert_eq!(parsed.messages[0].model_id, "deepseek-v3-0324");
        // Provider is canonicalized by the opencode parser (fireworks -> fireworks_ai).
        assert_eq!(parsed.messages[0].provider_id, "fireworks_ai");
    }

    #[test]
    fn test_opencode_embedded_cost_survives_repricing_while_missing_cost_reprices() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let message_dir = temp_dir
            .path()
            .join(".local/share/opencode/storage/message/project-1");
        std::fs::create_dir_all(&message_dir).unwrap();
        std::fs::write(
            message_dir.join("msg_reported.json"),
            r#"{"id":"msg-reported","sessionID":"session-1","role":"assistant","modelID":"gpt-4o","providerID":"openai","cost":0.05,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
        )
        .unwrap();
        std::fs::write(
            message_dir.join("msg_missing.json"),
            r#"{"id":"msg-missing","sessionID":"session-1","role":"assistant","modelID":"gpt-4o","providerID":"openai","tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011201000}}"#,
        )
        .unwrap();

        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-4o".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let messages = parse_all_messages_with_pricing_with_cache_policy(
            temp_dir.path().to_str().unwrap(),
            &["opencode".to_string()],
            Some(&pricing),
            false,
            &scanner::ScannerSettings::default(),
            SourceCachePolicy::InMemory,
        );

        let embedded = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("msg-reported"))
            .expect("embedded-cost message should parse");
        let missing = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("msg-missing"))
            .expect("missing-cost message should parse");
        assert_eq!(
            embedded.cost, 0.05,
            "OpenCode computes cost at request time; the embedded value must not be overwritten by LiteLLM repricing"
        );
        assert_eq!(embedded.cost_source, crate::CostSource::ProviderReported);
        assert_eq!(missing.cost, 0.2);
        assert_eq!(missing.cost_source, crate::CostSource::Estimated);
    }

    #[test]
    fn test_gjc_explicit_zero_cost_is_preserved_while_absent_cost_reprices() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let session_dir = temp_dir.path().join(".gjc/agent/sessions/project-1");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("session.jsonl"),
            r#"{"type":"session","id":"gjc_ses_cost","cwd":"/work/project-1"}
{"type":"message","id":"msg_zero","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1733011200000,"usage":{"input":10,"output":5,"cost":{"total":0.0}}}}
{"type":"message","id":"msg_absent","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1733011201000,"usage":{"input":10,"output":5}}}"#,
        )
        .unwrap();

        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-4o".to_string(),
            pricing::ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.02),
                ..Default::default()
            },
        );
        let pricing = pricing::PricingService::new(litellm, HashMap::new());
        let messages = parse_all_messages_with_pricing_with_cache_policy(
            temp_dir.path().to_str().unwrap(),
            &["gjc".to_string()],
            Some(&pricing),
            false,
            &scanner::ScannerSettings::default(),
            SourceCachePolicy::InMemory,
        );

        let explicit_zero = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("gjc_ses_cost:msg_zero"))
            .expect("explicit-zero message should parse");
        let absent = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("gjc_ses_cost:msg_absent"))
            .expect("absent-cost message should parse");
        assert_eq!(explicit_zero.cost, 0.0);
        assert_eq!(absent.cost, 0.2);
    }

    #[test]
    fn test_gjc_idless_replay_dedup_stable_across_ordinal_shift() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let session_dir = temp_dir.path().join(".gjc/agent/sessions/project-1");
        let child_dir = session_dir.join("session");
        std::fs::create_dir_all(&child_dir).unwrap();
        let assistant_line = r#"{"type":"message","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1733011200000,"usage":{"input":10,"output":5,"cost":{"total":0.03}}}}"#;
        std::fs::write(
            session_dir.join("session.jsonl"),
            format!(
                "{}\n{}\n",
                r#"{"type":"session","id":"gjc_ses_replay_idless","cwd":"/work/project-1"}"#,
                assistant_line
            ),
        )
        .unwrap();
        std::fs::write(
            child_dir.join("1-replay.jsonl"),
            format!(
                "{}\n{}\n{}\n",
                r#"{"type":"session","id":"gjc_ses_replay_idless","cwd":"/work/project-1"}"#,
                r#"{"type":"service_tier_change","tier":"pro"}"#,
                assistant_line
            ),
        )
        .unwrap();

        let messages = parse_all_messages_with_pricing_with_cache_policy(
            temp_dir.path().to_str().unwrap(),
            &["gjc".to_string()],
            None,
            false,
            &scanner::ScannerSettings::default(),
            SourceCachePolicy::InMemory,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.03);
    }

    #[test]
    fn test_parse_local_clients_honors_scanner_settings_opencode_db_paths() {
        // Regression guard: `parse_local_clients` used to call
        // `scan_all_clients_with_env_strategy`, which silently dropped
        // `options.scanner_settings`. Users with
        // `scanner.opencodeDbPaths` pointing at an OPENCODE_DB outside the
        // XDG data dir would see no rows through the clients/wrapped
        // command paths even though model/monthly/graph reports honored
        // the same config.
        let temp_dir = tempfile::TempDir::new().unwrap();
        // Deliberately do not create ~/.local/share/opencode so nothing
        // is auto-discoverable; the only db the scanner can find must
        // come from `scanner_settings`.
        let outside_dir = temp_dir.path().join("elsewhere");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let external_db = outside_dir.join("opencode.db");

        let conn = rusqlite::Connection::open(&external_db).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE message (
                 id TEXT PRIMARY KEY,
                 session_id TEXT NOT NULL,
                 data TEXT NOT NULL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "ext-msg-1",
                "ext-session",
                r#"{
                    "role": "assistant",
                    "modelID": "claude-sonnet-4",
                    "providerID": "anthropic",
                    "tokens": { "input": 42, "output": 7, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
                    "time": { "created": 1700000000000.0 }
                }"#
            ],
        )
        .unwrap();
        drop(conn);

        // Without scanner_settings: no rows (nothing auto-discoverable).
        let parsed_default = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["opencode".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();
        assert_eq!(parsed_default.counts.get(ClientId::OpenCode), 0);
        assert!(parsed_default.messages.is_empty());

        // With scanner_settings pointing at the external db: the user
        // row must show up.
        let parsed_with_settings = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["opencode".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                opencode_db_paths: vec![external_db.clone()],
                ..Default::default()
            },
        })
        .unwrap();
        assert_eq!(
            parsed_with_settings.counts.get(ClientId::OpenCode),
            1,
            "scanner.opencodeDbPaths must reach the parse_local_clients path"
        );
        assert_eq!(parsed_with_settings.messages.len(), 1);
        assert_eq!(parsed_with_settings.messages[0].client, "opencode");
        assert_eq!(parsed_with_settings.messages[0].model_id, "claude-sonnet-4");
    }

    #[test]
    fn test_parse_local_clients_honors_devin_cli_extra_scan_paths() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let external_dir = temp_dir.path().join("imports/devin/profile");
        std::fs::create_dir_all(&external_dir).unwrap();
        let external_db = external_dir.join("sessions.db");
        let conn = rusqlite::Connection::open(&external_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 working_directory TEXT NOT NULL,
                 backend_type TEXT NOT NULL,
                 model TEXT NOT NULL,
                 title TEXT,
                 agent_mode TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 last_activity_at INTEGER NOT NULL
             );
             CREATE TABLE message_nodes (
                 row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 node_id INTEGER NOT NULL,
                 parent_node_id INTEGER,
                 chat_message TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 metadata TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, working_directory, backend_type, model, agent_mode, created_at, last_activity_at) VALUES ('external-session', '/tmp/project', 'windsurf', 'gpt-5', 'accept-edits', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('external-session', 1, ?1, 1700000000)",
            [r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":42,"output_tokens":7}}}"#],
        )
        .unwrap();
        drop(conn);

        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert("devin-cli".to_string(), vec![external_dir]);
        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["devin-cli".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                extra_scan_paths,
                ..Default::default()
            },
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::DevinCli), 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].client, "devin-cli");
        assert_eq!(parsed.messages[0].session_id, "external-session");
    }

    #[test]
    fn test_parse_local_clients_devin_zero_cli_usage_does_not_suppress_desktop() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join(".local/share/devin/cli/sessions.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 working_directory TEXT NOT NULL,
                 backend_type TEXT NOT NULL,
                 model TEXT NOT NULL,
                 title TEXT,
                 agent_mode TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 last_activity_at INTEGER NOT NULL
             );
             CREATE TABLE message_nodes (
                 row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 node_id INTEGER NOT NULL,
                 parent_node_id INTEGER,
                 chat_message TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 metadata TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, working_directory, backend_type, model, title, agent_mode, created_at, last_activity_at) VALUES ('cli-session', '/tmp/project', 'windsurf', 'gpt-5', 'Desktop task', 'accept-edits', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('cli-session', 1, ?1, 1700000000)",
            [r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":0,"output_tokens":0}}}"#],
        )
        .unwrap();
        drop(conn);

        let desktop_dir = temp_dir
            .path()
            .join("Library/Application Support/Devin/User/acp-events");
        std::fs::create_dir_all(&desktop_dir).unwrap();
        std::fs::write(
            desktop_dir.join("desktop-file.ndjson"),
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"Desktop task"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":20,"cognition.ai/cachedReadTokens":10}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["devin-cli".to_string(), "devin-desktop".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::DevinCli), 0);
        assert_eq!(parsed.counts.get(ClientId::DevinDesktop), 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].client, "devin-desktop");
        assert_eq!(parsed.messages[0].session_id, "cli-session");
        assert_eq!(parsed.messages[0].model_id, "gpt-5");
        assert_eq!(parsed.messages[0].input, 90);
        assert_eq!(parsed.messages[0].cache_read, 10);
    }

    #[test]
    fn test_parse_local_clients_devin_nonzero_cli_usage_dedups_desktop_row_but_keeps_raw_count() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join(".local/share/devin/cli/sessions.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 working_directory TEXT NOT NULL,
                 backend_type TEXT NOT NULL,
                 model TEXT NOT NULL,
                 title TEXT,
                 agent_mode TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 last_activity_at INTEGER NOT NULL
             );
             CREATE TABLE message_nodes (
                 row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 node_id INTEGER NOT NULL,
                 parent_node_id INTEGER,
                 chat_message TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 metadata TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, working_directory, backend_type, model, title, agent_mode, created_at, last_activity_at) VALUES ('cli-session', '/tmp/project', 'windsurf', 'gpt-5', 'Desktop task', 'accept-edits', 1, 1)",
            [],
        )
        .unwrap();
        // Unlike the zero-usage regression test above, this CLI row carries
        // real attributable usage, so it must NOT be filtered by
        // `parse_devin_cli_sqlite`'s zero-metric guard. That means its
        // session id lands in `cli_session_ids`, which is exactly the
        // condition needed to exercise the dedup filter against the
        // matching Desktop NDJSON session.
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('cli-session', 1, ?1, 1700000000)",
            [r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":50,"output_tokens":25}}}"#],
        )
        .unwrap();
        drop(conn);

        let desktop_dir = temp_dir
            .path()
            .join("Library/Application Support/Devin/User/acp-events");
        std::fs::create_dir_all(&desktop_dir).unwrap();
        std::fs::write(
            desktop_dir.join("desktop-file.ndjson"),
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"Desktop task"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":20,"cognition.ai/cachedReadTokens":10}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["devin-cli".to_string(), "devin-desktop".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();

        // The Desktop row shares its resolved session id with the CLI row,
        // so it must be deduped out of `messages` and attributed to
        // devin-cli instead.
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].client, "devin-cli");
        assert_eq!(parsed.messages[0].session_id, "cli-session");

        // But the `clients` command count must still reflect the raw,
        // pre-dedup Desktop discovery so Desktop usage doesn't appear to
        // vanish when it overlaps with a CLI session.
        assert_eq!(parsed.counts.get(ClientId::DevinCli), 1);
        assert!(parsed.counts.get(ClientId::DevinDesktop) > 0);
    }

    #[test]
    fn test_parse_local_clients_desktop_uses_configured_cli_lookup_without_cli_usage() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let external_dir = temp_dir.path().join("imports/devin/profile");
        std::fs::create_dir_all(&external_dir).unwrap();
        let external_db = external_dir.join("sessions.db");
        let conn = rusqlite::Connection::open(&external_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 title TEXT,
                 model TEXT,
                 working_directory TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, title, model, working_directory) VALUES ('external-session', 'External desktop task', 'claude-sonnet-4', '/tmp/external-project')",
            [],
        )
        .unwrap();
        drop(conn);

        let desktop_dir = temp_dir
            .path()
            .join("Library/Application Support/Devin/User/acp-events");
        std::fs::create_dir_all(&desktop_dir).unwrap();
        std::fs::write(
            desktop_dir.join("desktop-file.ndjson"),
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"External desktop task"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":20}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert("devin-cli".to_string(), vec![external_dir]);
        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["devin-desktop".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                extra_scan_paths,
                ..Default::default()
            },
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::DevinCli), 0);
        assert_eq!(parsed.counts.get(ClientId::DevinDesktop), 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].client, "devin-desktop");
        assert_eq!(parsed.messages[0].session_id, "external-session");
        assert_eq!(parsed.messages[0].model_id, "claude-sonnet-4");
        assert_eq!(
            parsed.messages[0].workspace_key.as_deref(),
            Some("/tmp/external-project")
        );
    }

    #[test]
    fn test_devin_desktop_lookup_cache_separates_database_snapshots() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("sessions.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 title TEXT,
                 model TEXT,
                 working_directory TEXT
             );
             INSERT INTO sessions (id, title, model, working_directory)
             VALUES ('cli-session', 'Snapshot task', 'gpt-5', '/tmp/project');",
        )
        .unwrap();
        drop(conn);

        let desktop_path = temp_dir.path().join("desktop-file.ndjson");
        std::fs::write(
            &desktop_path,
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"Snapshot task"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":20}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let first_fingerprint =
            match message_cache::SourceFingerprint::check_devin_desktop_path_samples_only(
                &desktop_path,
                std::slice::from_ref(&db_path),
                None,
            )
            .unwrap()
            {
                message_cache::FingerprintStatus::Changed(fingerprint) => fingerprint,
                message_cache::FingerprintStatus::Unchanged => {
                    panic!("an uncached Desktop source must build a fingerprint")
                }
            };
        let lookup_cache = std::sync::Mutex::new(HashMap::new());
        let first_cell = super::devin_desktop_lookup_cell_for_snapshot(
            &lookup_cache,
            std::slice::from_ref(&db_path),
            &first_fingerprint,
        );
        let first_lookup = first_cell.get_or_init(|| {
            crate::sessions::devin::load_devin_desktop_session_lookup(std::slice::from_ref(
                &db_path,
            ))
        });
        let first_messages = crate::sessions::devin::parse_devin_desktop_ndjson_with_lookup(
            &desktop_path,
            first_lookup,
        );
        assert_eq!(first_messages[0].model_id, "gpt-5");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE sessions SET model = 'claude-sonnet-4' WHERE id = 'cli-session'",
            [],
        )
        .unwrap();
        drop(conn);

        let second_fingerprint =
            match message_cache::SourceFingerprint::check_devin_desktop_path_samples_only(
                &desktop_path,
                std::slice::from_ref(&db_path),
                None,
            )
            .unwrap()
            {
                message_cache::FingerprintStatus::Changed(fingerprint) => fingerprint,
                message_cache::FingerprintStatus::Unchanged => {
                    panic!("an uncached Desktop source must build a fingerprint")
                }
            };
        assert_ne!(
            first_fingerprint.related_files,
            second_fingerprint.related_files
        );

        let second_cell = super::devin_desktop_lookup_cell_for_snapshot(
            &lookup_cache,
            std::slice::from_ref(&db_path),
            &second_fingerprint,
        );
        assert!(
            !Arc::ptr_eq(&first_cell, &second_cell),
            "different database snapshots must not share a lookup cell"
        );
        let second_lookup = second_cell.get_or_init(|| {
            crate::sessions::devin::load_devin_desktop_session_lookup(std::slice::from_ref(
                &db_path,
            ))
        });
        let second_messages = crate::sessions::devin::parse_devin_desktop_ndjson_with_lookup(
            &desktop_path,
            second_lookup,
        );
        assert_eq!(second_messages[0].model_id, "claude-sonnet-4");
        assert_eq!(lookup_cache.lock().unwrap().len(), 2);
    }

    #[test]
    fn test_parse_local_clients_honors_scanner_extra_scan_paths_for_hermes_profile_db() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let profile_dir = temp_dir.path().join("external-hermes/director_planning");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let profile_db = profile_dir.join("state.db");
        let conn = create_hermes_sqlite_db(&profile_db);
        insert_hermes_session(
            &conn,
            "hermes-extra-session",
            "claude-sonnet-4",
            2,
            100,
            25,
            0.07,
        );
        drop(conn);

        let parsed_default = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["hermes".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();
        assert_eq!(parsed_default.counts.get(ClientId::Hermes), 0);
        assert!(parsed_default.messages.is_empty());

        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert("hermes".to_string(), vec![profile_dir]);
        let parsed_with_settings = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["hermes".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                extra_scan_paths,
                ..Default::default()
            },
        })
        .unwrap();

        assert_eq!(parsed_with_settings.counts.get(ClientId::Hermes), 2);
        assert_eq!(parsed_with_settings.messages.len(), 1);
        assert_eq!(parsed_with_settings.messages[0].client, "hermes");
        assert_eq!(
            parsed_with_settings.messages[0].agent.as_deref(),
            Some("Hermes Agent")
        );
        assert_eq!(
            parsed_with_settings.messages[0].session_id,
            "hermes-extra-session"
        );
        assert_eq!(parsed_with_settings.messages[0].model_id, "claude-sonnet-4");
        assert_eq!(parsed_with_settings.messages[0].input, 100);
        assert_eq!(parsed_with_settings.messages[0].output, 25);
    }

    #[test]
    fn test_parse_local_clients_honors_scanner_extra_scan_paths_for_zed_threads_db() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let extra_threads_dir = temp_dir.path().join("custom-zed/threads");
        std::fs::create_dir_all(&extra_threads_dir).unwrap();
        let threads_db = extra_threads_dir.join("threads.db");
        let conn = create_zed_sqlite_db(&threads_db);
        insert_zed_thread(&conn, "zed-extra-thread", "claude-sonnet-4-5");
        drop(conn);

        let parsed_default = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["zed".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();
        assert_eq!(parsed_default.counts.get(ClientId::Zed), 0);
        assert!(parsed_default.messages.is_empty());

        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert("zed".to_string(), vec![extra_threads_dir]);
        let parsed_with_settings = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["zed".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                extra_scan_paths,
                ..Default::default()
            },
        })
        .unwrap();

        assert_eq!(parsed_with_settings.counts.get(ClientId::Zed), 1);
        assert_eq!(parsed_with_settings.messages.len(), 1);
        assert_eq!(parsed_with_settings.messages[0].client, "zed");
        assert_eq!(
            parsed_with_settings.messages[0].session_id,
            "zed-extra-thread"
        );
        assert_eq!(
            parsed_with_settings.messages[0].model_id,
            "claude-sonnet-4-5"
        );
        assert_eq!(parsed_with_settings.messages[0].input, 42);
        assert_eq!(parsed_with_settings.messages[0].output, 7);
    }

    #[test]
    fn test_submit_default_graph_includes_antigravity_cache_rows() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // Resolved rather than hardcoded: under an explicit home the config
        // root is `~/.config/tokscale` on Unix and
        // `%HOME%\AppData\Roaming\tokscale` on Windows, so the Unix spelling
        // put the fixture outside the tree the scan walks and the graph came
        // back empty.
        let sessions_dir = std::path::PathBuf::from(
            ClientId::Antigravity
                .data()
                .resolve_path_with_env_strategy(&temp_dir.path().to_string_lossy(), false),
        );
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(
            sessions_dir.join("ag-submit.jsonl"),
            r#"{"type":"usage","sessionId":"ag-submit","modelId":"model_placeholder_m84","timestamp":1711200000000,"input":12,"output":4,"cacheRead":2,"cacheWrite":0,"reasoning":1,"responseId":"resp-ag"}
"#,
        )
        .unwrap();

        let mut clients: Vec<String> = ClientId::iter()
            .filter(|client| client.submit_default())
            .map(|client| client.as_str().to_string())
            .collect();
        clients.push("synthetic".to_string());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let graph = rt
            .block_on(generate_graph_with_loaded_pricing(
                ReportOptions {
                    home_dir: Some(temp_dir.path().to_string_lossy().to_string()),
                    use_env_roots: false,
                    clients: Some(clients),
                    since: None,
                    until: None,
                    year: None,
                    group_by: GroupBy::default(),
                    worktree_rollup: WorktreeRollup::default(),
                    scanner_settings: scanner::ScannerSettings::default(),
                },
                None,
                GraphPricingRequirement::Lenient,
            ))
            .unwrap();

        assert_eq!(graph.summary.clients, vec!["antigravity"]);
        assert_eq!(graph.summary.models, vec!["gemini-3-flash-preview"]);
        assert_eq!(graph.summary.total_tokens, 19);
        assert_eq!(graph.contributions.len(), 1);
        assert_eq!(graph.contributions[0].clients[0].client, "antigravity");
        assert_eq!(
            graph.contributions[0].clients[0].model_id,
            "gemini-3-flash-preview"
        );
    }

    #[test]
    fn test_parse_local_clients_dedups_zed_threads_across_default_and_extra_dbs() {
        let temp_dir = tempfile::TempDir::new().unwrap();

        // Place threads.db at the default platform path so the scanner finds it
        // as `zed_db` AND we also pass it via extraScanPaths.
        let default_threads_dir = temp_dir.path().join(".local/share/zed/threads");
        std::fs::create_dir_all(&default_threads_dir).unwrap();
        let default_db = default_threads_dir.join("threads.db");
        let conn = create_zed_sqlite_db(&default_db);
        insert_zed_thread(&conn, "shared-zed-thread", "claude-sonnet-4-5");
        drop(conn);

        // Point extraScanPaths.zed at the same directory — dedup should prevent
        // the thread from appearing twice.
        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert("zed".to_string(), vec![default_threads_dir.clone()]);
        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["zed".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                extra_scan_paths,
                ..Default::default()
            },
        })
        .unwrap();

        // Should see exactly 1 message, not 2 (deduped by canonicalize).
        assert_eq!(parsed.counts.get(ClientId::Zed), 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].session_id, "shared-zed-thread");
    }

    #[test]
    fn test_parse_local_clients_zed_extra_scan_paths_nonexistent_dir_is_silent() {
        let temp_dir = tempfile::TempDir::new().unwrap();

        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert(
            "zed".to_string(),
            vec![temp_dir.path().join("does/not/exist")],
        );
        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["zed".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                extra_scan_paths,
                ..Default::default()
            },
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::Zed), 0);
        assert!(parsed.messages.is_empty());
    }

    #[test]
    fn test_parse_local_clients_dedups_hermes_sessions_across_default_and_extra_dbs() {
        let temp_dir = tempfile::TempDir::new().unwrap();

        let default_dir = temp_dir.path().join(".hermes");
        std::fs::create_dir_all(&default_dir).unwrap();
        let default_db = default_dir.join("state.db");
        let default_conn = create_hermes_sqlite_db(&default_db);
        insert_hermes_session(
            &default_conn,
            "shared-hermes-session",
            "claude-sonnet-4",
            2,
            100,
            25,
            0.07,
        );
        drop(default_conn);

        let profile_dir = temp_dir.path().join(".hermes/profiles/director_planning");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let profile_db = profile_dir.join("state.db");
        let profile_conn = create_hermes_sqlite_db(&profile_db);
        insert_hermes_session(
            &profile_conn,
            "shared-hermes-session",
            "claude-sonnet-4",
            9,
            999,
            999,
            9.99,
        );
        drop(profile_conn);

        let mut extra_scan_paths = std::collections::BTreeMap::new();
        extra_scan_paths.insert("hermes".to_string(), vec![profile_db]);
        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["hermes".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                extra_scan_paths,
                ..Default::default()
            },
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::Hermes), 2);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].session_id, "shared-hermes-session");
        assert_eq!(parsed.messages[0].input, 100);
        assert_eq!(parsed.messages[0].output, 25);
    }

    #[test]
    fn test_parse_local_clients_claude_filter_ignores_scanner_settings_opencode_db_paths() {
        // Regression guard for the scanner client-filter bypass: even
        // when `scanner.opencodeDbPaths` pins an external opencode db,
        // a `--clients claude` request must NOT pull in OpenCode rows.
        // Before the fix, the merge ran outside the OpenCode-enabled
        // guard so user-pinned dbs leaked through both `messages` and
        // `counts` (the latter is computed before the message-level
        // client filter, so even the post-filter pipeline could not
        // hide a leaked count).
        let temp_dir = tempfile::TempDir::new().unwrap();

        // Claude session: one assistant message, the only thing the
        // filter should accept.
        let claude_dir = temp_dir.path().join(".claude/projects/myproject");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("conversation.jsonl"),
            r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
"#,
        )
        .unwrap();

        // External opencode.db that the user has pinned via
        // scanner.opencodeDbPaths. Without the fix, this would leak
        // into the Claude-only result.
        let outside_dir = temp_dir.path().join("elsewhere");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let external_db = outside_dir.join("opencode.db");
        let conn = rusqlite::Connection::open(&external_db).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE message (
                 id TEXT PRIMARY KEY,
                 session_id TEXT NOT NULL,
                 data TEXT NOT NULL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "leaked-opencode",
                "should-not-show-up",
                r#"{
                    "role": "assistant",
                    "modelID": "claude-sonnet-4",
                    "providerID": "anthropic",
                    "tokens": { "input": 9999, "output": 9999, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
                    "time": { "created": 1700000000000.0 }
                }"#
            ],
        )
        .unwrap();
        drop(conn);

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["claude".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings {
                opencode_db_paths: vec![external_db.clone()],
                ..Default::default()
            },
        })
        .unwrap();

        assert_eq!(
            parsed.counts.get(ClientId::OpenCode),
            0,
            "OpenCode count must stay zero under a Claude-only filter even \
             when scanner.opencodeDbPaths is set"
        );
        assert_eq!(
            parsed.counts.get(ClientId::Claude),
            1,
            "Claude message must still be counted"
        );
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].client, "claude");
        assert!(
            parsed.messages.iter().all(|m| m.client != "opencode"),
            "no OpenCode messages may leak into a Claude-only result, got {:?}",
            parsed.messages
        );
    }

    #[test]
    fn test_parse_local_clients_claude_transcripts_count_only_usage_metadata() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let transcripts_dir = temp_dir.path().join(".claude/transcripts");
        std::fs::create_dir_all(&transcripts_dir).unwrap();
        std::fs::write(
            transcripts_dir.join("ses_123456789012345678901234567.jsonl"),
            r#"{"type":"user","timestamp":"2026-04-01T10:00:00.000Z","message":{"content":"Wrapped prompt"}}
{"type":"assistant","timestamp":"2026-04-01T10:00:01.000Z","requestId":"req_wrapper","message":{"id":"msg_wrapper","model":"claude-sonnet-4","usage":{"input_tokens":123,"output_tokens":45,"cache_read_input_tokens":67,"cache_creation_input_tokens":8}}}
"#,
        )
        .unwrap();
        std::fs::write(
            transcripts_dir.join("ses_765432109876543210987654321.jsonl"),
            r#"{"type":"user","timestamp":"2026-04-01T10:00:00.000Z","message":{"content":"Wrapped prompt"}}
{"type":"tool_use","timestamp":"2026-04-01T10:00:01.000Z","message":{"content":"Run tool"}}
{"type":"tool_result","timestamp":"2026-04-01T10:00:02.000Z","message":{"content":"Tool result"}}
"#,
        )
        .unwrap();

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["claude".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::Claude), 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].client, "claude");
        assert_eq!(
            parsed.messages[0].session_id,
            "ses_123456789012345678901234567"
        );
        assert_eq!(parsed.messages[0].model_id, "claude-sonnet-4");
        assert_eq!(parsed.messages[0].input, 123);
        assert_eq!(parsed.messages[0].output, 45);
        assert_eq!(parsed.messages[0].cache_read, 67);
        assert_eq!(parsed.messages[0].cache_write, 8);
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_refreshes_cc_mirror_provider_when_variant_metadata_changes() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let variant_dir = source_home.path().join(".cc-mirror/kimi-code");
            let config_dir = source_home.path().join("mirror-configs/kimi-code");
            let project_dir = config_dir.join("projects/project-one");
            std::fs::create_dir_all(&project_dir).unwrap();
            std::fs::create_dir_all(&variant_dir).unwrap();
            let variant_path = variant_dir.join("variant.json");
            std::fs::write(
                &variant_path,
                format!(
                    r#"{{"name":"kimi-code","provider":"kimi","configDir":{}}}"#,
                    paths::json_path_literal(&config_dir)
                ),
            )
            .unwrap();
            let session_path = project_dir.join("session.jsonl");
            std::fs::write(
                &session_path,
                r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
"#,
            )
            .unwrap();

            let first_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
            );
            assert_eq!(first_messages.len(), 1);
            assert_eq!(first_messages[0].client, "cc-mirror/kimi-code");
            assert_eq!(first_messages[0].provider_id, "kimi");

            std::fs::write(
                &variant_path,
                format!(
                    r#"{{"name":"kimi-code","provider":"minimax","configDir":{}}}"#,
                    paths::json_path_literal(&config_dir)
                ),
            )
            .unwrap();

            let refreshed_messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
            );
            assert_eq!(refreshed_messages.len(), 1);
            assert_eq!(refreshed_messages[0].client, "cc-mirror/kimi-code");
            assert_eq!(refreshed_messages[0].provider_id, "minimax");
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_all_messages_keeps_normal_claude_when_cc_mirror_points_at_claude_config() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());

        {
            let claude_dir = source_home.path().join(".claude");
            let project_dir = claude_dir.join("projects/project-one");
            std::fs::create_dir_all(&project_dir).unwrap();
            let session_path = project_dir.join("session.jsonl");
            std::fs::write(
                &session_path,
                r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
"#,
            )
            .unwrap();

            let variant_dir = source_home.path().join(".cc-mirror/plain-mirror");
            std::fs::create_dir_all(&variant_dir).unwrap();
            std::fs::write(
                variant_dir.join("variant.json"),
                format!(
                    r#"{{"name":"plain-mirror","provider":"mirror","configDir":{}}}"#,
                    paths::json_path_literal(&claude_dir)
                ),
            )
            .unwrap();

            let messages = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &["claude".to_string()],
                None,
            );
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].client, "claude");
        }
    }

    #[test]
    fn test_parse_local_clients_amp_partial_ledger_recovers_message_fallback_day() {
        use chrono::TimeZone;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let amp_dir = temp_dir.path().join(".local/share/amp/threads");
        std::fs::create_dir_all(&amp_dir).unwrap();

        let thread_created = chrono::DateTime::parse_from_rfc3339("2026-04-04T12:00:00Z")
            .unwrap()
            .timestamp_millis();
        let ledger_timestamp = chrono::DateTime::parse_from_rfc3339("2026-04-08T12:00:00Z")
            .unwrap()
            .timestamp_millis();

        let thread = format!(
            r#"{{
                "id": "thread-amp-gap",
                "created": {thread_created},
                "usageLedger": {{
                    "events": [
                        {{
                            "timestamp": "2026-04-08T12:00:00Z",
                            "model": "claude-sonnet-4-0",
                            "credits": 0.75,
                            "tokens": {{ "input": 100, "output": 20 }}
                        }}
                    ]
                }},
                "messages": [
                    {{
                        "role": "assistant",
                        "messageId": 1,
                        "usage": {{
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 100,
                            "outputTokens": 20,
                            "credits": 0.75
                        }}
                    }},
                    {{
                        "role": "assistant",
                        "messageId": 2,
                        "usage": {{
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 50,
                            "outputTokens": 10,
                            "credits": 0.40
                        }}
                    }}
                ]
            }}"#
        );
        std::fs::write(amp_dir.join("T-thread-amp-gap.json"), thread).unwrap();

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["amp".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();

        assert_eq!(parsed.counts.get(ClientId::Amp), 2);
        assert_eq!(parsed.messages.len(), 2);

        let dates: HashSet<String> = parsed.messages.iter().map(|msg| msg.date.clone()).collect();
        let local_date = |timestamp_ms: i64| {
            chrono::Local
                .timestamp_millis_opt(timestamp_ms)
                .single()
                .unwrap()
                .format("%Y-%m-%d")
                .to_string()
        };
        assert!(dates.contains(&local_date(thread_created + 2000)));
        assert!(dates.contains(&local_date(ledger_timestamp)));
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_forked_parent_and_rlm_child_are_counted_once() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions = source_home.path().join(".prime/agent/sessions");
        let child_dir = source_home
            .path()
            .join(".prime/agent/session-artifacts/z-original/sub-child");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();

        let original_path = sessions.join("z-original.jsonl");
        std::fs::write(
            sessions.join("a-fork.jsonl"),
            format!(
                r#"{{"type":"session","version":3,"id":"fork","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":0}}
{{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}}}}}}
{{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{{"input":30,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":40}},"aggregateUsage":{{"input":130,"output":60,"cacheRead":20,"cacheWrite":10,"totalTokens":220}},"origin":"spawn_task"}}
{{"type":"child_usage_attributed","id":"usage-2","parentId":"usage-1","timestamp":"2026-08-08T00:00:03.000Z","targetId":"parent","childUsage":{{"input":20,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":30}},"aggregateUsage":{{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}},"origin":"spawn_task"}}
"#,
                paths::json_path_literal(&original_path)
            ),
        )
        .unwrap();
        std::fs::write(
            &original_path,
            r#"{"type":"session","version":3,"id":"original","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":30,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":40},"aggregateUsage":{"input":130,"output":60,"cacheRead":20,"cacheWrite":10,"totalTokens":220},"origin":"spawn_task"}
{"type":"child_usage_attributed","id":"usage-2","parentId":"usage-1","timestamp":"2026-08-08T00:00:03.000Z","targetId":"parent","childUsage":{"input":20,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":30},"aggregateUsage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        std::fs::write(
            child_dir.join("child.jsonl"),
            format!(
                r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:01.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message-1","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-response-1","usage":{{"input":30,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":40}}}}}}
{{"type":"message","id":"child-message-2","parentId":"child-message-1","timestamp":"2026-08-08T00:00:03.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-response-2","usage":{{"input":20,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":30}}}}}}
"#,
                paths::json_path_literal(&original_path)
            ),
        )
        .unwrap();

        let clients = ["prime-agent".to_string()];
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let cold =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        let cold_decode_calls = sessions::prime_agent::transcript_decode_call_counts();
        assert_eq!(cold_decode_calls, (3, 0));

        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            cold_decode_calls,
            "an unchanged warm scan must decode neither messages nor accounting"
        );

        for messages in [cold, warm] {
            assert_eq!(messages.len(), 3);
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                150
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.output)
                    .sum::<i64>(),
                70
            );
        }

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(source_home.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(clients.to_vec()),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(parsed.counts.get(ClientId::PrimeAgent), 3);
        assert_eq!(
            parsed
                .messages
                .iter()
                .map(|message| message.input)
                .sum::<i64>(),
            150
        );
        assert_eq!(
            parsed
                .messages
                .iter()
                .map(|message| message.output)
                .sum::<i64>(),
            70
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_cold_and_warm_reject_damaged_nested_attribution_usage() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        let child_dir = source_home
            .path()
            .join(".prime/agent/session-artifacts/root/sub-child");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        let root_path = sessions_dir.join("root.jsonl");
        std::fs::write(
            &root_path,
            r#"{"type":"session","version":3,"id":"root","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":100,"output":0}}}
{"type":"child_usage_attributed","id":"usage-1","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"out�put":999},"aggregateUsage":{"input":100,"output":0}}
"#,
        )
        .unwrap();
        std::fs::write(
            child_dir.join("child.jsonl"),
            format!(
                r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:01.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-response","usage":{{"input":50,"output":0}}}}}}
"#,
                paths::json_path_literal(&root_path)
            ),
        )
        .unwrap();

        let clients = ["prime-agent".to_string()];
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let cold =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        let cold_decode_calls = sessions::prime_agent::transcript_decode_call_counts();
        assert_eq!(cold_decode_calls, (2, 0));

        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            cold_decode_calls,
            "the unchanged warm scan must reuse the safely rejected accounting record"
        );

        for messages in [cold, warm] {
            assert_eq!(messages.len(), 2);
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                150,
                "damaged child usage must not subtract the matching child from the parent"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_rejects_damaged_child_lineage_and_timestamp_cold_and_warm() {
        for damage in ["parent-value", "timestamp-value"] {
            let cache_home = tempfile::TempDir::new().unwrap();
            let source_home = tempfile::TempDir::new().unwrap();
            let _cache_env = redirect_cache_home(cache_home.path());
            let sessions_dir = source_home.path().join(".prime/agent/sessions");
            let child_dir = source_home
                .path()
                .join(".prime/agent/session-artifacts/root/sub-child");
            std::fs::create_dir_all(&sessions_dir).unwrap();
            std::fs::create_dir_all(&child_dir).unwrap();
            let root_path = sessions_dir.join("root.jsonl");
            std::fs::write(
                &root_path,
                r#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":0}}}
{"type":"child_usage_attributed","id":"usage-1","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"output":0},"aggregateUsage":{"input":150,"output":0}}
"#,
            )
            .unwrap();

            let child_path = child_dir.join(format!("{damage}.jsonl"));
            let clean_parent = paths::json_path_literal(&root_path);
            let mut child = Vec::new();
            if damage == "parent-value" {
                child.extend_from_slice(b"{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"cwd\":\"/tmp/project\",\"parentSession\":");
                child.extend_from_slice(
                    clean_parent
                        .as_bytes()
                        .get(0..clean_parent.len() - 1)
                        .unwrap(),
                );
                child.extend_from_slice(b"\xff\",\"rlmDepth\":1}\n");
            } else {
                child.extend_from_slice(format!("{{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"cwd\":\"/tmp/project\",\"parentSession\":{clean_parent},\"rlmDepth\":1}}\n").as_bytes());
            }
            if damage == "timestamp-value" {
                child.extend_from_slice(b"{\"type\":\"message\",\"id\":\"child-message\",\"timestamp\":\"2026-08-08T00:00:0\xffZ\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":50,\"output\":0}}}\n");
            } else {
                child.extend_from_slice(b"{\"type\":\"message\",\"id\":\"child-message\",\"timestamp\":\"2026-08-08T00:00:02.000Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":50,\"output\":0}}}\n");
            }
            std::fs::write(child_path, child).unwrap();

            let clients = ["prime-agent".to_string()];
            sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
            let cold = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &clients,
                None,
            );
            let cold_calls = sessions::prime_agent::transcript_decode_call_counts();
            assert_eq!(cold_calls, (2, 0), "{damage}");
            let warm = parse_all_messages_with_pricing(
                source_home.path().to_str().unwrap(),
                &clients,
                None,
            );
            assert_eq!(
                sessions::prime_agent::transcript_decode_call_counts(),
                (3, 0),
                "{damage}: warm scan may revalidate the intentionally empty child, but must reuse the parent"
            );

            for messages in [cold, warm] {
                assert_eq!(messages.len(), 1, "{damage}");
                assert_eq!(messages[0].tokens.input, 150, "{damage}");
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_escaped_replacement_lineage_key_is_safe_cold_and_warm() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        let child_dir = source_home
            .path()
            .join(".prime/agent/session-artifacts/root/sub-child");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        let root_path = sessions_dir.join("root.jsonl");
        std::fs::write(
            &root_path,
            r#"{"type":"session","version":3,"id":"root","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":150,"output":0}}}
{"type":"child_usage_attributed","id":"usage-1","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"output":0},"aggregateUsage":{"input":150,"output":0}}
"#,
        )
        .unwrap();
        let clean_parent = paths::json_path_literal(&root_path);
        std::fs::write(
            child_dir.join("child.jsonl"),
            format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"cwd\":\"/tmp/project\",\"unrelated\\uD800\":true,\"parentSession\":{clean_parent},\"rlmDep\\uFFFDth\":1}}\n{{\"type\":\"message\",\"id\":\"child-message\",\"timestamp\":\"2026-08-08T00:00:02.000Z\",\"message\":{{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{{\"input\":50,\"output\":0}}}}}}\n"
            ),
        )
        .unwrap();

        let clients = ["prime-agent".to_string()];
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let cold =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            (2, 0)
        );
        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            (3, 0),
            "the rejected child may be revalidated, but the parent stays warm"
        );
        for messages in [cold, warm] {
            assert_eq!(messages.len(), 1, "only the parent aggregate is emitted");
            assert_eq!(messages[0].session_id, "root");
            assert_eq!(messages[0].tokens.input, 150);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_dropped_damaged_usage_keeps_later_adjustment_aligned_cold_and_warm() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        let child_dir = source_home
            .path()
            .join(".prime/agent/session-artifacts/root/sub-child");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        let root_path = sessions_dir.join("root.jsonl");
        std::fs::write(
            &root_path,
            r#"{"type":"session","version":3,"id":"root","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"damaged","timestamp":"2026-08-08T00:00:00.500Z","message":{"role":"assistant","provider":"anthropic","model":"damaged-model","responseId":"damaged-response","usage":{"in�put":999,"output":0}}}
{"type":"message","id":"valid","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"valid-model","responseId":"valid-response","usage":{"input":20,"output":0}}}
{"type":"child_usage_attributed","id":"usage-valid","timestamp":"2026-08-08T00:00:02.000Z","targetId":"valid","childUsage":{"input":3,"output":0},"aggregateUsage":{"input":20,"output":0}}
"#,
        )
        .unwrap();
        std::fs::write(
            child_dir.join("child.jsonl"),
            format!(
                r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:01.500Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"child-model","responseId":"child-response","usage":{{"input":3,"output":0}}}}}}
"#,
                paths::json_path_literal(&root_path)
            ),
        )
        .unwrap();

        let clients = ["prime-agent".to_string()];
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let cold =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        let cold_decode_calls = sessions::prime_agent::transcript_decode_call_counts();
        assert_eq!(cold_decode_calls, (2, 0));
        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            cold_decode_calls,
            "the unchanged warm scan must reuse aligned messages and accounting"
        );

        for messages in [cold, warm] {
            assert_eq!(messages.len(), 2);
            assert!(!messages
                .iter()
                .any(|message| message.model_id == "damaged-model"));
            assert_eq!(
                messages
                    .iter()
                    .find(|message| message.model_id == "valid-model")
                    .unwrap()
                    .tokens
                    .input,
                17,
                "the later valid parent must retain its child-usage adjustment"
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                20
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_warm_cache_preserves_distinct_invalid_utf8_ids() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let source_path = sessions_dir.join("damaged-ids.jsonl");
        let mut source = std::fs::File::create(&source_path).unwrap();
        use std::io::Write as _;
        source
            .write_all(
                b"{\"type\":\"session\",\"version\":3,\"id\":\"root\",\"cwd\":\"/tmp/project\"}\n",
            )
            .unwrap();
        source
            .write_all(b"{\"type\":\"message\",\"id\":\"assistant-\xff\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":10,\"output\":5}}}\n")
            .unwrap();
        source
            .write_all(b"{\"type\":\"message\",\"id\":\"assistant-\xfe\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":10,\"output\":5}}}\n")
            .unwrap();
        source.flush().unwrap();
        drop(source);

        let clients = ["prime-agent".to_string()];
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let cold =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        let cold_calls = sessions::prime_agent::transcript_decode_call_counts();
        assert_eq!(cold_calls, (1, 0));

        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            cold_calls,
            "the warm scan must reuse the two distinct cached records"
        );

        for messages in [cold, warm] {
            assert_eq!(messages.len(), 2);
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                20
            );
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.output)
                    .sum::<i64>(),
                10
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_warm_cache_hashes_unsampled_semantic_rewrite() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        let child_dir = source_home
            .path()
            .join(".prime/agent/session-artifacts/legacy/sub-child");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        let source_path = sessions_dir.join("legacy.jsonl");
        let child_path = child_dir.join("child.jsonl");
        let old_contents = large_prime_contents(120, 20);
        let new_contents = large_prime_contents(240, 40);
        assert_eq!(old_contents.len(), new_contents.len());
        assert_eq!(&old_contents[..4_096], &new_contents[..4_096]);
        assert_eq!(&old_contents[23_976..], &new_contents[23_976..]);
        std::fs::write(&source_path, old_contents).unwrap();
        std::fs::write(
            &child_path,
            format!(
                r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:01.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-response","usage":{{"input":40,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":40}}}}}}
"#,
                paths::json_path_literal(&source_path)
            ),
        )
        .unwrap();

        let clients = ["prime-agent".to_string()];
        let established =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            established
                .iter()
                .map(|message| message.tokens.input)
                .sum::<i64>(),
            160
        );

        let original_modified = std::fs::metadata(&source_path).unwrap().modified().unwrap();
        std::fs::write(&source_path, new_contents).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&source_path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .unwrap();

        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            (1, 0),
            "the rewritten root is decoded once while the unchanged child stays decode-free"
        );

        let (root_messages, root_accounting) =
            sessions::prime_agent::parse_prime_agent_file_with_accounting(&source_path);
        let (child_messages, child_accounting) =
            sessions::prime_agent::parse_prime_agent_file_with_accounting(&child_path);
        let expected_cold = sessions::prime_agent::reconcile_prime_agent_messages(
            root_messages.into_iter().chain(child_messages).collect(),
            &[root_accounting, child_accounting],
        );
        assert_eq!(warm, expected_cold);
        assert_eq!(
            warm.iter().map(|message| message.tokens.input).sum::<i64>(),
            240,
            "stale accounting would fail to subtract the rewritten 40-token child aggregate"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_retries_when_source_changes_before_combined_parse() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let source_path = sessions_dir.join("parse-race.jsonl");
        std::fs::write(&source_path, large_prime_contents(120, 20)).unwrap();

        let clients = ["prime-agent".to_string()];
        let established =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(established[0].tokens.input, 120);

        std::fs::write(&source_path, large_prime_contents(360, 60)).unwrap();
        sessions::prime_agent::schedule_stable_parse_test_rewrite(
            &source_path,
            large_prime_contents(480, 80),
        );
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let rebuilt =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(rebuilt[0].tokens.input, 480);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            (2, 0),
            "the first parse belongs to a different pre-parse fingerprint and must be retried"
        );

        let identity = message_cache::CacheIdentity::for_client(ClientId::PrimeAgent);
        let cached = message_cache::SourceMessageCache::load();
        let entry = cached.get(identity, &source_path).unwrap();
        assert_eq!(
            entry.fingerprint,
            message_cache::SourceFingerprint::from_path(&source_path).unwrap()
        );
        assert_eq!(entry.messages[0].tokens.input, 480);
        assert!(entry.prime_accounting.is_some());

        let decode_calls = sessions::prime_agent::transcript_decode_call_counts();
        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(warm[0].tokens.input, 480);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            decode_calls,
            "the exact stable retry snapshot should be a decode-free warm hit"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_legacy_cache_backfills_accounting_once() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let source_path = sessions_dir.join("legacy.jsonl");
        std::fs::write(
            &source_path,
            r#"{"type":"session","version":3,"id":"legacy","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":120,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":130}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":20,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":20},"aggregateUsage":{"input":120,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":130},"origin":"spawn_task"}
"#,
        )
        .unwrap();

        // Reproduce a successfully migrated v4 entry: messages and the exact
        // fingerprint survive, while the newly-added Prime accounting payload
        // is absent until the next scan backfills it.
        let identity = message_cache::CacheIdentity::for_client(ClientId::PrimeAgent);
        let messages = sessions::prime_agent::parse_prime_agent_file(&source_path);
        let legacy_fingerprint =
            match message_cache::SourceFingerprint::check_path_samples_only(&source_path, None)
                .unwrap()
            {
                message_cache::FingerprintStatus::Changed(fingerprint) => fingerprint,
                message_cache::FingerprintStatus::Unchanged => unreachable!(),
            };
        let mut cache = message_cache::SourceMessageCache::default();
        cache.insert(message_cache::CachedSourceEntry::new(
            identity,
            &source_path,
            legacy_fingerprint,
            messages,
            Vec::new(),
            None,
        ));
        cache.save_if_dirty();

        let clients = ["prime-agent".to_string()];
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let first =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        let first_calls = sessions::prime_agent::transcript_decode_call_counts();
        assert_eq!(first_calls, (1, 1));
        assert_eq!(first[0].tokens.input, 120);

        let second =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            first_calls
        );
        assert_eq!(second[0].tokens.input, 120);
        assert!(message_cache::SourceMessageCache::load()
            .get(identity, &source_path)
            .unwrap()
            .prime_accounting
            .is_some());
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_legacy_backfill_rebuilds_if_source_changes_during_read() {
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions_dir = source_home.path().join(".prime/agent/sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let source_path = sessions_dir.join("legacy-race.jsonl");

        let old_contents = large_prime_contents(120, 20);
        let new_contents = large_prime_contents(240, 40);
        assert_eq!(old_contents.len(), new_contents.len());
        assert_eq!(&old_contents[..4_096], &new_contents[..4_096]);
        assert_eq!(&old_contents[23_976..], &new_contents[23_976..]);
        std::fs::write(&source_path, &old_contents).unwrap();

        let identity = message_cache::CacheIdentity::for_client(ClientId::PrimeAgent);
        let messages = sessions::prime_agent::parse_prime_agent_file(&source_path);
        let mut cache = message_cache::SourceMessageCache::default();
        cache.insert(message_cache::CachedSourceEntry::new(
            identity,
            &source_path,
            message_cache::SourceFingerprint::from_path(&source_path).unwrap(),
            messages,
            Vec::new(),
            None,
        ));
        cache.save_if_dirty();

        sessions::prime_agent::schedule_accounting_backfill_test_rewrite(
            &source_path,
            new_contents,
        );
        sessions::prime_agent::reset_transcript_decode_call_counts(source_home.path());
        let clients = ["prime-agent".to_string()];
        let rebuilt =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        let rebuild_calls = sessions::prime_agent::transcript_decode_call_counts();
        assert_eq!(rebuild_calls, (1, 1));
        assert_eq!(rebuilt[0].tokens.input, 240);

        let warm =
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None);
        assert_eq!(
            sessions::prime_agent::transcript_decode_call_counts(),
            rebuild_calls
        );
        assert_eq!(warm[0].tokens.input, 240);
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_concurrent_equal_children_are_counted_once() {
        // Two children of the same parent spent identical tokens and finished in
        // the same millisecond, so no timestamp separates one child's response
        // from the other's attribution. Both must still be paired off: keeping
        // the aggregate parent while also counting both transcripts would report
        // their usage twice.
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions = source_home.path().join(".prime/agent/sessions");
        std::fs::create_dir_all(&sessions).unwrap();

        let root_path = sessions.join("a-root.jsonl");
        std::fs::write(
            &root_path,
            r#"{"type":"session","version":3,"id":"root","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":300,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":300}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":100,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":100},"aggregateUsage":{"input":200,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":200},"origin":"spawn_task"}
{"type":"child_usage_attributed","id":"usage-b","parentId":"usage-a","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":100,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":100},"aggregateUsage":{"input":300,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":300},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        for child in ["sub-a", "sub-b"] {
            let child_dir = source_home
                .path()
                .join(".prime/agent/session-artifacts/a-root")
                .join(child);
            std::fs::create_dir_all(&child_dir).unwrap();
            std::fs::write(
                child_dir.join("child.jsonl"),
                format!(
                    r#"{{"type":"session","version":3,"id":"{child}","timestamp":"2026-08-08T00:00:01.500Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"{child}-response","usage":{{"input":100,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":100}}}}}}
"#,
                    paths::json_path_literal(&root_path)
                ),
            )
            .unwrap();
        }

        let clients = ["prime-agent".to_string()];
        for messages in [
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None),
            // Warm source-cache lane must agree with the cold parse exactly.
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None),
        ] {
            assert_eq!(messages.len(), 3);
            // 100 own parent usage plus the two 100-token children, each counted
            // once from its own transcript.
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                300
            );
        }

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(source_home.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(clients.to_vec()),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(
            parsed
                .messages
                .iter()
                .map(|message| message.input)
                .sum::<i64>(),
            300
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_colliding_attribution_ids_do_not_cross_lineages() {
        // Prime mints attribution ids as `randomUUID().slice(0, 8)` and only
        // checks them against the session it is writing, so two unrelated
        // sessions can carry the same id. Resolving one lineage's child must
        // not mark the other lineage's attribution as accounted for.
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions = source_home.path().join(".prime/agent/sessions");
        let child_dir = source_home
            .path()
            .join(".prime/agent/session-artifacts/a-lineage/sub-a");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();

        let lineage_a = sessions.join("a-lineage.jsonl");
        std::fs::write(
            &lineage_a,
            r#"{"type":"session","version":3,"id":"parent-a","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-a-response","usage":{"input":120,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":120}}}
{"type":"child_usage_attributed","id":"deadbeef","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":20,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":20},"aggregateUsage":{"input":120,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":120},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        std::fs::write(
            child_dir.join("child.jsonl"),
            format!(
                r#"{{"type":"session","version":3,"id":"child-a","timestamp":"2026-08-08T00:00:01.500Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.001Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-a-response","usage":{{"input":20,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":20}}}}}}
"#,
                paths::json_path_literal(&lineage_a)
            ),
        )
        .unwrap();
        // Same 8-hex id, unrelated session, and its child transcript is gone.
        std::fs::write(
            sessions.join("b-lineage.jsonl"),
            r#"{"type":"session","version":3,"id":"parent-b","timestamp":"2026-08-09T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-09T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-b-response","usage":{"input":130,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":130}}}
{"type":"child_usage_attributed","id":"deadbeef","parentId":"parent","timestamp":"2026-08-09T00:00:02.000Z","targetId":"parent","childUsage":{"input":30,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":30},"aggregateUsage":{"input":130,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":130},"origin":"spawn_task"}
"#,
        )
        .unwrap();

        let clients = ["prime-agent".to_string()];
        for messages in [
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None),
            // Warm source-cache lane must agree with the cold parse exactly.
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None),
        ] {
            assert_eq!(messages.len(), 3);
            // 100 reconciled parent + 20 parsed child + 130 aggregate parent
            // whose own child was pruned.
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.tokens.input)
                    .sum::<i64>(),
                250
            );
        }

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(source_home.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(clients.to_vec()),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(
            parsed
                .messages
                .iter()
                .map(|message| message.input)
                .sum::<i64>(),
            250
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_prime_agent_contested_child_is_attributed_to_the_nearest_model() {
        // Two parent responses on different models each persist an aggregate that
        // contains one 50-token child, and only the second parent's child
        // transcript survives. Both attributions are inside the tolerance window,
        // so a maximum-cardinality match could reduce either aggregate and leave
        // the global total intact -- but pricing is applied per model after
        // reconciliation, so the wrong choice moves cost between models.
        let cache_home = tempfile::TempDir::new().unwrap();
        let source_home = tempfile::TempDir::new().unwrap();
        let _cache_env = redirect_cache_home(cache_home.path());
        let sessions = source_home.path().join(".prime/agent/sessions");
        let child_dir = source_home
            .path()
            .join(".prime/agent/session-artifacts/parent/sub-child");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();

        let parent_path = sessions.join("parent.jsonl");
        std::fs::write(
            &parent_path,
            r#"{"type":"session","version":3,"id":"parent","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent-a","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"model-a","responseId":"parent-response-a","usage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"child_usage_attributed","id":"00000000","parentId":"parent-a","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent-a","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150},"origin":"spawn_task"}
{"type":"message","id":"parent-b","parentId":"00000000","timestamp":"2026-08-08T00:00:01.500Z","message":{"role":"assistant","provider":"anthropic","model":"model-b","responseId":"parent-response-b","usage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"child_usage_attributed","id":"ffffffff","parentId":"parent-b","timestamp":"2026-08-08T00:00:02.002Z","targetId":"parent-b","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        std::fs::write(
            child_dir.join("child.jsonl"),
            format!(
                r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:01.600Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:02.002Z","message":{{"role":"assistant","provider":"anthropic","model":"child-model","responseId":"child-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                paths::json_path_literal(&parent_path)
            ),
        )
        .unwrap();

        let clients = ["prime-agent".to_string()];
        for messages in [
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None),
            // The warm source-cache lane must produce the same per-model rows,
            // not just the same total.
            parse_all_messages_with_pricing(source_home.path().to_str().unwrap(), &clients, None),
        ] {
            let mut per_model: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            for message in &messages {
                *per_model.entry(message.model_id.clone()).or_default() += message.tokens.input;
            }
            assert_eq!(per_model.get("model-a").copied(), Some(150));
            assert_eq!(per_model.get("model-b").copied(), Some(100));
            assert_eq!(per_model.get("child-model").copied(), Some(50));
            assert_eq!(per_model.values().sum::<i64>(), 300);
        }
    }

    #[test]
    fn test_parse_local_clients_reasonix_counts_reported_requests() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // Where the scan will actually look, rather than the Unix spelling of
        // it: under an explicit home Reasonix lives at `~/.reasonix` on Unix
        // and `%HOME%\AppData\Roaming\reasonix` on Windows, so a hardcoded
        // `.reasonix/stats` fixture is written somewhere the scanner never
        // reads and the test asserts on an empty parse. The path layout has its
        // own coverage in `clients::tests`; this test is about the request
        // count.
        let stats_dir = std::path::PathBuf::from(
            ClientId::Reasonix
                .data()
                .resolve_path_with_env_strategy(&temp_dir.path().to_string_lossy(), false),
        );
        std::fs::create_dir_all(&stats_dir).unwrap();
        std::fs::write(
            stats_dir.join("2026-08-04.jsonl"),
            "{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"prompt\":100,\"completion\":20,\"total\":120,\"requests\":3}\n",
        )
        .unwrap();

        let parsed = parse_local_clients(LocalParseOptions {
            home_dir: Some(temp_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["reasonix".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: scanner::ScannerSettings::default(),
        })
        .unwrap();

        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].message_count, 3);
        assert_eq!(parsed.counts.get(ClientId::Reasonix), 3);
    }

    #[test]
    fn test_retain_for_requested_clients_gjc_superset_of_9router() {
        let gjc_requested: HashSet<&str> = HashSet::from(["gjc"]);
        // Bridge messages carry client="9router"; requesting "gjc" retains
        // them (9router data IS gjc-format, so gjc is a superset request).
        assert!(retain_for_requested_clients(
            "9router",
            "deepseek-ai/deepseek-v4-flash",
            "nvidia",
            &gjc_requested
        ));
        // --client 9router retains bridge-stamped messages…
        let ninerouter_requested: HashSet<&str> = HashSet::from(["9router"]);
        assert!(retain_for_requested_clients(
            "9router",
            "deepseek-ai/deepseek-v4-flash",
            "nvidia",
            &ninerouter_requested
        ));
        // …but must NOT retain native gjc messages: the alias is one-way
        // (gjc is the superset request, 9router is the narrow one).
        assert!(!retain_for_requested_clients(
            "gjc",
            "claude-sonnet-4",
            "anthropic",
            &ninerouter_requested
        ));
        // Unrelated clients still filtered out.
        assert!(!retain_for_requested_clients(
            "claude",
            "gpt-4o",
            "openai",
            &gjc_requested
        ));
    }

    #[test]
    fn test_filter_messages_preserves_pi_9router_when_no_duplicate() {
        let messages = vec![
            UnifiedMessage::new(
                "pi",
                "deepseek_v4_flash_free",
                "9router",
                "session-1",
                1783412353188,
                TokenBreakdown::default(),
                0.0,
            ),
            UnifiedMessage::new(
                "9router",
                "deepseek-ai/deepseek-v4-flash",
                "nvidia",
                "session-2",
                1783412353188,
                TokenBreakdown {
                    input: 100,
                    output: 50,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.05,
            ),
        ];
        // Without verified cross-source dedup, both messages are preserved.
        let filtered = filter_messages_for_report(messages, &ReportOptions::default());
        assert_eq!(filtered.len(), 2);
    }
}
