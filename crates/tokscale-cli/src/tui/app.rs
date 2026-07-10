use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use tokscale_core::ClientId;

use crate::commands::usage::{UsageFetchReport, UsageOutput};
use crate::ClientFilter;

use ratatui::style::Color;

use super::codex_login::{
    cancel_codex_login_child, run_codex_login_worker, CodexLoginChildSlot, CodexLoginEvent,
    CodexLoginOutcome,
};
use super::data::{
    AgentUsage, DailyUsage, DataLoader, HourlyUsage, MinutelyUsage, ModelUsage, MonthlyUsage,
    TokenBreakdown, UsageData,
};
use super::privacy::looks_like_email;
use super::settings::Settings;
use super::themes::{Theme, ThemeName};
use super::ui::dialog::{ClientPickerDialog, ConfirmDialog, DialogStack};
use super::ui::widgets::{get_model_color, get_provider_from_model, get_provider_shade};

/// Configuration for TUI initialization
pub struct TuiConfig {
    pub theme: String,
    pub refresh: u64,
    pub sessions_path: Option<String>,
    pub clients: Option<Vec<String>>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
    pub initial_tab: Option<Tab>,
}

#[cfg(not(test))]
fn default_usage_fetcher() -> UsageFetchReport {
    crate::commands::usage::fetch_all_report_with_intent(
        crate::commands::usage::UsageFetchIntent::TuiSurface,
    )
}

#[cfg(test)]
fn test_usage_fetcher() -> UsageFetchReport {
    UsageFetchReport::default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Overview,
    Usage,
    Models,
    Daily,
    Hourly,
    Minutely,
    Monthly,
    Stats,
    Agents,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[
            Tab::Overview,
            Tab::Usage,
            Tab::Models,
            Tab::Daily,
            Tab::Hourly,
            Tab::Minutely,
            Tab::Monthly,
            Tab::Stats,
            Tab::Agents,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Usage => "Usage",
            Tab::Models => "Models",
            Tab::Daily => "Daily",
            Tab::Hourly => "Hourly",
            Tab::Minutely => "Minutely",
            Tab::Monthly => "Monthly",
            Tab::Stats => "Stats",
            Tab::Agents => "Agents",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Tab::Overview => "Ovw",
            Tab::Usage => "Use",
            Tab::Models => "Mod",
            Tab::Daily => "Day",
            Tab::Hourly => "Hr",
            Tab::Minutely => "Min",
            Tab::Monthly => "Mon",
            Tab::Stats => "Sta",
            Tab::Agents => "Agt",
        }
    }

    pub fn next(self) -> Tab {
        match self {
            Tab::Overview => Tab::Usage,
            Tab::Usage => Tab::Models,
            Tab::Models => Tab::Daily,
            Tab::Daily => Tab::Hourly,
            Tab::Hourly => Tab::Minutely,
            Tab::Minutely => Tab::Monthly,
            Tab::Monthly => Tab::Stats,
            Tab::Stats => Tab::Agents,
            Tab::Agents => Tab::Overview,
        }
    }

    pub fn prev(self) -> Tab {
        match self {
            Tab::Overview => Tab::Agents,
            Tab::Usage => Tab::Overview,
            Tab::Models => Tab::Usage,
            Tab::Daily => Tab::Models,
            Tab::Hourly => Tab::Daily,
            Tab::Minutely => Tab::Hourly,
            Tab::Monthly => Tab::Minutely,
            Tab::Stats => Tab::Monthly,
            Tab::Agents => Tab::Stats,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartGranularity {
    #[default]
    Daily,
    Hourly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Cost,
    Tokens,
    Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HourlyViewMode {
    #[default]
    Table,
    Profile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

pub struct ClickArea {
    pub rect: Rect,
    pub action: ClickAction,
}

#[derive(Debug, Clone, Copy)]
pub struct DailyDetailRow<'a> {
    pub source: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub color_key: &'a str,
    pub tokens: &'a TokenBreakdown,
    pub cost: f64,
    pub messages: u64,
}

#[derive(Debug, Clone)]
pub enum ClickAction {
    Tab(Tab),
    Sort(SortField),
    GraphCell { week: usize, day: usize },
    UsageRefresh,
    CodexStartLogin,
    CodexDismissLogin,
    UsageSelect { index: usize },
    UsageToggleEmailPrivacy,
    CodexUseAccount { account_id: String },
    CodexRemoveAccount { account_id: String },
    CodexResetAccount { account_id: String },
}

fn codex_reset_outcome_label(
    result: &crate::commands::usage::codex::RateLimitResetConsumeResult,
) -> String {
    match result.code.as_str() {
        "reset" => match result.windows_reset {
            Some(1) => "reset 1 window".to_string(),
            Some(count) => format!("reset {count} windows"),
            None => "reset complete".to_string(),
        },
        "already_redeemed" => "credit already redeemed".to_string(),
        "nothing_to_reset" => "nothing to reset".to_string(),
        "no_credit" => "no credit available".to_string(),
        "" => "unknown response".to_string(),
        other => other.to_string(),
    }
}

fn short_account_id(account_id: &str) -> String {
    let id = account_id.trim();
    if id.is_empty() {
        return "Account unknown".to_string();
    }

    let char_count = id.chars().count();
    if char_count <= 12 {
        return format!("Account {id}");
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
    format!("Account {head}...{tail}")
}

fn compare_codex_usage_outputs(a: &UsageOutput, b: &UsageOutput) -> std::cmp::Ordering {
    let active_order = codex_usage_is_active(b).cmp(&codex_usage_is_active(a));
    if active_order != std::cmp::Ordering::Equal {
        return active_order;
    }

    codex_usage_sort_key(a)
        .cmp(&codex_usage_sort_key(b))
        .then_with(|| codex_usage_account_id(a).cmp(codex_usage_account_id(b)))
}

fn codex_usage_is_active(output: &UsageOutput) -> bool {
    output
        .account
        .as_ref()
        .is_some_and(|account| account.is_active)
}

fn codex_usage_sort_key(output: &UsageOutput) -> String {
    output
        .account
        .as_ref()
        .map(|account| {
            account
                .label_name()
                .unwrap_or(account.id.as_str())
                .to_lowercase()
        })
        .unwrap_or_else(|| output.display_name().to_lowercase())
}

fn codex_usage_account_id(output: &UsageOutput) -> &str {
    output
        .account
        .as_ref()
        .map(|account| account.id.as_str())
        .unwrap_or_default()
}

struct MinutelySortCache {
    sort_field: SortField,
    sort_direction: SortDirection,
    data_version: u64,
    data_len: usize,
    indices: Vec<usize>,
}

pub struct App {
    pub should_quit: bool,
    pub current_tab: Tab,
    pub theme: Theme,
    pub settings: Settings,
    pub data: UsageData,
    pub data_loader: DataLoader,

    /// Set of clients currently selected in the source picker. The
    /// `Synthetic` variant is part of the same set so dialog code can
    /// uniformly toggle/inspect every option without a separate boolean.
    /// Code that talks to `tokscale_core` (which still expects a
    /// `Vec<ClientId>` plus a `bool include_synthetic`) projects this set
    /// at the boundary via `App::scan_clients` and `App::include_synthetic`.
    pub enabled_clients: Rc<RefCell<HashSet<ClientFilter>>>,
    pub group_by: Rc<RefCell<tokscale_core::GroupBy>>,
    pub sort_field: SortField,
    pub sort_direction: SortDirection,
    tab_sort_state: HashMap<Tab, (SortField, SortDirection)>,
    pub chart_granularity: ChartGranularity,

    pub scroll_offset: usize,
    pub selected_index: usize,
    pub max_visible_items: usize,
    pub selected_daily_detail_date: Option<NaiveDate>,
    daily_list_selected_index: usize,
    daily_list_scroll_offset: usize,

    pub selected_monthly_detail_month: Option<String>,
    monthly_list_selected_index: usize,
    monthly_list_scroll_offset: usize,

    pub selected_graph_cell: Option<(usize, usize)>,
    pub stats_breakdown_total_lines: usize,

    pub auto_refresh: bool,
    pub auto_refresh_interval: Duration,
    pub last_auto_refresh: Instant,
    pub last_refresh: Instant,

    pub status_message: Option<String>,
    pub status_message_time: Option<Instant>,

    pub terminal_width: u16,
    pub terminal_height: u16,

    pub click_areas: Vec<ClickArea>,

    pub spinner_frame: usize,

    pub background_loading: bool,

    pub needs_reload: bool,

    pub dialog_stack: DialogStack,

    pub dialog_needs_reload: Rc<RefCell<bool>>,

    pub hourly_view_mode: HourlyViewMode,

    pub model_shade_map: HashMap<String, Color>,

    pub subscription_usage: Vec<crate::commands::usage::UsageOutput>,
    pub usage_fetch_diagnostics: Vec<crate::commands::usage::UsageFetchDiagnostic>,
    confirmed_codex_use_account_id: Rc<RefCell<Option<String>>>,
    confirmed_codex_remove_account_id: Rc<RefCell<Option<String>>>,
    confirmed_codex_reset_account_id: Rc<RefCell<Option<String>>>,
    pub hide_usage_emails: bool,
    pub codex_login_lines: Vec<String>,
    pub(crate) codex_login_outcome: Option<CodexLoginOutcome>,

    pub usage_fetch_attempted: bool,
    usage_rx: Option<std::sync::mpsc::Receiver<UsageFetchReport>>,
    usage_fetch_preserve_status: bool,
    usage_fetcher: fn() -> UsageFetchReport,
    codex_reset_rx: Option<
        std::sync::mpsc::Receiver<
            Result<crate::commands::usage::codex::RateLimitResetConsumeResult, String>,
        >,
    >,
    codex_login_rx: Option<std::sync::mpsc::Receiver<CodexLoginEvent>>,
    codex_login_child: Option<CodexLoginChildSlot>,

    /// Server-side stats aggregated across all of the user's devices
    /// (`GET /api/me/stats`). `None` means local-only: logged out, offline,
    /// or the fetch has not completed yet.
    pub remote_stats: Option<crate::tui::remote::RemoteStats>,
    remote_stats_rx: Option<std::sync::mpsc::Receiver<crate::tui::remote::RemoteStats>>,
    /// Throttles background refresh attempts so a failing fetch (offline,
    /// expired token) is not retried on every tick.
    remote_stats_last_attempt: Option<std::time::Instant>,

    data_version: u64,
    minutely_sort_cache: RefCell<Option<MinutelySortCache>>,
}

impl App {
    pub fn new_with_cached_data(config: TuiConfig, cached_data: Option<UsageData>) -> Result<Self> {
        let settings = Settings::load();
        let theme_name: ThemeName = if config.theme.is_empty() {
            settings.theme_name()
        } else {
            config
                .theme
                .parse()
                .unwrap_or_else(|_| settings.theme_name())
        };
        let theme = Theme::from_name_for_current_terminal(theme_name);

        let enabled_clients: HashSet<ClientFilter> = if let Some(ref cli_clients) = config.clients {
            // CLI-provided filter list. Each entry is the canonical
            // lowercase id (`opencode`, `claude`, ..., `synthetic`).
            // Unknown ids are dropped silently; the CLI parser already
            // validated against `ClientFilter` so this lookup should be
            // total in practice.
            cli_clients
                .iter()
                .filter_map(|s| ClientFilter::from_filter_str(&s.to_lowercase()))
                .collect()
        } else {
            // No filter → use the canonical default set (every real
            // client, Synthetic opt-in only). MUST stay in sync with
            // `run_warm_tui_cache()` so a fresh cache warm produces a
            // fresh hit on the next no-filter launch.
            ClientFilter::default_set()
        };

        let auto_refresh_interval = if config.refresh > 0 {
            Duration::from_secs(config.refresh)
        } else if let Some(interval) = settings.get_auto_refresh_interval() {
            interval
        } else {
            Duration::from_secs(30)
        };

        let auto_refresh = config.refresh > 0 || settings.auto_refresh_enabled;

        let data_loader = DataLoader::with_filters(
            config.sessions_path.map(std::path::PathBuf::from),
            config.since,
            config.until,
            config.year,
        )
        .with_minutely_enabled(settings.minutely_tab_enabled);

        let data = cached_data.unwrap_or_default();
        let has_data = !data.models.is_empty();
        let dialog_stack = DialogStack::new(theme.clone());
        let dialog_needs_reload = Rc::new(RefCell::new(false));
        let confirmed_codex_use_account_id = Rc::new(RefCell::new(None));
        let confirmed_codex_remove_account_id = Rc::new(RefCell::new(None));
        let confirmed_codex_reset_account_id = Rc::new(RefCell::new(None));
        let requested_tab = config.initial_tab.unwrap_or(Tab::Overview);
        let current_tab = if Self::tab_visible(&settings, requested_tab) {
            requested_tab
        } else {
            Tab::Overview
        };
        let (sort_field, sort_direction) = Self::default_sort_for_tab(current_tab);

        let mut app = Self {
            should_quit: false,
            current_tab,
            theme,
            settings,
            data,
            data_loader,
            enabled_clients: Rc::new(RefCell::new(enabled_clients)),
            group_by: Rc::new(RefCell::new(super::cache::TUI_DEFAULT_GROUP_BY)),
            sort_field,
            sort_direction,
            tab_sort_state: HashMap::new(),
            chart_granularity: ChartGranularity::default(),
            scroll_offset: 0,
            selected_index: 0,
            max_visible_items: 20,
            selected_daily_detail_date: None,
            daily_list_selected_index: 0,
            daily_list_scroll_offset: 0,
            selected_monthly_detail_month: None,
            monthly_list_selected_index: 0,
            monthly_list_scroll_offset: 0,
            selected_graph_cell: None,
            stats_breakdown_total_lines: 0,
            auto_refresh,
            auto_refresh_interval,
            last_auto_refresh: Instant::now(),
            last_refresh: Instant::now(),
            status_message: if has_data {
                Some("Loaded from cache".to_string())
            } else {
                None
            },
            status_message_time: if has_data { Some(Instant::now()) } else { None },
            terminal_width: 80,
            terminal_height: 24,
            click_areas: Vec::new(),
            spinner_frame: 0,
            background_loading: false,
            needs_reload: false,
            dialog_stack,
            dialog_needs_reload,
            hourly_view_mode: HourlyViewMode::default(),
            model_shade_map: HashMap::new(),
            subscription_usage: {
                #[cfg(not(test))]
                {
                    crate::commands::usage::load_cache().unwrap_or_default()
                }
                #[cfg(test)]
                {
                    Vec::new()
                }
            },
            usage_fetch_diagnostics: Vec::new(),
            confirmed_codex_use_account_id,
            confirmed_codex_remove_account_id,
            confirmed_codex_reset_account_id,
            hide_usage_emails: true,
            codex_login_lines: Vec::new(),
            codex_login_outcome: None,
            usage_fetch_attempted: false,
            usage_rx: None,
            usage_fetch_preserve_status: false,
            usage_fetcher: {
                #[cfg(test)]
                {
                    test_usage_fetcher
                }
                #[cfg(not(test))]
                {
                    default_usage_fetcher
                }
            },
            codex_reset_rx: None,
            codex_login_rx: None,
            codex_login_child: None,
            remote_stats: None,
            remote_stats_rx: None,
            remote_stats_last_attempt: None,
            data_version: 0,
            minutely_sort_cache: RefCell::new(None),
        };
        app.build_model_shade_map();
        app.maybe_fetch_usage_on_entry();
        Ok(app)
    }

    pub fn set_background_loading(&mut self, loading: bool) {
        self.background_loading = loading;
        // Don't set data.loading - let cached data remain visible during background refresh
    }

    pub fn update_data(&mut self, data: UsageData) {
        self.data = data;
        self.data_version = self.data_version.saturating_add(1);
        self.last_refresh = Instant::now();
        self.build_model_shade_map();
        self.minutely_sort_cache.borrow_mut().take();

        // Exit Daily-detail mode if the refresh dropped the day we were
        // viewing; otherwise `get_sorted_daily_detail_rows()` would return
        // empty while the user is still nominally in detail mode.
        if let Some(date) = self.selected_daily_detail_date {
            if !self.data.daily.iter().any(|day| day.date == date) {
                self.selected_daily_detail_date = None;
                self.selected_index = self.daily_list_selected_index;
                self.scroll_offset = self.daily_list_scroll_offset;
            }
        }

        // Same for Monthly-detail mode: exit if the month disappeared.
        if let Some(ref month) = self.selected_monthly_detail_month {
            if !self.data.monthly.iter().any(|m| &m.month == month) {
                self.selected_monthly_detail_month = None;
                self.selected_index = self.monthly_list_selected_index;
                self.scroll_offset = self.monthly_list_scroll_offset;
            }
        }

        self.clamp_selection();
    }

    pub fn build_model_shade_map(&mut self) {
        self.model_shade_map = super::colors::build_model_shade_map(&self.data.models);
    }

    pub fn model_color_for(&self, provider: &str, model: &str) -> Color {
        // Same key derivation as `build_model_shade_map`, so gateway providers
        // (e.g. `github-copilot`) resolve to the model's own vendor ramp and the
        // lookup never misses into the fallback for a model we've ranked.
        let provider = super::colors::provider_color_key(provider, model);
        let lookup_key = super::colors::model_shade_key(provider, model);
        let color = self
            .model_shade_map
            .get(&lookup_key)
            .copied()
            .unwrap_or_else(|| get_provider_shade(provider, 0));
        self.theme.color(color)
    }

    pub fn model_color(&self, model: &str) -> Color {
        let provider = get_provider_from_model(model);
        let lookup_key = super::colors::model_shade_key(provider, model);
        let color = self
            .model_shade_map
            .get(&lookup_key)
            .copied()
            .unwrap_or_else(|| get_model_color(model));
        self.theme.color(color)
    }

    pub fn has_visible_data(&self) -> bool {
        !self.data.models.is_empty()
            || !self.data.daily.is_empty()
            || !self.data.agents.is_empty()
            || self.data.graph.is_some()
            || self.data.total_tokens > 0
            || self.data.total_cost > 0.0
    }

    pub fn set_error(&mut self, error: Option<String>) {
        self.data.error = error;
    }

    pub fn on_tick(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % 20;

        if let Some(status_time) = self.status_message_time {
            if status_time.elapsed() > Duration::from_secs(3) {
                self.status_message = None;
                self.status_message_time = None;
            }
        }

        if self.auto_refresh && self.last_auto_refresh.elapsed() >= self.auto_refresh_interval {
            if self.current_tab == Tab::Usage {
                self.last_auto_refresh = Instant::now();
                // Auto-refresh is a silent background poll, not a user action,
                // so it must not overwrite the current status message (e.g. a
                // Codex reset result) with "Fetching usage data...".
                self.fetch_subscription_usage_preserving_status();
            } else if !self.background_loading {
                self.last_auto_refresh = Instant::now();
                self.needs_reload = true;
            }
        }

        if *self.dialog_needs_reload.borrow() {
            *self.dialog_needs_reload.borrow_mut() = false;
            self.needs_reload = true;
        }

        // Poll background usage fetch
        if let Some(ref rx) = self.usage_rx {
            match rx.try_recv() {
                Ok(report) => {
                    let preserve_status = self.usage_fetch_preserve_status;
                    self.usage_fetch_preserve_status = false;
                    self.usage_rx = None;
                    self.subscription_usage = report.outputs;
                    self.usage_fetch_diagnostics = report.diagnostics;
                    if !self.subscription_usage.is_empty() {
                        crate::commands::usage::save_cache(&self.subscription_usage);
                        if !preserve_status {
                            self.status_message = Some(self.usage_loaded_status());
                        }
                    } else {
                        crate::commands::usage::clear_cache();
                        if !preserve_status {
                            self.status_message = Some(self.usage_empty_status());
                        }
                    }
                    if !preserve_status {
                        self.status_message_time = Some(std::time::Instant::now());
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let preserve_status = self.usage_fetch_preserve_status;
                    self.usage_fetch_preserve_status = false;
                    self.usage_rx = None;
                    if !preserve_status {
                        self.status_message = Some("Usage fetch failed".into());
                        self.status_message_time = Some(std::time::Instant::now());
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some(ref rx) = self.codex_reset_rx {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    self.codex_reset_rx = None;
                    self.status_message = Some(format!(
                        "Codex reset credit: {}",
                        codex_reset_outcome_label(&result)
                    ));
                    self.status_message_time = Some(std::time::Instant::now());
                    self.fetch_subscription_usage_preserving_status();
                }
                Ok(Err(error)) => {
                    self.codex_reset_rx = None;
                    self.status_message = Some(format!("Codex reset failed: {error}"));
                    self.status_message_time = Some(std::time::Instant::now());
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.codex_reset_rx = None;
                    self.status_message = Some("Codex reset failed".into());
                    self.status_message_time = Some(std::time::Instant::now());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        self.poll_remote_stats();
        self.maybe_refresh_remote_stats();

        self.poll_codex_login();
    }

    fn poll_codex_login(&mut self) {
        let mut events = Vec::new();
        let mut disconnected = false;

        if let Some(rx) = &self.codex_login_rx {
            loop {
                match rx.try_recv() {
                    Ok(event) => events.push(event),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        let mut finished = false;
        for event in events {
            match event {
                CodexLoginEvent::Output(line) => {
                    self.codex_login_lines.push(line);
                    const MAX_LOGIN_LINES: usize = 12;
                    if self.codex_login_lines.len() > MAX_LOGIN_LINES {
                        let drain_count = self.codex_login_lines.len() - MAX_LOGIN_LINES;
                        self.codex_login_lines.drain(0..drain_count);
                    }
                }
                CodexLoginEvent::Finished(outcome) => {
                    finished = true;
                    match &outcome {
                        CodexLoginOutcome::Imported(info) => {
                            let display = info.label.as_deref().unwrap_or(&info.id);
                            self.set_status(&format!("Imported Codex account: {display}"));
                        }
                        CodexLoginOutcome::Failed(error) => {
                            self.set_status(&format!("Codex login failed: {error}"));
                        }
                    }
                    self.codex_login_outcome = Some(outcome);
                }
            }
        }

        if disconnected && !finished && self.codex_login_outcome.is_none() {
            self.codex_login_outcome = Some(CodexLoginOutcome::Failed(
                "login worker stopped".to_string(),
            ));
            self.set_status("Codex login failed: login worker stopped");
            finished = true;
        }

        if finished {
            self.codex_login_rx = None;
            self.codex_login_child = None;
            if matches!(
                self.codex_login_outcome,
                Some(CodexLoginOutcome::Imported(_))
            ) {
                self.codex_login_lines.clear();
                self.codex_login_outcome = None;
                self.refresh_usage();
            }
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        // Remap the produced character to its US-QWERTY physical position so
        // single-letter hotkeys keep working under non-Latin layouts (Russian,
        // Greek, …). Modifiers and non-char keys are unaffected. Dialogs still
        // receive the raw `key.code` and normalize per-field, since some of
        // them (e.g. the picker filter) accept literal text input.
        let code = crate::tui::keymap::normalize_hotkey(key.code);

        if code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return true;
        }

        if self.dialog_stack.is_active() {
            self.dialog_stack.handle_key(key.code);
            self.consume_confirmed_codex_account_action();
            return false;
        }

        if code == KeyCode::Esc
            && self.current_tab == Tab::Usage
            && self.should_show_codex_login_panel()
        {
            self.dismiss_codex_login();
            return false;
        }

        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return true;
            }
            KeyCode::Tab => {
                let next = self.next_visible_tab();
                self.switch_tab(next);
                self.reset_selection();
            }
            KeyCode::BackTab => {
                let prev = self.prev_visible_tab();
                self.switch_tab(prev);
                self.reset_selection();
            }
            KeyCode::Left => {
                let prev = self.prev_visible_tab();
                self.switch_tab(prev);
                self.reset_selection();
            }
            KeyCode::Right => {
                let next = self.next_visible_tab();
                self.switch_tab(next);
                self.reset_selection();
            }
            KeyCode::Up => {
                self.move_selection_up();
            }
            KeyCode::Down => {
                self.move_selection_down();
            }
            KeyCode::PageUp => {
                self.move_page_up();
            }
            KeyCode::PageDown => {
                self.move_page_down();
            }
            KeyCode::Home => {
                self.move_to_top();
            }
            KeyCode::End => {
                self.move_to_bottom();
            }
            KeyCode::Char('c') => {
                self.set_sort(SortField::Cost);
            }
            KeyCode::Char('t') => {
                self.set_sort(SortField::Tokens);
            }
            KeyCode::Char('d') => {
                self.set_sort(SortField::Date);
            }
            KeyCode::Char('j') => {
                self.jump_to_today();
            }
            KeyCode::Char('p') => {
                self.cycle_theme();
            }
            KeyCode::Char('r') => {
                self.last_auto_refresh = Instant::now();
                if self.current_tab == Tab::Usage {
                    self.refresh_usage();
                } else if self.background_loading {
                    self.set_status("Refresh already in progress");
                } else {
                    self.needs_reload = true;
                }
            }
            KeyCode::Char('R') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.toggle_auto_refresh();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.increase_refresh_interval();
            }
            KeyCode::Char('-') => {
                self.decrease_refresh_interval();
            }
            KeyCode::Char('y') => {
                self.copy_selected_to_clipboard();
            }
            KeyCode::Char('e') => {
                self.export_to_json();
            }
            KeyCode::Char('s') => {
                self.open_client_picker();
            }
            KeyCode::Char('h') if self.current_tab == Tab::Overview => {
                self.chart_granularity = match self.chart_granularity {
                    ChartGranularity::Daily => ChartGranularity::Hourly,
                    ChartGranularity::Hourly => ChartGranularity::Daily,
                };
            }
            KeyCode::Char('v') if self.current_tab == Tab::Hourly => {
                self.hourly_view_mode = match self.hourly_view_mode {
                    HourlyViewMode::Table => HourlyViewMode::Profile,
                    HourlyViewMode::Profile => HourlyViewMode::Table,
                };
                self.reset_selection();
            }
            KeyCode::Char('g') => {
                self.open_group_by_picker();
            }
            KeyCode::Char('a') if self.current_tab == Tab::Usage => {
                self.start_codex_login();
            }
            KeyCode::Char('m') if self.current_tab == Tab::Usage => {
                self.toggle_usage_email_privacy();
            }
            KeyCode::Char('x') if self.current_tab == Tab::Usage => {
                self.confirm_selected_codex_rate_limit_reset();
            }
            KeyCode::Enter if self.current_tab == Tab::Daily => {
                self.open_selected_daily_detail();
            }
            KeyCode::Enter if self.current_tab == Tab::Monthly => {
                self.open_selected_monthly_detail();
            }
            KeyCode::Enter if self.current_tab == Tab::Stats => {
                self.handle_graph_selection();
            }
            KeyCode::Esc | KeyCode::Backspace
                if self.current_tab == Tab::Daily && self.is_daily_detail_active() =>
            {
                self.close_daily_detail();
            }
            KeyCode::Esc | KeyCode::Backspace
                if self.current_tab == Tab::Monthly && self.is_monthly_detail_active() =>
            {
                self.close_monthly_detail();
            }
            KeyCode::Esc if self.selected_graph_cell.is_some() => {
                self.selected_graph_cell = None;
                self.stats_breakdown_total_lines = 0;
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
            _ => {}
        }
        false
    }

    pub fn fetch_subscription_usage(&mut self) {
        self.fetch_subscription_usage_with_status(false);
    }

    fn fetch_subscription_usage_preserving_status(&mut self) {
        self.fetch_subscription_usage_with_status(true);
    }

    fn fetch_subscription_usage_with_status(&mut self, preserve_status: bool) {
        if self.usage_rx.is_some() {
            if preserve_status {
                self.usage_fetch_preserve_status = true;
            }
            return; // already fetching
        }
        self.usage_fetch_attempted = true;
        self.usage_fetch_preserve_status = preserve_status;
        self.usage_fetch_diagnostics.clear();
        if !preserve_status {
            self.status_message = Some("Fetching usage data...".into());
            self.status_message_time = Some(std::time::Instant::now());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.usage_rx = Some(rx);
        let fetcher = self.usage_fetcher;
        std::thread::spawn(move || {
            let report = fetcher();
            let _ = tx.send(report);
        });
    }

    pub fn refresh_usage(&mut self) {
        if self.usage_rx.is_some() {
            self.set_status("Refresh already in progress");
        } else {
            self.fetch_subscription_usage();
        }
    }

    pub(crate) fn maybe_fetch_usage_on_entry(&mut self) {
        if self.current_tab == Tab::Usage && !self.usage_fetch_attempted && self.usage_rx.is_none()
        {
            self.fetch_subscription_usage();
        }
    }

    pub fn is_fetching_usage(&self) -> bool {
        self.usage_rx.is_some()
    }

    fn usage_loaded_status(&self) -> String {
        match self.usage_fetch_diagnostics.len() {
            0 => "Usage data loaded".to_string(),
            1 => "Usage data loaded with 1 issue".to_string(),
            count => format!("Usage data loaded with {count} issues"),
        }
    }

    fn usage_empty_status(&self) -> String {
        if self.usage_fetch_diagnostics.is_empty() {
            "No usage data available".to_string()
        } else {
            format!("Usage fetch failed: {}", self.usage_diagnostic_summary())
        }
    }

    fn usage_diagnostic_summary(&self) -> String {
        let mut names = self
            .usage_fetch_diagnostics
            .iter()
            .map(|diagnostic| diagnostic.display_name())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        let visible = names.iter().take(2).cloned().collect::<Vec<_>>();
        let hidden = names.len().saturating_sub(visible.len());
        if hidden == 0 {
            visible.join(", ")
        } else {
            format!("{} +{hidden}", visible.join(", "))
        }
    }

    /// Cache-first load of server-side aggregated multi-device stats.
    /// Called once at TUI startup; the background refresh for a stale or
    /// missing cache is driven by `on_tick` via `maybe_refresh_remote_stats`.
    /// Silent on every failure path — the TUI stays local-only.
    pub fn init_remote_stats(&mut self) {
        #[cfg(not(test))]
        {
            let Some(auth) = crate::auth::resolve_api_token() else {
                return;
            };
            // Env-provided tokens carry no username, so their responses are
            // never trusted from cache (cache entries are scoped per account).
            let username = auth.username.unwrap_or_default();
            let api_url = crate::auth::get_api_base_url();
            if let Some(stats) = crate::tui::remote::load_cached_remote_stats(&username, &api_url) {
                self.remote_stats = Some(stats);
            }
        }
    }

    /// Spawn a background `GET /api/me/stats` fetch when the current remote
    /// stats are missing or older than the cache TTL. Attempts are throttled
    /// so an offline machine or expired token does not retry on every tick.
    fn maybe_refresh_remote_stats(&mut self) {
        const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

        if self.remote_stats_rx.is_some() {
            return;
        }
        let stale = self
            .remote_stats
            .as_ref()
            .is_none_or(crate::tui::remote::RemoteStats::is_stale);
        if !stale {
            return;
        }
        if self
            .remote_stats_last_attempt
            .is_some_and(|at| at.elapsed() < RETRY_INTERVAL)
        {
            return;
        }
        self.remote_stats_last_attempt = Some(std::time::Instant::now());

        // Tests must not read real credentials or hit the network.
        #[cfg(not(test))]
        {
            let Some(auth) = crate::auth::resolve_api_token() else {
                return;
            };
            let token = auth.token;
            let username = auth.username.unwrap_or_default();
            let api_url = crate::auth::get_api_base_url();

            let (tx, rx) = std::sync::mpsc::channel();
            self.remote_stats_rx = Some(rx);
            std::thread::spawn(move || {
                if let Ok(stats) =
                    crate::tui::remote::fetch_remote_stats(&token, &username, &api_url)
                {
                    let _ = tx.send(stats);
                }
            });
        }
    }

    /// Poll the background remote stats fetch. Errors are silent: the sender
    /// is simply dropped without a payload and the TUI stays local-only.
    fn poll_remote_stats(&mut self) {
        if let Some(ref rx) = self.remote_stats_rx {
            match rx.try_recv() {
                Ok(stats) => {
                    self.remote_stats_rx = None;
                    self.remote_stats = Some(stats);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.remote_stats_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    fn handle_click_action(&mut self, action: ClickAction) {
        match action {
            ClickAction::Tab(tab) => {
                self.switch_tab(tab);
                self.reset_selection();
            }
            ClickAction::Sort(field) => {
                self.set_sort(field);
            }
            ClickAction::GraphCell { week, day } => {
                self.selected_graph_cell = Some((week, day));
                self.stats_breakdown_total_lines = 0;
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
            ClickAction::UsageRefresh => {
                self.last_auto_refresh = Instant::now();
                self.refresh_usage();
            }
            ClickAction::CodexStartLogin => {
                self.start_codex_login();
            }
            ClickAction::CodexDismissLogin => {
                self.dismiss_codex_login();
            }
            ClickAction::UsageSelect { index } => {
                self.selected_index = index;
                self.clamp_selection();
            }
            ClickAction::UsageToggleEmailPrivacy => {
                self.toggle_usage_email_privacy();
            }
            ClickAction::CodexUseAccount { account_id } => {
                self.confirm_codex_account_switch(&account_id);
            }
            ClickAction::CodexRemoveAccount { account_id } => {
                self.confirm_codex_account_removal(&account_id);
            }
            ClickAction::CodexResetAccount { account_id } => {
                self.confirm_codex_rate_limit_reset(&account_id);
            }
        }
    }

    pub fn is_codex_login_running(&self) -> bool {
        self.codex_login_rx.is_some()
    }

    pub fn should_show_codex_login_panel(&self) -> bool {
        self.is_codex_login_running()
            || self.codex_login_outcome.is_some()
            || !self.codex_login_lines.is_empty()
    }

    fn start_codex_login(&mut self) {
        if self.codex_login_rx.is_some() {
            self.set_status("Codex login already in progress");
            return;
        }

        self.codex_login_lines.clear();
        self.codex_login_outcome = None;

        let (tx, rx) = std::sync::mpsc::channel();
        self.codex_login_rx = Some(rx);
        let child_slot = CodexLoginChildSlot::default();
        self.codex_login_child = Some(std::sync::Arc::clone(&child_slot));
        self.set_status("Starting Codex login...");
        std::thread::spawn(move || run_codex_login_worker(tx, child_slot));
    }

    fn dismiss_codex_login(&mut self) {
        if self.codex_login_rx.is_some() {
            self.kill_codex_login_child();
            self.codex_login_rx = None;
            self.codex_login_lines.clear();
            self.codex_login_outcome = None;
            self.set_status("Codex login cancelled");
            return;
        }

        self.codex_login_lines.clear();
        self.codex_login_outcome = None;
        self.set_status("Codex login panel dismissed");
    }

    /// Kills any in-flight `codex login` child process. Called on dismiss and
    /// on TUI exit so a dangling login can't keep holding the OAuth port.
    pub fn kill_codex_login_child(&mut self) {
        let Some(slot) = self.codex_login_child.take() else {
            return;
        };
        cancel_codex_login_child(&slot);
    }

    fn confirm_codex_account_switch(&mut self, account_id: &str) {
        if self.subscription_usage.iter().any(|usage| {
            usage
                .account
                .as_ref()
                .is_some_and(|account| account.id == account_id && account.is_active)
        }) {
            self.set_status("Codex account already active");
            return;
        }

        let account_label = self.codex_account_label(account_id);
        let dialog = ConfirmDialog::codex_switch(
            account_id.to_string(),
            account_label,
            self.confirmed_codex_use_account_id.clone(),
        );
        self.dialog_stack.show(Box::new(dialog));
        self.set_status("Confirm Codex account switch");
    }

    fn confirm_codex_account_removal(&mut self, account_id: &str) {
        if self.subscription_usage.iter().any(|usage| {
            usage
                .account
                .as_ref()
                .is_some_and(|account| account.id == account_id && account.is_active)
        }) {
            self.set_status("Switch Codex accounts before removing the current account");
            return;
        }

        let account_label = self.codex_account_label(account_id);
        let dialog = ConfirmDialog::codex_remove(
            account_id.to_string(),
            account_label,
            self.confirmed_codex_remove_account_id.clone(),
        );
        self.dialog_stack.show(Box::new(dialog));
        self.set_status("Confirm Codex account removal");
    }

    fn consume_confirmed_codex_account_action(&mut self) {
        let account_id = self.confirmed_codex_use_account_id.borrow_mut().take();
        if let Some(account_id) = account_id {
            self.use_codex_account(&account_id);
            return;
        }

        let account_id = self.confirmed_codex_remove_account_id.borrow_mut().take();
        if let Some(account_id) = account_id {
            self.remove_codex_account(&account_id);
            return;
        }

        let account_id = self.confirmed_codex_reset_account_id.borrow_mut().take();
        if let Some(account_id) = account_id {
            self.reset_codex_rate_limits(&account_id);
        }
    }

    fn codex_account_label(&self, account_id: &str) -> String {
        self.subscription_usage
            .iter()
            .find_map(|usage| {
                let account = usage.account.as_ref()?;
                if account.id != account_id {
                    return None;
                }

                let label = usage
                    .account_display_name()
                    .unwrap_or_else(|| account.display_name());
                if self.hide_usage_emails && looks_like_email(&label) {
                    Some(format!("Account {}", account.short_id()))
                } else {
                    Some(label)
                }
            })
            .unwrap_or_else(|| short_account_id(account_id))
    }

    fn use_codex_account(&mut self, account_id: &str) {
        match crate::commands::usage::codex::switch_active_account(account_id) {
            Ok(info) => {
                self.mark_active_codex_account(&info.id);
                self.sort_codex_subscription_usage();
                if let Some(index) = self.subscription_usage.iter().position(|usage| {
                    usage
                        .account
                        .as_ref()
                        .is_some_and(|account| account.id == info.id)
                }) {
                    self.selected_index = index;
                    if self.selected_index < self.scroll_offset {
                        self.scroll_offset = self.selected_index;
                    } else if self.selected_index >= self.scroll_offset + self.max_visible_items {
                        self.scroll_offset = self
                            .selected_index
                            .saturating_sub(self.max_visible_items.saturating_sub(1));
                    }
                }
                self.persist_subscription_usage_cache();
                let display = info.label.as_deref().unwrap_or(&info.id);
                self.set_status(&format!("Active Codex account: {display}"));
            }
            Err(e) => {
                self.set_status(&format!("Codex account switch failed: {e}"));
            }
        }
    }

    fn toggle_usage_email_privacy(&mut self) {
        self.hide_usage_emails = !self.hide_usage_emails;
        if self.hide_usage_emails {
            self.set_status("Usage emails hidden");
        } else {
            self.set_status("Usage emails shown");
        }
    }

    fn confirm_selected_codex_rate_limit_reset(&mut self) {
        let Some(output) = self.subscription_usage.get(self.selected_index) else {
            self.set_status("No usage account selected");
            return;
        };

        if output.provider != "Codex" {
            self.set_status("Codex reset only supports Codex accounts");
            return;
        }

        let Some(account_id) = output.account.as_ref().map(|account| account.id.clone()) else {
            self.set_status("Select a saved Codex account to reset");
            return;
        };

        self.confirm_codex_rate_limit_reset(&account_id);
    }

    fn confirm_codex_rate_limit_reset(&mut self, account_id: &str) {
        if self.codex_reset_rx.is_some() {
            self.set_status("Codex reset already in progress");
            return;
        }

        let Some(output) = self.subscription_usage.iter().find(|usage| {
            usage.provider == "Codex"
                && usage
                    .account
                    .as_ref()
                    .is_some_and(|account| account.id == account_id)
        }) else {
            self.set_status("Codex account not found");
            return;
        };

        let available = output
            .reset_credits
            .as_ref()
            .map(|credits| credits.available_count)
            .unwrap_or(0);
        if available == 0 {
            self.set_status("No Codex reset credits available");
            return;
        }

        let mut account_label = self.codex_account_label(account_id);
        account_label.push_str(&format!(" - {available} reset"));
        if available != 1 {
            account_label.push('s');
        }
        if let Some(expiry) = output.reset_credits.as_ref().and_then(|credits| {
            credits
                .credits
                .iter()
                .find_map(|credit| credit.expires_at.as_ref())
        }) {
            account_label.push_str(&format!(
                " - {}",
                crate::commands::usage::helpers::format_reset_time(expiry)
                    .replace("resets", "expires")
            ));
        }

        let dialog = ConfirmDialog::codex_reset(
            account_id.to_string(),
            account_label,
            self.confirmed_codex_reset_account_id.clone(),
        );
        self.dialog_stack.show(Box::new(dialog));
        self.set_status("Confirm Codex reset credit use");
    }

    fn reset_codex_rate_limits(&mut self, account_id: &str) {
        if self.codex_reset_rx.is_some() {
            self.set_status("Codex reset already in progress");
            return;
        }

        let account_id = account_id.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.codex_reset_rx = Some(rx);
        self.set_status("Resetting Codex limits...");
        std::thread::spawn(move || {
            let result =
                crate::commands::usage::codex::consume_rate_limit_reset_credit(&account_id)
                    .map_err(|error| error.to_string());
            let _ = tx.send(result);
        });
    }

    fn remove_codex_account(&mut self, account_id: &str) {
        match crate::commands::usage::codex::remove_account(account_id) {
            Ok(info) => {
                self.subscription_usage.retain(|usage| {
                    usage.account.as_ref().map(|account| account.id.as_str())
                        != Some(info.id.as_str())
                });
                self.clamp_selection();
                if let Some(active) = crate::commands::usage::codex::list_accounts()
                    .into_iter()
                    .find(|account| account.is_active)
                {
                    self.mark_active_codex_account(&active.id);
                    self.sort_codex_subscription_usage();
                } else {
                    self.clear_active_codex_accounts();
                }
                self.persist_subscription_usage_cache();
                let display = info.label.as_deref().unwrap_or(&info.id);
                self.set_status(&format!(
                    "Stopped tracking Codex account: {display} (codex CLI login unchanged)"
                ));
            }
            Err(e) => {
                self.set_status(&format!("Codex account removal failed: {e}"));
            }
        }
    }

    fn persist_subscription_usage_cache(&self) {
        if self.subscription_usage.is_empty() {
            crate::commands::usage::clear_cache();
        } else {
            crate::commands::usage::save_cache(&self.subscription_usage);
        }
    }

    fn mark_active_codex_account(&mut self, active_account_id: &str) {
        for usage in &mut self.subscription_usage {
            if usage.provider == "Codex" {
                if let Some(account) = &mut usage.account {
                    account.is_active = account.id == active_account_id;
                }
            }
        }
    }

    fn clear_active_codex_accounts(&mut self) {
        for usage in &mut self.subscription_usage {
            if usage.provider == "Codex" {
                if let Some(account) = &mut usage.account {
                    account.is_active = false;
                }
            }
        }
    }

    fn sort_codex_subscription_usage(&mut self) {
        let mut codex_outputs = self
            .subscription_usage
            .iter()
            .filter(|usage| usage.provider == "Codex")
            .cloned()
            .collect::<Vec<_>>();
        if codex_outputs.len() < 2 {
            return;
        }

        codex_outputs.sort_by(compare_codex_usage_outputs);
        let mut sorted = codex_outputs.into_iter();
        for usage in &mut self.subscription_usage {
            if usage.provider == "Codex" {
                if let Some(next) = sorted.next() {
                    *usage = next;
                }
            }
        }
    }

    pub fn handle_mouse_event(&mut self, event: MouseEvent) {
        if self.dialog_stack.is_active() {
            self.dialog_stack.handle_mouse(event);
            self.consume_confirmed_codex_account_action();
            return;
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let x = event.column;
                let y = event.row;

                let action = self
                    .click_areas
                    .iter()
                    .find(|area| {
                        x >= area.rect.x
                            && x < area.rect.x + area.rect.width
                            && y >= area.rect.y
                            && y < area.rect.y + area.rect.height
                    })
                    .map(|area| area.action.clone());

                if let Some(action) = action {
                    self.handle_click_action(action);
                }
            }
            MouseEventKind::ScrollUp => {
                self.move_selection_up();
            }
            MouseEventKind::ScrollDown => {
                self.move_selection_down();
            }
            _ => {}
        }
    }

    /// Cache the latest terminal dimensions. `max_visible_items` is
    /// intentionally not updated here: each tab's renderer owns its own
    /// visible-item capacity and pushes the rendered count via
    /// [`Self::set_max_visible_items`] (which clamps selection and scroll
    /// state). Between resize and the next render, scroll math runs
    /// against the previous tab's capacity for one frame and self-corrects.
    pub fn handle_resize(&mut self, width: u16, height: u16) {
        self.terminal_width = width;
        self.terminal_height = height;
    }

    pub(crate) fn set_max_visible_items(&mut self, max_visible_items: usize) {
        self.max_visible_items = max_visible_items.max(1);
        self.clamp_selection();
    }

    /// Clamp selection and scroll offset to valid bounds after data/resize changes.
    /// Stats breakdown is skipped here because `render_breakdown_panel` clamps
    /// with the actual panel height (not the full-terminal `max_visible_items`).
    fn clamp_selection(&mut self) {
        if self.current_tab == Tab::Stats && self.selected_graph_cell.is_some() {
            return;
        }
        let len = self.get_current_list_len();
        if len == 0 {
            self.selected_index = 0;
            self.scroll_offset = 0;
            return;
        }
        self.selected_index = self.selected_index.min(len.saturating_sub(1));
        let max_scroll = len.saturating_sub(self.max_visible_items);
        self.scroll_offset = self.scroll_offset.min(max_scroll);
    }

    pub fn clear_click_areas(&mut self) {
        self.click_areas.clear();
    }

    pub fn add_click_area(&mut self, rect: Rect, action: ClickAction) {
        self.click_areas.push(ClickArea { rect, action });
    }

    fn reset_selection(&mut self) {
        self.scroll_offset = 0;
        self.selected_index = 0;
        self.selected_daily_detail_date = None;
        self.daily_list_selected_index = 0;
        self.daily_list_scroll_offset = 0;
        self.selected_monthly_detail_month = None;
        self.monthly_list_selected_index = 0;
        self.monthly_list_scroll_offset = 0;
        self.selected_graph_cell = None;
        self.stats_breakdown_total_lines = 0;
    }

    fn switch_tab(&mut self, target: Tab) {
        self.persist_current_sort();

        self.current_tab = target;
        if target != Tab::Daily {
            self.selected_daily_detail_date = None;
        }
        if target != Tab::Monthly {
            self.selected_monthly_detail_month = None;
        }

        let (field, dir) = self
            .tab_sort_state
            .get(&target)
            .copied()
            .unwrap_or_else(|| Self::default_sort_for_tab(target));
        self.sort_field = field;
        self.sort_direction = dir;

        self.maybe_fetch_usage_on_entry();
    }

    fn default_sort_for_tab(tab: Tab) -> (SortField, SortDirection) {
        if matches!(tab, Tab::Hourly | Tab::Minutely | Tab::Monthly) {
            (SortField::Date, SortDirection::Descending)
        } else {
            (SortField::Cost, SortDirection::Descending)
        }
    }

    pub(crate) fn tab_visible(settings: &Settings, tab: Tab) -> bool {
        match tab {
            Tab::Minutely => settings.minutely_tab_enabled,
            _ => true,
        }
    }

    pub(crate) fn is_tab_visible(&self, tab: Tab) -> bool {
        Self::tab_visible(&self.settings, tab)
    }

    fn next_visible_tab(&self) -> Tab {
        let mut candidate = self.current_tab.next();
        while !self.is_tab_visible(candidate) && candidate != self.current_tab {
            candidate = candidate.next();
        }
        candidate
    }

    fn prev_visible_tab(&self) -> Tab {
        let mut candidate = self.current_tab.prev();
        while !self.is_tab_visible(candidate) && candidate != self.current_tab {
            candidate = candidate.prev();
        }
        candidate
    }

    fn persist_current_sort(&mut self) {
        self.tab_sort_state
            .insert(self.current_tab, (self.sort_field, self.sort_direction));
    }

    fn move_selection_up(&mut self) {
        if self.current_tab == Tab::Stats && self.selected_graph_cell.is_some() {
            let len = self.get_current_list_len();
            if len == 0 {
                return;
            }

            if self.selected_index > 0 {
                self.selected_index -= 1;
                if self.selected_index < self.scroll_offset {
                    self.scroll_offset = self.selected_index;
                }
            }
            return;
        }

        let len = self.get_current_list_len();
        if len == 0 {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = len - 1;
            self.scroll_offset = len.saturating_sub(self.max_visible_items);
        } else {
            self.selected_index -= 1;
            if self.selected_index < self.scroll_offset {
                self.scroll_offset = self.selected_index;
            }
        }
    }

    fn move_selection_down(&mut self) {
        if self.current_tab == Tab::Stats && self.selected_graph_cell.is_some() {
            let len = self.get_current_list_len();
            if len == 0 {
                return;
            }

            let max_index = len - 1;
            if self.selected_index < max_index {
                self.selected_index += 1;
                if self.selected_index >= self.scroll_offset + self.max_visible_items {
                    self.scroll_offset = self.selected_index - self.max_visible_items + 1;
                }
            }
            return;
        }

        let len = self.get_current_list_len();
        if len == 0 {
            return;
        }
        let max_index = len - 1;
        if self.selected_index >= max_index {
            self.selected_index = 0;
            self.scroll_offset = 0;
        } else {
            self.selected_index += 1;
            if self.selected_index >= self.scroll_offset + self.max_visible_items {
                self.scroll_offset = self.selected_index - self.max_visible_items + 1;
            }
        }
    }

    fn move_page_up(&mut self) {
        let len = self.get_current_list_len();
        if len == 0 {
            return;
        }
        let jump = (self.max_visible_items / 2).max(1);
        self.selected_index = self.selected_index.saturating_sub(jump);
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }
    }

    fn move_page_down(&mut self) {
        let len = self.get_current_list_len();
        if len == 0 {
            return;
        }
        let jump = (self.max_visible_items / 2).max(1);
        let max_index = len - 1;
        self.selected_index = (self.selected_index + jump).min(max_index);
        if self.selected_index >= self.scroll_offset + self.max_visible_items {
            self.scroll_offset = self.selected_index - self.max_visible_items + 1;
        }
    }

    fn move_to_top(&mut self) {
        let len = self.get_current_list_len();
        if len == 0 {
            return;
        }
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    fn move_to_bottom(&mut self) {
        let len = self.get_current_list_len();
        if len == 0 {
            return;
        }
        self.selected_index = len - 1;
        self.scroll_offset = len.saturating_sub(self.max_visible_items);
    }

    fn get_current_list_len(&self) -> usize {
        match self.current_tab {
            Tab::Overview | Tab::Models => self.data.models.len(),
            Tab::Agents => self.data.agents.len(),
            Tab::Daily if self.is_daily_detail_active() => {
                self.get_sorted_daily_detail_rows().len()
            }
            Tab::Daily => self.data.daily.len(),
            Tab::Hourly => self.data.hourly.len(),
            Tab::Minutely => self.data.minutely.len(),
            Tab::Monthly if self.is_monthly_detail_active() => {
                self.get_sorted_monthly_detail_days().len()
            }
            Tab::Monthly => self.data.monthly.len(),
            Tab::Stats => {
                if self.selected_graph_cell.is_some() {
                    self.stats_breakdown_total_lines
                } else {
                    0
                }
            }
            Tab::Usage => self
                .subscription_usage
                .iter()
                .map(|u| u.metrics.len())
                .sum(),
        }
    }

    fn set_sort(&mut self, field: SortField) {
        if self.sort_field == field {
            self.sort_direction = match self.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            self.sort_field = field;
            self.sort_direction = SortDirection::Descending;
        }
        self.persist_current_sort();
        if (self.current_tab == Tab::Daily && self.is_daily_detail_active())
            || (self.current_tab == Tab::Monthly && self.is_monthly_detail_active())
        {
            self.selected_index = 0;
            self.scroll_offset = 0;
        } else {
            self.reset_selection();
        }
        self.set_status(&format!(
            "Sorted by {:?} {:?}",
            self.sort_field, self.sort_direction
        ));
    }

    fn jump_to_today(&mut self) {
        if self.current_tab != Tab::Daily {
            return;
        }
        self.selected_daily_detail_date = None;

        let today = chrono::Local::now().date_naive();
        let (today_index, total_len) = {
            let sorted_daily = self.get_sorted_daily();
            (
                sorted_daily.iter().position(|d| d.date == today),
                sorted_daily.len(),
            )
        };

        if let Some(index) = today_index {
            self.selected_index = index;

            if self.max_visible_items > 0 {
                let max_scroll = total_len.saturating_sub(self.max_visible_items);
                self.scroll_offset = index
                    .saturating_sub(self.max_visible_items / 2)
                    .min(max_scroll);
            } else {
                self.scroll_offset = 0;
            }

            self.selected_graph_cell = None;
            self.set_status("Jumped to today's usage");
        } else {
            self.set_status("No usage recorded for today");
        }
    }

    fn cycle_theme(&mut self) {
        let new_theme = self.theme.name.next();
        self.theme = Theme::from_name_for_current_terminal(new_theme);
        self.dialog_stack.set_theme(self.theme.clone());
        self.settings.set_theme(new_theme);
        if let Err(e) = self.settings.save() {
            self.set_status(&format!(
                "Theme: {} (save failed: {})",
                new_theme.as_str(),
                e
            ));
        } else {
            self.set_status(&format!("Theme: {}", new_theme.as_str()));
        }
    }

    fn open_client_picker(&mut self) {
        let dialog = ClientPickerDialog::new(
            self.enabled_clients.clone(),
            self.dialog_needs_reload.clone(),
        );
        self.dialog_stack.show(Box::new(dialog));
    }

    /// Project the unified `HashSet<ClientFilter>` into the
    /// `Vec<ClientId>` shape that `tokscale_core` scanners still consume.
    /// `ClientFilter::Synthetic` does not have a `ClientId` and is
    /// excluded from this projection — use [`Self::include_synthetic`]
    /// for that signal.
    pub fn scan_clients(&self) -> Vec<ClientId> {
        let mut out: Vec<ClientId> = self
            .enabled_clients
            .borrow()
            .iter()
            .filter_map(|f| f.to_client_id())
            .collect();
        // Stable order for downstream cache key + log output. Sort by the
        // declaration index in ClientId::ALL so the projection mirrors
        // the canonical ordering used elsewhere.
        out.sort_by_key(|c| *c as usize);
        out
    }

    /// Whether the user has Synthetic enabled. Boundary helper for code
    /// paths that still take a separate `bool include_synthetic` argument.
    pub fn include_synthetic(&self) -> bool {
        self.enabled_clients
            .borrow()
            .contains(&ClientFilter::Synthetic)
    }

    fn open_group_by_picker(&mut self) {
        use super::ui::dialog::GroupByPickerDialog;
        let dialog =
            GroupByPickerDialog::new(self.group_by.clone(), self.dialog_needs_reload.clone());
        self.dialog_stack.show(Box::new(dialog));
    }

    fn open_selected_daily_detail(&mut self) {
        if self.is_daily_detail_active() {
            return;
        }

        let selected_date = {
            let daily = self.get_sorted_daily();
            daily.get(self.selected_index).map(|day| day.date)
        };

        if let Some(date) = selected_date {
            self.daily_list_selected_index = self.selected_index;
            self.daily_list_scroll_offset = self.scroll_offset;
            self.selected_daily_detail_date = Some(date);
            self.selected_index = 0;
            self.scroll_offset = 0;
            self.set_status(&format!("Viewing daily details for {}", date));
            self.clamp_selection();
        }
    }

    fn close_daily_detail(&mut self) {
        let Some(detail_date) = self.selected_daily_detail_date else {
            return;
        };

        self.selected_daily_detail_date = None;

        // Re-anchor by date so a sort change inside detail mode still
        // restores the same day rather than the stale list index.
        let restored_index = self
            .get_sorted_daily()
            .iter()
            .position(|day| day.date == detail_date)
            .unwrap_or(self.daily_list_selected_index);

        self.selected_index = restored_index;

        let max_visible = self.max_visible_items.max(1);
        let viewport_still_holds = restored_index >= self.daily_list_scroll_offset
            && restored_index < self.daily_list_scroll_offset + max_visible;
        self.scroll_offset = if viewport_still_holds {
            self.daily_list_scroll_offset
        } else {
            restored_index.saturating_sub(max_visible / 2)
        };

        self.set_status("Returned to daily usage");
        self.clamp_selection();
    }

    fn open_selected_monthly_detail(&mut self) {
        if self.is_monthly_detail_active() {
            return;
        }

        let selected_month = {
            let monthly = self.get_sorted_monthly();
            monthly.get(self.selected_index).map(|m| m.month.clone())
        };

        if let Some(month) = selected_month {
            self.monthly_list_selected_index = self.selected_index;
            self.monthly_list_scroll_offset = self.scroll_offset;
            self.selected_monthly_detail_month = Some(month.clone());
            self.selected_index = 0;
            self.scroll_offset = 0;
            self.set_status(&format!("Viewing daily breakdown for {}", month));
            self.clamp_selection();
        }
    }

    fn close_monthly_detail(&mut self) {
        let Some(ref detail_month) = self.selected_monthly_detail_month else {
            return;
        };
        let detail_month = detail_month.clone();

        self.selected_monthly_detail_month = None;

        let restored_index = self
            .get_sorted_monthly()
            .iter()
            .position(|m| m.month == detail_month)
            .unwrap_or(self.monthly_list_selected_index);

        self.selected_index = restored_index;

        let max_visible = self.max_visible_items.max(1);
        let viewport_still_holds = restored_index >= self.monthly_list_scroll_offset
            && restored_index < self.monthly_list_scroll_offset + max_visible;
        self.scroll_offset = if viewport_still_holds {
            self.monthly_list_scroll_offset
        } else {
            restored_index.saturating_sub(max_visible / 2)
        };

        self.set_status("Returned to monthly usage");
        self.clamp_selection();
    }

    fn toggle_auto_refresh(&mut self) {
        self.auto_refresh = !self.auto_refresh;
        if self.auto_refresh {
            self.last_auto_refresh = Instant::now();
        }
        self.settings.auto_refresh_enabled = self.auto_refresh;
        let save_result = self.settings.save();
        let msg = if self.auto_refresh {
            format!(
                "Auto-refresh ON ({}s)",
                self.auto_refresh_interval.as_secs()
            )
        } else {
            "Auto-refresh OFF".to_string()
        };
        if let Err(e) = save_result {
            self.set_status(&format!("{} (save failed: {})", msg, e));
        } else {
            self.set_status(&msg);
        }
    }

    fn increase_refresh_interval(&mut self) {
        let ms = self.auto_refresh_interval.as_millis() as u64;
        let new_ms = ms.saturating_add(10_000).min(300_000);
        self.auto_refresh_interval = Duration::from_millis(new_ms);
        self.settings.auto_refresh_ms = new_ms;
        let save_result = self.settings.save();
        let msg = format!("Refresh interval: {}s", new_ms / 1000);
        if let Err(e) = save_result {
            self.set_status(&format!("{} (save failed: {})", msg, e));
        } else {
            self.set_status(&msg);
        }
    }

    fn decrease_refresh_interval(&mut self) {
        let ms = self.auto_refresh_interval.as_millis() as u64;
        let new_ms = ms.saturating_sub(10_000).max(30_000);
        self.auto_refresh_interval = Duration::from_millis(new_ms);
        self.settings.auto_refresh_ms = new_ms;
        let save_result = self.settings.save();
        let msg = format!("Refresh interval: {}s", new_ms / 1000);
        if let Err(e) = save_result {
            self.set_status(&format!("{} (save failed: {})", msg, e));
        } else {
            self.set_status(&msg);
        }
    }

    fn copy_selected_to_clipboard(&mut self) {
        let text = match self.current_tab {
            Tab::Overview | Tab::Models => self
                .get_sorted_models()
                .get(self.selected_index)
                .map(|m| format!("{}: {} tokens, ${:.4}", m.model, m.tokens.total(), m.cost)),
            Tab::Agents => self
                .get_sorted_agents()
                .get(self.selected_index)
                .map(|a| format!("{}: {} tokens, ${:.4}", a.agent, a.tokens.total(), a.cost)),
            Tab::Daily if self.is_daily_detail_active() => self
                .get_sorted_daily_detail_rows()
                .get(self.selected_index)
                .map(|row| {
                    format!(
                        "{} / {}: {} tokens, ${:.4}",
                        row.source,
                        row.model,
                        row.tokens.total(),
                        row.cost
                    )
                }),
            Tab::Daily => self
                .get_sorted_daily()
                .get(self.selected_index)
                .map(|d| format!("{}: {} tokens, ${:.4}", d.date, d.tokens.total(), d.cost)),
            Tab::Hourly => self.get_sorted_hourly().get(self.selected_index).map(|h| {
                format!(
                    "{}: {} tokens, ${:.4}",
                    h.datetime.format("%Y-%m-%d %H:%M"),
                    h.tokens.total(),
                    h.cost
                )
            }),
            Tab::Minutely => self
                .get_sorted_minutely()
                .get(self.selected_index)
                .map(|m| {
                    format!(
                        "{}: {} tokens, ${:.4}",
                        m.datetime.format("%Y-%m-%d %H:%M"),
                        m.tokens.total(),
                        m.cost
                    )
                }),
            Tab::Monthly if self.is_monthly_detail_active() => self
                .get_sorted_monthly_detail_days()
                .get(self.selected_index)
                .map(|d| format!("{}: {} tokens, ${:.4}", d.date, d.tokens.total(), d.cost)),
            Tab::Monthly => self
                .get_sorted_monthly()
                .get(self.selected_index)
                .map(|m| format!("{}: {} tokens, ${:.4}", m.month, m.tokens.total(), m.cost)),
            Tab::Stats | Tab::Usage => None,
        };

        if let Some(text) = text {
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&text)) {
                Ok(_) => self.set_status("Copied to clipboard"),
                Err(_) => self.set_status("Failed to copy"),
            }
        }
    }

    fn export_to_json(&mut self) {
        let filename = format!(
            "tokscale-export-{}.json",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );

        match super::export::build_export_json(&self.data) {
            Ok(json) => match std::fs::write(&filename, json) {
                Ok(_) => self.set_status(&format!("Exported to {}", filename)),
                Err(e) => self.set_status(&format!("Export failed: {}", e)),
            },
            Err(e) => self.set_status(&format!("Export failed: {}", e)),
        }
    }

    fn handle_graph_selection(&mut self) {
        if self.current_tab == Tab::Stats && self.selected_graph_cell.is_some() {
            self.set_status("Press ESC to deselect");
        }
    }

    pub fn set_status(&mut self, message: &str) {
        self.status_message = Some(message.to_string());
        self.status_message_time = Some(Instant::now());
    }

    pub fn get_sorted_models(&self) -> Vec<&ModelUsage> {
        let mut models: Vec<&ModelUsage> = self.data.models.iter().collect();

        let tie_breaker = |a: &&ModelUsage, b: &&ModelUsage| {
            a.model
                .cmp(&b.model)
                .then_with(|| a.workspace_label.cmp(&b.workspace_label))
                .then_with(|| a.workspace_key.cmp(&b.workspace_key))
                .then_with(|| a.provider.cmp(&b.provider))
                .then_with(|| a.client.cmp(&b.client))
        };

        match (self.sort_field, self.sort_direction) {
            (SortField::Cost, SortDirection::Descending) => {
                models.sort_by(|a, b| b.cost.total_cmp(&a.cost).then_with(|| tie_breaker(a, b)))
            }
            (SortField::Cost, SortDirection::Ascending) => {
                models.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| tie_breaker(a, b)))
            }
            (SortField::Tokens, SortDirection::Descending) => models.sort_by(|a, b| {
                b.tokens
                    .total()
                    .cmp(&a.tokens.total())
                    .then_with(|| tie_breaker(a, b))
            }),
            (SortField::Tokens, SortDirection::Ascending) => models.sort_by(|a, b| {
                a.tokens
                    .total()
                    .cmp(&b.tokens.total())
                    .then_with(|| tie_breaker(a, b))
            }),
            (SortField::Date, _) => {
                models.sort_by(|a, b| tie_breaker(a, b));
            }
        }

        models
    }

    pub fn get_sorted_agents(&self) -> Vec<&AgentUsage> {
        let mut agents: Vec<&AgentUsage> = self.data.agents.iter().collect();

        let tie_breaker = |a: &&AgentUsage, b: &&AgentUsage| {
            a.agent
                .cmp(&b.agent)
                .then_with(|| a.clients.cmp(&b.clients))
        };

        match (self.sort_field, self.sort_direction) {
            (SortField::Cost, SortDirection::Descending) => {
                agents.sort_by(|a, b| b.cost.total_cmp(&a.cost).then_with(|| tie_breaker(a, b)))
            }
            (SortField::Cost, SortDirection::Ascending) => {
                agents.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| tie_breaker(a, b)))
            }
            (SortField::Tokens, SortDirection::Descending) => agents.sort_by(|a, b| {
                b.tokens
                    .total()
                    .cmp(&a.tokens.total())
                    .then_with(|| tie_breaker(a, b))
            }),
            (SortField::Tokens, SortDirection::Ascending) => agents.sort_by(|a, b| {
                a.tokens
                    .total()
                    .cmp(&b.tokens.total())
                    .then_with(|| tie_breaker(a, b))
            }),
            (SortField::Date, _) => {
                agents.sort_by(|a, b| tie_breaker(a, b));
            }
        }

        agents
    }

    pub fn get_sorted_daily(&self) -> Vec<&DailyUsage> {
        let mut daily: Vec<&DailyUsage> = self.data.daily.iter().collect();

        match (self.sort_field, self.sort_direction) {
            (SortField::Cost, SortDirection::Descending) => {
                daily.sort_by(|a, b| b.cost.total_cmp(&a.cost).then_with(|| a.date.cmp(&b.date)))
            }
            (SortField::Cost, SortDirection::Ascending) => {
                daily.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.date.cmp(&b.date)))
            }
            (SortField::Tokens, SortDirection::Descending) => daily.sort_by(|a, b| {
                b.tokens
                    .total()
                    .cmp(&a.tokens.total())
                    .then_with(|| a.date.cmp(&b.date))
            }),
            (SortField::Tokens, SortDirection::Ascending) => daily.sort_by(|a, b| {
                a.tokens
                    .total()
                    .cmp(&b.tokens.total())
                    .then_with(|| a.date.cmp(&b.date))
            }),
            (SortField::Date, SortDirection::Descending) => {
                daily.sort_by_key(|b| std::cmp::Reverse(b.date))
            }
            (SortField::Date, SortDirection::Ascending) => daily.sort_by_key(|a| a.date),
        }

        daily
    }

    pub fn is_daily_detail_active(&self) -> bool {
        self.selected_daily_detail_date.is_some()
    }

    pub fn daily_detail_date(&self) -> Option<NaiveDate> {
        self.selected_daily_detail_date
    }

    pub fn is_monthly_detail_active(&self) -> bool {
        self.selected_monthly_detail_month.is_some()
    }

    pub fn monthly_detail_month(&self) -> Option<&str> {
        self.selected_monthly_detail_month.as_deref()
    }

    pub fn get_sorted_monthly_detail_days(&self) -> Vec<&DailyUsage> {
        let Some(month) = self.selected_monthly_detail_month.as_ref() else {
            return Vec::new();
        };
        let Some((year, month)) = month.split_once('-').and_then(|(year, month)| {
            Some((year.parse::<i32>().ok()?, month.parse::<u32>().ok()?))
        }) else {
            return Vec::new();
        };

        self.get_sorted_daily()
            .into_iter()
            .filter(|day| day.date.year() == year && day.date.month() == month)
            .collect()
    }

    pub fn get_sorted_daily_detail_rows(&self) -> Vec<DailyDetailRow<'_>> {
        let Some(date) = self.selected_daily_detail_date else {
            return Vec::new();
        };
        let Some(day) = self.data.daily.iter().find(|day| day.date == date) else {
            return Vec::new();
        };

        let mut rows: Vec<DailyDetailRow<'_>> = day
            .source_breakdown
            .iter()
            .flat_map(|(source, source_info)| {
                source_info
                    .models
                    .values()
                    .map(move |model_info| DailyDetailRow {
                        source,
                        provider: &model_info.provider,
                        model: &model_info.display_name,
                        color_key: &model_info.color_key,
                        tokens: &model_info.tokens,
                        cost: model_info.cost,
                        messages: model_info.messages,
                    })
            })
            .collect();

        let tie_breaker = |a: &DailyDetailRow<'_>, b: &DailyDetailRow<'_>| {
            a.source
                .cmp(b.source)
                .then_with(|| a.model.cmp(b.model))
                .then_with(|| a.provider.cmp(b.provider))
        };

        match (self.sort_field, self.sort_direction) {
            (SortField::Cost, SortDirection::Descending) => {
                rows.sort_by(|a, b| b.cost.total_cmp(&a.cost).then_with(|| tie_breaker(a, b)))
            }
            (SortField::Cost, SortDirection::Ascending) => {
                rows.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| tie_breaker(a, b)))
            }
            (SortField::Tokens, SortDirection::Descending) => rows.sort_by(|a, b| {
                b.tokens
                    .total()
                    .cmp(&a.tokens.total())
                    .then_with(|| tie_breaker(a, b))
            }),
            (SortField::Tokens, SortDirection::Ascending) => rows.sort_by(|a, b| {
                a.tokens
                    .total()
                    .cmp(&b.tokens.total())
                    .then_with(|| tie_breaker(a, b))
            }),
            (SortField::Date, _) => rows.sort_by(tie_breaker),
        }

        rows
    }

    pub fn get_sorted_hourly(&self) -> Vec<&HourlyUsage> {
        let mut hourly: Vec<&HourlyUsage> = self.data.hourly.iter().collect();

        match (self.sort_field, self.sort_direction) {
            (SortField::Cost, SortDirection::Descending) => hourly.sort_by(|a, b| {
                b.cost
                    .total_cmp(&a.cost)
                    .then_with(|| a.datetime.cmp(&b.datetime))
            }),
            (SortField::Cost, SortDirection::Ascending) => hourly.sort_by(|a, b| {
                a.cost
                    .total_cmp(&b.cost)
                    .then_with(|| a.datetime.cmp(&b.datetime))
            }),
            (SortField::Tokens, SortDirection::Descending) => hourly.sort_by(|a, b| {
                b.tokens
                    .total()
                    .cmp(&a.tokens.total())
                    .then_with(|| a.datetime.cmp(&b.datetime))
            }),
            (SortField::Tokens, SortDirection::Ascending) => hourly.sort_by(|a, b| {
                a.tokens
                    .total()
                    .cmp(&b.tokens.total())
                    .then_with(|| a.datetime.cmp(&b.datetime))
            }),
            (SortField::Date, SortDirection::Descending) => {
                hourly.sort_by_key(|b| std::cmp::Reverse(b.datetime))
            }
            (SortField::Date, SortDirection::Ascending) => hourly.sort_by_key(|a| a.datetime),
        }

        hourly
    }

    pub fn get_sorted_minutely(&self) -> Vec<&MinutelyUsage> {
        let sort_field = self.sort_field;
        let sort_direction = self.sort_direction;
        let data_version = self.data_version;
        let data_len = self.data.minutely.len();

        let cached_indices = {
            let cache = self.minutely_sort_cache.borrow();
            cache
                .as_ref()
                .filter(|cache| {
                    cache.sort_field == sort_field
                        && cache.sort_direction == sort_direction
                        && cache.data_version == data_version
                        && cache.data_len == data_len
                })
                .map(|cache| cache.indices.clone())
        };

        let indices = if let Some(indices) = cached_indices {
            indices
        } else {
            let mut indices: Vec<usize> = (0..data_len).collect();

            match (sort_field, sort_direction) {
                (SortField::Cost, SortDirection::Descending) => indices.sort_by(|a, b| {
                    let a = &self.data.minutely[*a];
                    let b = &self.data.minutely[*b];
                    b.cost
                        .total_cmp(&a.cost)
                        .then_with(|| a.datetime.cmp(&b.datetime))
                }),
                (SortField::Cost, SortDirection::Ascending) => indices.sort_by(|a, b| {
                    let a = &self.data.minutely[*a];
                    let b = &self.data.minutely[*b];
                    a.cost
                        .total_cmp(&b.cost)
                        .then_with(|| a.datetime.cmp(&b.datetime))
                }),
                (SortField::Tokens, SortDirection::Descending) => indices.sort_by(|a, b| {
                    let a = &self.data.minutely[*a];
                    let b = &self.data.minutely[*b];
                    b.tokens
                        .total()
                        .cmp(&a.tokens.total())
                        .then_with(|| a.datetime.cmp(&b.datetime))
                }),
                (SortField::Tokens, SortDirection::Ascending) => indices.sort_by(|a, b| {
                    let a = &self.data.minutely[*a];
                    let b = &self.data.minutely[*b];
                    a.tokens
                        .total()
                        .cmp(&b.tokens.total())
                        .then_with(|| a.datetime.cmp(&b.datetime))
                }),
                (SortField::Date, SortDirection::Descending) => indices
                    .sort_by_key(|index| std::cmp::Reverse(self.data.minutely[*index].datetime)),
                (SortField::Date, SortDirection::Ascending) => {
                    indices.sort_by_key(|index| self.data.minutely[*index].datetime)
                }
            }

            *self.minutely_sort_cache.borrow_mut() = Some(MinutelySortCache {
                sort_field,
                sort_direction,
                data_version,
                data_len,
                indices: indices.clone(),
            });

            indices
        };

        indices
            .into_iter()
            .map(|index| &self.data.minutely[index])
            .collect()
    }

    pub fn get_sorted_monthly(&self) -> Vec<&MonthlyUsage> {
        let mut monthly: Vec<&MonthlyUsage> = self.data.monthly.iter().collect();

        match (self.sort_field, self.sort_direction) {
            (SortField::Cost, SortDirection::Descending) => monthly.sort_by(|a, b| {
                b.cost
                    .total_cmp(&a.cost)
                    .then_with(|| b.month.cmp(&a.month))
            }),
            (SortField::Cost, SortDirection::Ascending) => monthly.sort_by(|a, b| {
                a.cost
                    .total_cmp(&b.cost)
                    .then_with(|| a.month.cmp(&b.month))
            }),
            (SortField::Tokens, SortDirection::Descending) => monthly.sort_by(|a, b| {
                b.tokens
                    .total()
                    .cmp(&a.tokens.total())
                    .then_with(|| b.month.cmp(&a.month))
            }),
            (SortField::Tokens, SortDirection::Ascending) => monthly.sort_by(|a, b| {
                a.tokens
                    .total()
                    .cmp(&b.tokens.total())
                    .then_with(|| a.month.cmp(&b.month))
            }),
            (SortField::Date, SortDirection::Descending) => {
                monthly.sort_by(|a, b| b.month.cmp(&a.month))
            }
            (SortField::Date, SortDirection::Ascending) => {
                monthly.sort_by(|a, b| a.month.cmp(&b.month))
            }
        }

        monthly
    }

    pub fn is_narrow(&self) -> bool {
        self.terminal_width < 80
    }

    pub fn is_very_narrow(&self) -> bool {
        self.terminal_width < 60
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui::widgets::get_provider_shade;
    use super::*;
    use crate::commands::usage::{
        UsageAccount, UsageFetchDiagnostic, UsageFetchReport, UsageMetric, UsageOutput,
    };
    use crate::tui::data::{DailyModelInfo, DailySourceInfo, ModelUsage, TokenBreakdown};
    use chrono::{NaiveDate, NaiveDateTime};
    use std::collections::{BTreeMap, BTreeSet};
    use std::{env, fs};

    #[test]
    fn test_tab_all() {
        let tabs = Tab::all();
        assert_eq!(tabs.len(), 9);
        assert_eq!(tabs[0], Tab::Overview);
        assert_eq!(tabs[1], Tab::Usage);
        assert_eq!(tabs[2], Tab::Models);
        assert_eq!(tabs[3], Tab::Daily);
        assert_eq!(tabs[4], Tab::Hourly);
        assert_eq!(tabs[5], Tab::Minutely);
        assert_eq!(tabs[6], Tab::Monthly);
        assert_eq!(tabs[7], Tab::Stats);
        assert_eq!(tabs[8], Tab::Agents);
    }

    #[test]
    fn test_tab_next() {
        assert_eq!(Tab::Overview.next(), Tab::Usage);
        assert_eq!(Tab::Usage.next(), Tab::Models);
        assert_eq!(Tab::Models.next(), Tab::Daily);
        assert_eq!(Tab::Daily.next(), Tab::Hourly);
        assert_eq!(Tab::Hourly.next(), Tab::Minutely);
        assert_eq!(Tab::Minutely.next(), Tab::Monthly);
        assert_eq!(Tab::Monthly.next(), Tab::Stats);
        assert_eq!(Tab::Stats.next(), Tab::Agents);
        assert_eq!(Tab::Agents.next(), Tab::Overview);
    }

    #[test]
    fn test_tab_prev() {
        assert_eq!(Tab::Overview.prev(), Tab::Agents);
        assert_eq!(Tab::Usage.prev(), Tab::Overview);
        assert_eq!(Tab::Models.prev(), Tab::Usage);
        assert_eq!(Tab::Daily.prev(), Tab::Models);
        assert_eq!(Tab::Hourly.prev(), Tab::Daily);
        assert_eq!(Tab::Minutely.prev(), Tab::Hourly);
        assert_eq!(Tab::Monthly.prev(), Tab::Minutely);
        assert_eq!(Tab::Stats.prev(), Tab::Monthly);
        assert_eq!(Tab::Agents.prev(), Tab::Stats);
    }

    #[test]
    fn test_tab_as_str() {
        assert_eq!(Tab::Overview.as_str(), "Overview");
        assert_eq!(Tab::Models.as_str(), "Models");
        assert_eq!(Tab::Agents.as_str(), "Agents");
        assert_eq!(Tab::Daily.as_str(), "Daily");
        assert_eq!(Tab::Hourly.as_str(), "Hourly");
        assert_eq!(Tab::Minutely.as_str(), "Minutely");
        assert_eq!(Tab::Monthly.as_str(), "Monthly");
        assert_eq!(Tab::Stats.as_str(), "Stats");
    }

    #[test]
    fn test_tab_short_name() {
        assert_eq!(Tab::Overview.short_name(), "Ovw");
        assert_eq!(Tab::Models.short_name(), "Mod");
        assert_eq!(Tab::Agents.short_name(), "Agt");
        assert_eq!(Tab::Daily.short_name(), "Day");
        assert_eq!(Tab::Hourly.short_name(), "Hr");
        assert_eq!(Tab::Minutely.short_name(), "Min");
        assert_eq!(Tab::Monthly.short_name(), "Mon");
        assert_eq!(Tab::Stats.short_name(), "Sta");
    }

    #[test]
    fn test_reset_selection() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();

        app.selected_index = 5;
        app.scroll_offset = 3;
        app.selected_graph_cell = Some((2, 4));

        app.reset_selection();

        assert_eq!(app.selected_index, 0);
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.selected_graph_cell, None);
    }

    #[test]
    fn test_move_selection_up() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();

        // Add some mock data
        app.data.models = vec![
            ModelUsage {
                model: "model1".to_string(),
                provider: "provider1".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                performance: Default::default(),
                session_count: 1,
                workspace_key: None,
                workspace_label: None,
            },
            ModelUsage {
                model: "model2".to_string(),
                provider: "provider2".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                performance: Default::default(),
                session_count: 1,
                workspace_key: None,
                workspace_label: None,
            },
        ];

        app.selected_index = 1;
        app.move_selection_up();
        assert_eq!(app.selected_index, 0);

        // At top boundary - wraps to last item (index 1)
        app.move_selection_up();
        assert_eq!(app.selected_index, 1);
    }

    #[test]
    fn test_move_selection_down() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();

        // Add some mock data
        app.data.models = vec![
            ModelUsage {
                model: "model1".to_string(),
                provider: "provider1".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                performance: Default::default(),
                session_count: 1,
                workspace_key: None,
                workspace_label: None,
            },
            ModelUsage {
                model: "model2".to_string(),
                provider: "provider2".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                performance: Default::default(),
                session_count: 1,
                workspace_key: None,
                workspace_label: None,
            },
        ];

        app.selected_index = 0;
        app.move_selection_down();
        assert_eq!(app.selected_index, 1);

        // At bottom boundary - wraps to first item (index 0)
        app.move_selection_down();
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_clamp_selection() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();

        // Add some mock data
        app.data.models = vec![ModelUsage {
            model: "model1".to_string(),
            provider: "provider1".to_string(),
            client: "opencode".to_string(),
            tokens: TokenBreakdown::default(),
            cost: 0.0,
            performance: Default::default(),
            session_count: 1,
            workspace_key: None,
            workspace_label: None,
        }];

        // Set selection beyond bounds
        app.selected_index = 10;
        app.clamp_selection();
        assert_eq!(app.selected_index, 0);

        // Empty data
        app.data.models.clear();
        app.selected_index = 5;
        app.clamp_selection();
        assert_eq!(app.selected_index, 0);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_set_sort() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();

        // Initial state
        assert_eq!(app.sort_field, SortField::Cost);
        assert_eq!(app.sort_direction, SortDirection::Descending);

        // Change to different field
        app.set_sort(SortField::Tokens);
        assert_eq!(app.sort_field, SortField::Tokens);
        assert_eq!(app.sort_direction, SortDirection::Descending);

        // Toggle same field
        app.set_sort(SortField::Tokens);
        assert_eq!(app.sort_field, SortField::Tokens);
        assert_eq!(app.sort_direction, SortDirection::Ascending);

        // Toggle again
        app.set_sort(SortField::Tokens);
        assert_eq!(app.sort_field, SortField::Tokens);
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    #[test]
    fn test_should_quit() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let app = App::new_with_cached_data(config, None).unwrap();

        assert!(!app.should_quit);
    }

    #[test]
    #[serial_test::serial]
    fn app_uses_saved_theme_when_cli_theme_is_absent() {
        let temp = tempfile::TempDir::new().unwrap();
        let previous_config_dir = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", temp.path());
        }
        fs::write(
            temp.path().join("settings.json"),
            r#"{"colorPalette":"halloween"}"#,
        )
        .unwrap();

        let config = TuiConfig {
            theme: String::new(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let app = App::new_with_cached_data(config, None).unwrap();

        unsafe {
            match previous_config_dir {
                Some(value) => env::set_var("TOKSCALE_CONFIG_DIR", value),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
        assert_eq!(app.theme.name, ThemeName::Halloween);
    }

    #[test]
    #[serial_test::serial]
    fn app_explicit_cli_theme_overrides_saved_theme() {
        let temp = tempfile::TempDir::new().unwrap();
        let previous_config_dir = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", temp.path());
        }
        fs::write(
            temp.path().join("settings.json"),
            r#"{"colorPalette":"halloween"}"#,
        )
        .unwrap();

        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        let app = App::new_with_cached_data(config, None).unwrap();

        unsafe {
            match previous_config_dir {
                Some(value) => env::set_var("TOKSCALE_CONFIG_DIR", value),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
        assert_eq!(app.theme.name, ThemeName::Blue);
    }

    // ── Helper ──────────────────────────────────────────────────────

    fn make_app() -> App {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
        };
        App::new_with_cached_data(config, None).unwrap()
    }

    fn usage_output(provider: &str, account: Option<UsageAccount>) -> UsageOutput {
        UsageOutput {
            provider: provider.to_string(),
            account,
            plan: Some("Pro".to_string()),
            email: None,
            metrics: vec![UsageMetric {
                label: "Session".to_string(),
                used_percent: 20.0,
                remaining_percent: 80.0,
                remaining_label: Some("80% left".to_string()),
                resets_at: None,
            }],
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        }
    }

    fn sample_subscription_usage() -> Vec<UsageOutput> {
        vec![usage_output(
            "Codex",
            Some(UsageAccount {
                id: "acct_work".to_string(),
                label: Some("work".to_string()),
                is_active: true,
            }),
        )]
    }

    fn sample_usage_fetcher() -> UsageFetchReport {
        UsageFetchReport {
            outputs: sample_subscription_usage(),
            diagnostics: Vec::new(),
        }
    }

    fn failing_usage_fetcher() -> UsageFetchReport {
        UsageFetchReport {
            outputs: Vec::new(),
            diagnostics: vec![UsageFetchDiagnostic::new(
                "Codex",
                None,
                "token refresh failed",
            )],
        }
    }

    fn partial_usage_fetcher() -> UsageFetchReport {
        UsageFetchReport {
            outputs: sample_subscription_usage(),
            diagnostics: vec![UsageFetchDiagnostic::new(
                "Codex",
                Some(UsageAccount {
                    id: "acct_personal".to_string(),
                    label: Some("personal".to_string()),
                    is_active: false,
                }),
                "usage endpoint rejected credentials",
            )],
        }
    }

    fn drain_usage_fetch(app: &mut App) {
        for _ in 0..20 {
            app.on_tick();
            if !app.is_fetching_usage() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn test_codex_usage_sort_moves_active_account_to_first_codex_row() {
        let mut app = make_app();
        app.subscription_usage = vec![
            usage_output("Claude", None),
            usage_output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_work".to_string(),
                    label: Some("work".to_string()),
                    is_active: true,
                }),
            ),
            usage_output("Warp/Oz", None),
            usage_output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_personal".to_string(),
                    label: Some("personal".to_string()),
                    is_active: false,
                }),
            ),
        ];

        app.mark_active_codex_account("acct_personal");
        app.sort_codex_subscription_usage();

        assert_eq!(app.subscription_usage[0].provider, "Claude");
        assert_eq!(app.subscription_usage[2].provider, "Warp/Oz");
        let codex_ids = app
            .subscription_usage
            .iter()
            .filter(|usage| usage.provider == "Codex")
            .filter_map(|usage| usage.account.as_ref().map(|account| account.id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(codex_ids, vec!["acct_personal", "acct_work"]);
        assert!(app.subscription_usage[1]
            .account
            .as_ref()
            .is_some_and(|account| account.is_active));
    }

    #[test]
    fn test_app_no_filter_default_matches_default_set() {
        // Regression for an Oracle-flagged HIGH bug: the no-filter TUI
        // default and the `submit` warm-cache filter set drifted apart,
        // making every TUI launch after submit a stale-cache reuse
        // instead of a fresh hit. Both paths now go through
        // `ClientFilter::default_set()`; assert it stays that way.
        let app = make_app();
        let actual = app.enabled_clients.borrow().clone();
        let expected = ClientFilter::default_set();
        assert_eq!(
            actual, expected,
            "no-filter App default drifted from ClientFilter::default_set() — \
             warm cache and TUI launch will mismatch"
        );
        assert!(
            !actual.contains(&ClientFilter::Synthetic),
            "no-filter default must not include Synthetic (opt-in only)"
        );
    }

    fn make_app_with_models(n: usize) -> App {
        let mut app = make_app();
        app.data.models = (0..n)
            .map(|i| ModelUsage {
                model: format!("model{}", i),
                provider: "provider".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                performance: Default::default(),
                session_count: 1,
                workspace_key: None,
                workspace_label: None,
            })
            .collect();
        app
    }

    fn daily_usage(date: &str, cost: f64, models: Vec<(&str, &str, f64)>) -> DailyUsage {
        let mut model_breakdown = BTreeMap::new();
        let mut total_tokens = TokenBreakdown::default();
        let mut total_cost = 0.0;

        for (model, provider, model_cost) in models {
            let tokens = TokenBreakdown {
                input: (model_cost * 100.0) as u64,
                output: 10,
                cache_read: 5,
                cache_write: 0,
                reasoning: 0,
            };
            total_tokens.input = total_tokens.input.saturating_add(tokens.input);
            total_tokens.output = total_tokens.output.saturating_add(tokens.output);
            total_tokens.cache_read = total_tokens.cache_read.saturating_add(tokens.cache_read);
            total_cost += model_cost;

            model_breakdown.insert(
                model.to_string(),
                DailyModelInfo {
                    provider: provider.to_string(),
                    display_name: model.to_string(),
                    color_key: model.to_string(),
                    tokens,
                    cost: model_cost,
                    messages: 1,
                },
            );
        }

        let mut source_breakdown = BTreeMap::new();
        source_breakdown.insert(
            "claude".to_string(),
            DailySourceInfo {
                tokens: total_tokens.clone(),
                cost: total_cost,
                models: model_breakdown,
            },
        );

        DailyUsage {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            tokens: total_tokens,
            cost: if cost > 0.0 { cost } else { total_cost },
            source_breakdown,
            message_count: 1,
            turn_count: 1,
        }
    }

    fn minutely_usage(datetime: &str, input_tokens: u64, cost: f64) -> MinutelyUsage {
        MinutelyUsage {
            datetime: NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d %H:%M:%S").unwrap(),
            tokens: TokenBreakdown {
                input: input_tokens,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            clients: BTreeSet::new(),
            models: BTreeMap::new(),
            message_count: 1,
            turn_count: 1,
        }
    }

    #[test]
    fn test_get_sorted_minutely_reuses_cached_order_for_same_sort() {
        let mut app = make_app();
        app.data.minutely = vec![
            minutely_usage("2026-05-20 10:00:00", 10, 1.0),
            minutely_usage("2026-05-20 10:01:00", 20, 9.0),
        ];

        let first = app
            .get_sorted_minutely()
            .iter()
            .map(|entry| entry.datetime)
            .collect::<Vec<_>>();
        assert_eq!(
            first,
            vec![
                NaiveDateTime::parse_from_str("2026-05-20 10:01:00", "%Y-%m-%d %H:%M:%S").unwrap(),
                NaiveDateTime::parse_from_str("2026-05-20 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            ]
        );

        app.data.minutely.swap(0, 1);

        let second = app
            .get_sorted_minutely()
            .iter()
            .map(|entry| entry.datetime)
            .collect::<Vec<_>>();
        assert_eq!(
            second,
            vec![
                NaiveDateTime::parse_from_str("2026-05-20 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
                NaiveDateTime::parse_from_str("2026-05-20 10:01:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            ],
            "unchanged data should reuse the cached sorted index order"
        );
    }

    #[test]
    fn test_get_sorted_minutely_invalidates_cache_when_sort_changes() {
        let mut app = make_app();
        app.data.minutely = vec![
            minutely_usage("2026-05-20 10:00:00", 10, 1.0),
            minutely_usage("2026-05-20 10:01:00", 20, 9.0),
        ];
        let _ = app.get_sorted_minutely();

        app.data.minutely.swap(0, 1);
        app.set_sort(SortField::Date);

        let sorted = app
            .get_sorted_minutely()
            .iter()
            .map(|entry| entry.datetime)
            .collect::<Vec<_>>();
        assert_eq!(
            sorted,
            vec![
                NaiveDateTime::parse_from_str("2026-05-20 10:01:00", "%Y-%m-%d %H:%M:%S").unwrap(),
                NaiveDateTime::parse_from_str("2026-05-20 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            ],
            "changing sort key should rebuild the minutely sorted cache"
        );
    }

    #[test]
    fn test_get_sorted_minutely_invalidates_cache_when_data_updates() {
        let mut app = make_app();
        app.data.minutely = vec![
            minutely_usage("2026-05-20 10:00:00", 10, 1.0),
            minutely_usage("2026-05-20 10:01:00", 20, 9.0),
        ];
        let _ = app.get_sorted_minutely();

        let refreshed = UsageData {
            minutely: vec![
                minutely_usage("2026-05-20 10:02:00", 30, 2.0),
                minutely_usage("2026-05-20 10:03:00", 40, 12.0),
            ],
            ..Default::default()
        };
        app.update_data(refreshed);

        let sorted = app
            .get_sorted_minutely()
            .iter()
            .map(|entry| entry.datetime)
            .collect::<Vec<_>>();
        assert_eq!(
            sorted,
            vec![
                NaiveDateTime::parse_from_str("2026-05-20 10:03:00", "%Y-%m-%d %H:%M:%S").unwrap(),
                NaiveDateTime::parse_from_str("2026-05-20 10:02:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            ],
            "update_data should clear stale minutely sorted cache entries"
        );
    }

    fn monthly_usage(month: &str, input_tokens: u64, cost: f64) -> MonthlyUsage {
        MonthlyUsage {
            month: month.to_string(),
            tokens: TokenBreakdown {
                input: input_tokens,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            message_count: 1,
            turn_count: 1,
        }
    }

    #[test]
    fn test_get_sorted_monthly_defaults_to_date_descending() {
        let mut app = make_app();
        app.switch_tab(Tab::Monthly);
        app.data.monthly = vec![
            monthly_usage("2026-03", 100, 1.0),
            monthly_usage("2026-05", 200, 2.0),
            monthly_usage("2026-04", 300, 3.0),
        ];

        let sorted = app
            .get_sorted_monthly()
            .iter()
            .map(|m| m.month.as_str())
            .collect::<Vec<_>>();
        assert_eq!(sorted, vec!["2026-05", "2026-04", "2026-03"]);
    }

    #[test]
    fn test_get_sorted_monthly_by_cost() {
        let mut app = make_app();
        app.current_tab = Tab::Monthly;
        app.sort_field = SortField::Cost;
        app.sort_direction = SortDirection::Ascending;
        app.data.monthly = vec![
            monthly_usage("2026-05", 100, 3.0),
            monthly_usage("2026-04", 100, 1.0),
            monthly_usage("2026-06", 100, 2.0),
        ];

        let sorted = app
            .get_sorted_monthly()
            .iter()
            .map(|m| m.month.as_str())
            .collect::<Vec<_>>();
        assert_eq!(sorted, vec!["2026-04", "2026-06", "2026-05"]);
    }

    #[test]
    fn test_get_sorted_monthly_detail_days_filters_by_month() {
        let mut app = make_app();
        app.current_tab = Tab::Monthly;
        app.selected_monthly_detail_month = Some("2026-05".to_string());
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("model-a", "openai", 1.0)]),
            daily_usage("2026-05-20", 2.0, vec![("model-b", "anthropic", 2.0)]),
            daily_usage("2026-06-01", 3.0, vec![("model-c", "google", 3.0)]),
        ];

        let days = app.get_sorted_monthly_detail_days();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date.to_string(), "2026-05-20");
        assert_eq!(days[1].date.to_string(), "2026-05-10");
    }

    #[test]
    fn test_open_monthly_detail_shows_daily_breakdown() {
        let mut app = make_app();
        app.switch_tab(Tab::Monthly);
        app.data.monthly = vec![
            monthly_usage("2026-05", 100, 1.0),
            monthly_usage("2026-04", 200, 2.0),
        ];
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("model-a", "openai", 1.0)]),
            daily_usage("2026-04-05", 2.0, vec![("model-b", "anthropic", 2.0)]),
        ];

        app.handle_key_event(key(KeyCode::Enter));

        assert!(app.is_monthly_detail_active());
        assert_eq!(app.monthly_detail_month(), Some("2026-05"));
        assert_eq!(app.get_sorted_monthly_detail_days().len(), 1);
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_esc_closes_monthly_detail_and_restores_selection() {
        let mut app = make_app();
        app.switch_tab(Tab::Monthly);
        app.data.monthly = vec![
            monthly_usage("2026-05", 100, 1.0),
            monthly_usage("2026-04", 200, 2.0),
        ];
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("model-a", "openai", 1.0)]),
            daily_usage("2026-04-05", 2.0, vec![("model-b", "anthropic", 2.0)]),
        ];

        app.handle_key_event(key(KeyCode::Enter));
        assert!(app.is_monthly_detail_active());

        app.handle_key_event(key(KeyCode::Esc));

        assert!(!app.is_monthly_detail_active());
        assert_eq!(app.monthly_detail_month(), None);
        assert_eq!(app.current_tab, Tab::Monthly);
    }

    #[test]
    fn test_close_monthly_detail_restores_saved_viewport() {
        let mut app = make_app();
        app.switch_tab(Tab::Monthly);
        app.data.monthly = (1..=10)
            .map(|month| monthly_usage(&format!("2026-{month:02}"), 100, 1.0))
            .collect();
        app.data.daily = vec![daily_usage(
            "2026-03-10",
            1.0,
            vec![("model-a", "openai", 1.0)],
        )];
        app.max_visible_items = 4;
        app.selected_index = 7;
        app.scroll_offset = 6;

        app.open_selected_monthly_detail();
        assert_eq!(app.monthly_detail_month(), Some("2026-03"));
        assert_eq!(app.scroll_offset, 0);

        app.close_monthly_detail();

        assert_eq!(app.selected_index, 7);
        assert_eq!(app.scroll_offset, 6);
    }

    #[test]
    fn test_switch_tab_clears_monthly_detail() {
        let mut app = make_app();
        app.switch_tab(Tab::Monthly);
        app.data.monthly = vec![monthly_usage("2026-05", 100, 1.0)];
        app.data.daily = vec![daily_usage(
            "2026-05-10",
            1.0,
            vec![("model-a", "openai", 1.0)],
        )];

        app.open_selected_monthly_detail();
        assert!(app.is_monthly_detail_active());

        app.switch_tab(Tab::Daily);

        assert!(!app.is_monthly_detail_active());
    }

    #[test]
    fn test_update_data_exits_monthly_detail_when_month_disappears() {
        let mut app = make_app();
        app.switch_tab(Tab::Monthly);
        app.data.monthly = vec![monthly_usage("2026-05", 100, 1.0)];
        app.data.daily = vec![daily_usage(
            "2026-05-10",
            1.0,
            vec![("model-a", "openai", 1.0)],
        )];

        app.open_selected_monthly_detail();
        assert!(app.is_monthly_detail_active());

        app.update_data(UsageData {
            monthly: vec![monthly_usage("2026-04", 200, 2.0)],
            daily: vec![daily_usage(
                "2026-04-05",
                2.0,
                vec![("model-b", "anthropic", 2.0)],
            )],
            ..Default::default()
        });

        assert!(!app.is_monthly_detail_active());
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_with_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    // ── handle_key_event: quit ──────────────────────────────────────

    #[test]
    fn test_handle_key_quit_q() {
        let mut app = make_app();
        let quit = app.handle_key_event(key(KeyCode::Char('q')));
        assert!(quit);
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_key_quit_ctrl_c() {
        let mut app = make_app();
        let quit = app.handle_key_event(key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(quit);
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_key_quit_q_russian_layout() {
        // Physical `Q` on a Russian layout produces 'й'; it must still quit.
        let mut app = make_app();
        let quit = app.handle_key_event(key(KeyCode::Char('й')));
        assert!(quit);
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_key_quit_ctrl_c_russian_layout() {
        // Physical `C` on a Russian layout produces 'с'; Ctrl+С must still quit.
        let mut app = make_app();
        let quit = app.handle_key_event(key_with_mod(KeyCode::Char('с'), KeyModifiers::CONTROL));
        assert!(quit);
        assert!(app.should_quit);
    }

    // ── handle_key_event: tab switching ─────────────────────────────

    #[test]
    fn test_handle_key_tab_switch() {
        let mut app = make_app();
        assert_eq!(app.current_tab, Tab::Overview);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Usage);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Models);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Daily);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Hourly);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Monthly);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Stats);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Agents);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Overview);
    }

    #[test]
    fn test_handle_key_backtab_switch() {
        let mut app = make_app();
        assert_eq!(app.current_tab, Tab::Overview);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Agents);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Stats);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Monthly);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Hourly);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Daily);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Models);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Usage);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Overview);
    }

    #[test]
    fn test_handle_key_tab_switch_with_minutely_enabled_includes_minutely() {
        let mut app = make_app();
        app.settings.minutely_tab_enabled = true;
        assert_eq!(app.current_tab, Tab::Overview);

        for expected in [
            Tab::Usage,
            Tab::Models,
            Tab::Daily,
            Tab::Hourly,
            Tab::Minutely,
            Tab::Monthly,
            Tab::Stats,
            Tab::Agents,
            Tab::Overview,
        ] {
            app.handle_key_event(key(KeyCode::Tab));
            assert_eq!(app.current_tab, expected);
        }
    }

    #[test]
    fn test_initial_minutely_tab_clamps_to_overview_when_flag_off() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: Some(Tab::Minutely),
        };
        let app = App::new_with_cached_data(config, Some(UsageData::default())).unwrap();
        assert_eq!(app.current_tab, Tab::Overview);
    }

    #[test]
    fn test_get_sorted_agents_by_cost_desc() {
        let mut app = make_app();
        app.data.agents = vec![
            AgentUsage {
                agent: "builder".to_string(),
                clients: "opencode".to_string(),
                tokens: TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                cost: 3.0,
                message_count: 1,
            },
            AgentUsage {
                agent: "reviewer".to_string(),
                clients: "roocode".to_string(),
                tokens: TokenBreakdown {
                    input: 50,
                    output: 20,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                cost: 7.0,
                message_count: 2,
            },
        ];

        let agents = app.get_sorted_agents();
        assert_eq!(agents[0].agent, "reviewer");
        assert_eq!(agents[1].agent, "builder");
    }

    #[test]
    fn test_get_sorted_agents_by_tokens_asc() {
        let mut app = make_app();
        app.sort_field = SortField::Tokens;
        app.sort_direction = SortDirection::Ascending;
        app.data.agents = vec![
            AgentUsage {
                agent: "builder".to_string(),
                clients: "opencode".to_string(),
                tokens: TokenBreakdown {
                    input: 100,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                cost: 1.0,
                message_count: 1,
            },
            AgentUsage {
                agent: "reviewer".to_string(),
                clients: "roocode".to_string(),
                tokens: TokenBreakdown {
                    input: 20,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                cost: 5.0,
                message_count: 1,
            },
        ];

        let agents = app.get_sorted_agents();
        assert_eq!(agents[0].agent, "reviewer");
        assert_eq!(agents[1].agent, "builder");
    }

    #[test]
    fn test_handle_key_left_right_switch() {
        let mut app = make_app();
        app.handle_key_event(key(KeyCode::Right));
        assert_eq!(app.current_tab, Tab::Usage);

        app.handle_key_event(key(KeyCode::Right));
        assert_eq!(app.current_tab, Tab::Models);

        app.handle_key_event(key(KeyCode::Left));
        assert_eq!(app.current_tab, Tab::Usage);
    }

    #[test]
    fn test_handle_key_tab_resets_selection() {
        let mut app = make_app_with_models(5);
        app.selected_index = 3;
        app.scroll_offset = 1;
        app.selected_graph_cell = Some((2, 4));

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.selected_index, 0);
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.selected_graph_cell, None);
    }

    #[test]
    fn test_enter_on_daily_opens_selected_day_detail_rows() {
        let mut app = make_app();
        app.current_tab = Tab::Daily;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("old-model", "anthropic", 1.0)]),
            daily_usage(
                "2026-05-17",
                7.0,
                vec![("target-a", "openai", 5.0), ("target-b", "anthropic", 2.0)],
            ),
            daily_usage("2026-05-18", 3.0, vec![("other-model", "google", 3.0)]),
        ];

        app.selected_index = 0;
        app.handle_key_event(key(KeyCode::Down));
        app.handle_key_event(key(KeyCode::Enter));

        assert_eq!(app.get_current_list_len(), 2);
    }

    #[test]
    fn test_esc_from_daily_detail_restores_daily_selection() {
        let mut app = make_app();
        app.current_tab = Tab::Daily;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("old-model", "anthropic", 1.0)]),
            daily_usage(
                "2026-05-17",
                7.0,
                vec![("target-a", "openai", 5.0), ("target-b", "anthropic", 2.0)],
            ),
            daily_usage("2026-05-18", 3.0, vec![("other-model", "google", 3.0)]),
        ];

        app.max_visible_items = 2;
        app.selected_index = 1;
        app.scroll_offset = 1;
        app.handle_key_event(key(KeyCode::Enter));
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.selected_index, 1);

        app.handle_key_event(key(KeyCode::Esc));

        assert_eq!(app.current_tab, Tab::Daily);
        assert_eq!(app.selected_index, 1);
        assert_eq!(app.scroll_offset, 1);
        assert_eq!(app.get_current_list_len(), 3);
    }

    #[test]
    fn test_close_daily_detail_reanchors_selection_by_date_after_sort_change() {
        let mut app = make_app();
        app.current_tab = Tab::Daily;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("old-model", "anthropic", 1.0)]),
            daily_usage(
                "2026-05-17",
                7.0,
                vec![("target-a", "openai", 5.0), ("target-b", "anthropic", 2.0)],
            ),
            daily_usage("2026-05-18", 3.0, vec![("other-model", "google", 3.0)]),
        ];

        app.selected_index = 1;
        let target_date = app.get_sorted_daily()[app.selected_index].date;

        app.handle_key_event(key(KeyCode::Enter));
        assert!(app.is_daily_detail_active());
        assert_eq!(app.daily_detail_date(), Some(target_date));

        app.handle_key_event(key(KeyCode::Char('c')));
        assert_eq!(app.sort_field, SortField::Cost);

        app.handle_key_event(key(KeyCode::Esc));

        assert!(!app.is_daily_detail_active());
        let restored_index = app.selected_index;
        let restored_date = app.get_sorted_daily()[restored_index].date;
        assert_eq!(
            restored_date, target_date,
            "Closing detail after sort change should re-anchor on the original date"
        );
    }

    #[test]
    fn test_update_data_exits_daily_detail_when_date_disappears() {
        let mut app = make_app();
        app.current_tab = Tab::Daily;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("old-model", "anthropic", 1.0)]),
            daily_usage(
                "2026-05-17",
                7.0,
                vec![("target-a", "openai", 5.0), ("target-b", "anthropic", 2.0)],
            ),
            daily_usage("2026-05-18", 3.0, vec![("other-model", "google", 3.0)]),
        ];

        app.selected_index = 1;
        app.handle_key_event(key(KeyCode::Enter));
        assert!(app.is_daily_detail_active());

        let refreshed = UsageData {
            daily: vec![
                daily_usage("2026-05-10", 1.0, vec![("old-model", "anthropic", 1.0)]),
                daily_usage("2026-05-18", 3.0, vec![("other-model", "google", 3.0)]),
            ],
            ..Default::default()
        };
        app.update_data(refreshed);

        assert!(
            !app.is_daily_detail_active(),
            "update_data should drop detail mode when the selected date is gone"
        );
        assert_eq!(app.daily_detail_date(), None);
        assert!(app.get_sorted_daily_detail_rows().is_empty());
    }

    #[test]
    fn test_update_data_keeps_daily_detail_when_date_still_present() {
        let mut app = make_app();
        app.current_tab = Tab::Daily;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app.data.daily = vec![
            daily_usage("2026-05-10", 1.0, vec![("old-model", "anthropic", 1.0)]),
            daily_usage(
                "2026-05-17",
                7.0,
                vec![("target-a", "openai", 5.0), ("target-b", "anthropic", 2.0)],
            ),
        ];

        app.selected_index = 1;
        let target_date = app.get_sorted_daily()[app.selected_index].date;
        app.handle_key_event(key(KeyCode::Enter));
        assert!(app.is_daily_detail_active());

        let refreshed = UsageData {
            daily: vec![
                daily_usage("2026-05-10", 1.0, vec![("old-model", "anthropic", 1.0)]),
                daily_usage(
                    "2026-05-17",
                    9.0,
                    vec![("target-a", "openai", 7.0), ("target-b", "anthropic", 2.0)],
                ),
            ],
            ..Default::default()
        };
        app.update_data(refreshed);

        assert!(app.is_daily_detail_active());
        assert_eq!(app.daily_detail_date(), Some(target_date));
    }

    // ── handle_key_event: sort ──────────────────────────────────────

    #[test]
    fn test_handle_key_sort_cost() {
        let mut app = make_app();
        app.handle_key_event(key(KeyCode::Char('c')));
        assert_eq!(app.sort_field, SortField::Cost);
        assert_eq!(app.sort_direction, SortDirection::Ascending);
    }

    #[test]
    fn test_handle_key_sort_tokens() {
        let mut app = make_app();
        app.handle_key_event(key(KeyCode::Char('t')));
        assert_eq!(app.sort_field, SortField::Tokens);
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    #[test]
    fn test_handle_key_sort_date() {
        let mut app = make_app();
        app.handle_key_event(key(KeyCode::Char('d')));
        assert_eq!(app.sort_field, SortField::Date);
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    #[test]
    fn test_handle_key_sort_toggle_direction() {
        let mut app = make_app();
        app.handle_key_event(key(KeyCode::Char('t')));
        assert_eq!(app.sort_direction, SortDirection::Descending);

        app.handle_key_event(key(KeyCode::Char('t')));
        assert_eq!(app.sort_direction, SortDirection::Ascending);

        app.handle_key_event(key(KeyCode::Char('t')));
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    #[test]
    fn test_switch_tab_restores_hourly_date_default() {
        let mut app = make_app();
        assert_eq!(app.sort_field, SortField::Cost);

        app.switch_tab(Tab::Hourly);
        assert_eq!(app.sort_field, SortField::Date);
        assert_eq!(app.sort_direction, SortDirection::Descending);

        app.switch_tab(Tab::Models);
        assert_eq!(app.sort_field, SortField::Cost);
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    #[test]
    fn test_initial_hourly_tab_uses_hourly_sort_default() {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: Some(Tab::Hourly),
        };

        let app = App::new_with_cached_data(config, None).unwrap();

        assert_eq!(app.current_tab, Tab::Hourly);
        assert_eq!(app.sort_field, SortField::Date);
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    #[test]
    fn test_switch_tab_preserves_user_sort() {
        let mut app = make_app();
        app.switch_tab(Tab::Models);

        app.set_sort(SortField::Tokens);
        assert_eq!(app.sort_field, SortField::Tokens);
        assert_eq!(app.sort_direction, SortDirection::Descending);

        app.switch_tab(Tab::Daily);
        assert_eq!(app.sort_field, SortField::Cost);

        app.switch_tab(Tab::Models);
        assert_eq!(app.sort_field, SortField::Tokens);
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    #[test]
    fn test_switch_tab_preserves_daily_sort_after_hourly_roundtrip() {
        let mut app = make_app();

        app.switch_tab(Tab::Daily);
        app.set_sort(SortField::Date);
        assert_eq!(app.sort_field, SortField::Date);
        assert_eq!(app.sort_direction, SortDirection::Descending);

        app.switch_tab(Tab::Hourly);
        assert_eq!(app.sort_field, SortField::Date);
        assert_eq!(app.sort_direction, SortDirection::Descending);

        app.switch_tab(Tab::Daily);
        assert_eq!(app.sort_field, SortField::Date);
        assert_eq!(app.sort_direction, SortDirection::Descending);
    }

    // ── handle_key_event: navigation ────────────────────────────────

    #[test]
    fn test_handle_key_navigation_up_down() {
        let mut app = make_app_with_models(5);
        assert_eq!(app.selected_index, 0);

        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.selected_index, 1);

        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.selected_index, 2);

        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.selected_index, 1);

        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.selected_index, 0);

        // At top boundary - wraps to last item (index 4, 5 models)
        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.selected_index, 4);
    }

    #[test]
    fn test_handle_key_navigation_boundary() {
        let mut app = make_app_with_models(3);
        app.handle_key_event(key(KeyCode::Down));
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.selected_index, 2);

        // At bottom boundary - wraps to first item (index 0)
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.selected_index, 0);
    }

    // ── wrap-around navigation ──────────────────────────────────────

    #[test]
    fn test_move_selection_up_wraps_to_last() {
        let mut app = make_app_with_models(3);
        app.max_visible_items = 10;
        app.selected_index = 0;
        app.move_selection_up();
        assert_eq!(app.selected_index, 2);
    }

    #[test]
    fn test_move_selection_down_wraps_to_first() {
        let mut app = make_app_with_models(3);
        app.max_visible_items = 10;
        app.selected_index = 2;
        app.move_selection_down();
        assert_eq!(app.selected_index, 0);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_move_selection_up_empty_list_noop() {
        let mut app = make_app();
        app.data.models.clear();
        app.selected_index = 0;
        app.move_selection_up();
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_move_selection_down_empty_list_noop() {
        let mut app = make_app();
        app.data.models.clear();
        app.selected_index = 0;
        app.move_selection_down();
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_move_selection_up_wrap_scroll_offset() {
        let mut app = make_app_with_models(10);
        app.max_visible_items = 3;
        app.selected_index = 0;
        app.move_selection_up();
        // Should wrap to index 9 and scroll so last item is visible
        assert_eq!(app.selected_index, 9);
        assert_eq!(app.scroll_offset, 7); // 10 - 3 = 7
    }

    #[test]
    fn test_move_selection_down_wrap_resets_scroll() {
        let mut app = make_app_with_models(10);
        app.max_visible_items = 3;
        app.selected_index = 9;
        app.scroll_offset = 7;
        app.move_selection_down();
        assert_eq!(app.selected_index, 0);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_overview_scroll_keeps_rendered_capacity_after_resize() {
        let mut app = make_app_with_models(33);
        app.current_tab = Tab::Overview;
        app.set_max_visible_items(9);

        for _ in 0..32 {
            app.move_selection_down();
            app.handle_resize(120, 40);
            app.set_max_visible_items(9);
        }

        assert_eq!(app.selected_index, 32);
        assert_eq!(app.scroll_offset, 24);
    }

    // ── handle_key_event: theme ─────────────────────────────────────

    #[test]
    fn test_handle_key_theme_cycle() {
        let mut app = make_app();
        let initial_theme = app.theme.name;

        app.handle_key_event(key(KeyCode::Char('p')));
        assert_ne!(app.theme.name, initial_theme);

        for _ in 1..ThemeName::all().len() {
            app.handle_key_event(key(KeyCode::Char('p')));
        }
        assert_eq!(app.theme.name, initial_theme);
    }

    // ── handle_key_event: export ────────────────────────────────────

    #[test]
    fn test_handle_key_export() {
        let mut app = make_app();
        app.handle_key_event(key(KeyCode::Char('e')));
        assert!(app.status_message.is_some());
        let msg = app.status_message.as_ref().unwrap();
        assert!(
            msg.contains("Exported to") || msg.contains("Export failed"),
            "unexpected status: {}",
            msg
        );
    }

    // ── handle_key_event: refresh ───────────────────────────────────

    #[test]
    #[ignore] // triggers load_data() which requires network + filesystem I/O
    fn test_handle_key_refresh() {
        let mut app = make_app();
        std::thread::sleep(Duration::from_millis(5));
        app.handle_key_event(key(KeyCode::Char('r')));
        assert!(app.needs_reload);
    }

    #[test]
    fn test_handle_key_refresh_while_loading_does_not_queue_reload() {
        let mut app = make_app();
        app.background_loading = true;

        app.handle_key_event(key(KeyCode::Char('r')));

        assert!(!app.needs_reload);
        assert!(!app.is_fetching_usage());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Refresh already in progress")
        );
    }

    #[test]
    fn test_handle_key_refresh_usage_tab_fetches_usage() {
        let mut app = make_app();
        app.usage_fetcher = sample_usage_fetcher;
        app.current_tab = Tab::Usage;

        app.handle_key_event(key(KeyCode::Char('r')));

        assert!(!app.needs_reload);
        assert!(app.is_fetching_usage());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Fetching usage data...")
        );

        drain_usage_fetch(&mut app);

        assert_eq!(app.subscription_usage.len(), 1);
        assert_eq!(app.subscription_usage[0].provider, "Codex");
        assert_eq!(app.status_message.as_deref(), Some("Usage data loaded"));
    }

    #[test]
    fn test_handle_key_refresh_usage_tab_reports_fetch_failure_diagnostic() {
        let mut app = make_app();
        app.usage_fetcher = failing_usage_fetcher;
        app.current_tab = Tab::Usage;

        app.handle_key_event(key(KeyCode::Char('r')));
        drain_usage_fetch(&mut app);

        assert!(app.subscription_usage.is_empty());
        assert_eq!(app.usage_fetch_diagnostics.len(), 1);
        assert_eq!(
            app.status_message.as_deref(),
            Some("Usage fetch failed: Codex")
        );
    }

    #[test]
    fn test_handle_key_refresh_usage_tab_keeps_partial_fetch_diagnostic() {
        let mut app = make_app();
        app.usage_fetcher = partial_usage_fetcher;
        app.current_tab = Tab::Usage;

        app.handle_key_event(key(KeyCode::Char('r')));
        drain_usage_fetch(&mut app);

        assert_eq!(app.subscription_usage.len(), 1);
        assert_eq!(app.usage_fetch_diagnostics.len(), 1);
        assert_eq!(
            app.status_message.as_deref(),
            Some("Usage data loaded with 1 issue")
        );
    }

    #[test]
    fn test_handle_key_refresh_usage_tab_clears_stale_diagnostics() {
        let mut app = make_app();
        app.current_tab = Tab::Usage;
        app.usage_fetch_diagnostics = vec![UsageFetchDiagnostic::new("Codex", None, "stale issue")];

        app.handle_key_event(key(KeyCode::Char('r')));

        assert!(app.is_fetching_usage());
        assert!(app.usage_fetch_diagnostics.is_empty());
    }

    #[test]
    fn test_handle_key_u_on_usage_is_unassigned() {
        let mut app = make_app();
        app.current_tab = Tab::Usage;

        app.handle_key_event(key(KeyCode::Char('u')));

        assert!(!app.needs_reload);
        assert!(!app.is_fetching_usage());
        assert!(!app.usage_fetch_attempted);
    }

    #[test]
    fn test_auto_refresh_on_usage_refreshes_usage_only() {
        let mut app = make_app();
        app.current_tab = Tab::Usage;
        app.auto_refresh = true;
        app.auto_refresh_interval = Duration::from_millis(1);
        app.last_auto_refresh = Instant::now() - Duration::from_secs(1);

        app.on_tick();

        assert!(!app.needs_reload);
        assert!(app.usage_fetch_attempted);
    }

    #[test]
    fn test_auto_refresh_on_usage_while_fetching_preserves_status() {
        let mut app = make_app();
        app.current_tab = Tab::Usage;
        app.auto_refresh = true;
        app.auto_refresh_interval = Duration::from_millis(1);
        app.last_auto_refresh = Instant::now() - Duration::from_secs(1);
        let (_tx, rx) = std::sync::mpsc::channel();
        app.usage_rx = Some(rx);
        app.status_message = Some("Existing status".into());

        app.on_tick();

        assert_eq!(app.status_message.as_deref(), Some("Existing status"));
        assert!(!app.needs_reload);
    }

    #[test]
    fn test_auto_refresh_on_usage_when_idle_preserves_status() {
        // The while-fetching case above hits the early return in
        // fetch_subscription_usage_with_status. This covers the idle case
        // (no fetch in flight), where a non-preserving fetch would overwrite
        // the status with "Fetching usage data...". Auto-refresh must keep the
        // existing message and start a silent background fetch.
        let mut app = make_app();
        app.current_tab = Tab::Usage;
        app.auto_refresh = true;
        app.auto_refresh_interval = Duration::from_millis(1);
        app.last_auto_refresh = Instant::now() - Duration::from_secs(1);
        app.status_message = Some("Existing status".into());
        assert!(app.usage_rx.is_none());

        app.on_tick();

        assert_eq!(app.status_message.as_deref(), Some("Existing status"));
        assert!(app.usage_fetch_attempted);
        assert!(!app.needs_reload);
    }

    #[test]
    fn test_auto_refresh_on_overview_refreshes_token_data_only() {
        let mut app = make_app();
        app.current_tab = Tab::Overview;
        app.auto_refresh = true;
        app.auto_refresh_interval = Duration::from_millis(1);
        app.last_auto_refresh = Instant::now() - Duration::from_secs(1);

        app.on_tick();

        assert!(app.needs_reload);
        assert!(!app.usage_fetch_attempted);
        assert!(!app.is_fetching_usage());
    }

    #[test]
    fn test_codex_reset_success_status_survives_follow_up_usage_refresh() {
        let mut app = make_app();
        let (tx, rx) = std::sync::mpsc::channel();
        app.codex_reset_rx = Some(rx);
        tx.send(Ok(
            crate::commands::usage::codex::RateLimitResetConsumeResult {
                code: "reset".to_string(),
                windows_reset: Some(1),
            },
        ))
        .unwrap();
        drop(tx);

        app.on_tick();

        assert_eq!(
            app.status_message.as_deref(),
            Some("Codex reset credit: reset 1 window")
        );
        assert!(app.is_fetching_usage());

        for _ in 0..20 {
            app.on_tick();
            if !app.is_fetching_usage() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(!app.is_fetching_usage());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Codex reset credit: reset 1 window")
        );
    }

    // ── handle_key_event: misc keys ─────────────────────────────────

    #[test]
    fn test_handle_key_esc_clears_graph_selection() {
        let mut app = make_app();
        app.selected_graph_cell = Some((1, 2));

        app.handle_key_event(key(KeyCode::Esc));
        assert_eq!(app.selected_graph_cell, None);
    }

    #[test]
    fn test_handle_key_enter_on_stats() {
        let mut app = make_app();
        app.current_tab = Tab::Stats;
        app.selected_graph_cell = Some((1, 2));

        app.handle_key_event(key(KeyCode::Enter));
        assert!(app.status_message.is_some());
    }

    #[test]
    fn test_handle_key_unrecognized_returns_false() {
        let mut app = make_app();
        let result = app.handle_key_event(key(KeyCode::F(12)));
        assert!(!result);
        assert!(!app.should_quit);
    }

    #[test]
    fn test_handle_key_auto_refresh_toggle() {
        let mut app = make_app();
        let initial = app.auto_refresh;
        app.handle_key_event(key_with_mod(KeyCode::Char('R'), KeyModifiers::SHIFT));
        assert_ne!(app.auto_refresh, initial);
    }

    #[test]
    fn test_enabling_auto_refresh_waits_for_next_interval() {
        let mut app = make_app();
        app.auto_refresh = false;
        app.auto_refresh_interval = Duration::from_secs(60);
        app.last_auto_refresh = Instant::now() - Duration::from_secs(120);

        app.handle_key_event(key_with_mod(KeyCode::Char('R'), KeyModifiers::SHIFT));
        app.on_tick();

        assert!(app.auto_refresh);
        assert!(!app.needs_reload);
        assert!(!app.usage_fetch_attempted);
    }

    #[test]
    fn test_handle_key_increase_decrease_refresh() {
        let mut app = make_app();
        let initial_interval = app.auto_refresh_interval;

        app.handle_key_event(key(KeyCode::Char('+')));
        assert!(app.auto_refresh_interval > initial_interval);

        let after_increase = app.auto_refresh_interval;
        app.handle_key_event(key(KeyCode::Char('-')));
        assert!(app.auto_refresh_interval < after_increase);
    }

    // ── handle_mouse_event ──────────────────────────────────────────

    #[test]
    fn test_handle_mouse_left_click() {
        let mut app = make_app();
        app.add_click_area(Rect::new(0, 0, 10, 2), ClickAction::Tab(Tab::Models));

        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);
        assert_eq!(app.current_tab, Tab::Models);
    }

    #[test]
    fn test_handle_mouse_click_sort() {
        let mut app = make_app();
        app.add_click_area(Rect::new(0, 0, 10, 2), ClickAction::Sort(SortField::Tokens));

        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);
        assert_eq!(app.sort_field, SortField::Tokens);
    }

    #[test]
    fn test_handle_mouse_click_graph_cell() {
        let mut app = make_app();
        app.add_click_area(
            Rect::new(10, 5, 3, 3),
            ClickAction::GraphCell { week: 2, day: 3 },
        );

        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 11,
            row: 6,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);
        assert_eq!(app.selected_graph_cell, Some((2, 3)));
    }

    #[test]
    fn test_handle_mouse_click_usage_refresh_uses_refresh_path() {
        let mut app = make_app();
        app.background_loading = true;
        app.add_click_area(Rect::new(0, 0, 10, 2), ClickAction::UsageRefresh);

        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);

        assert!(!app.needs_reload);
        assert!(app.is_fetching_usage());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Fetching usage data...")
        );
    }

    #[test]
    fn test_handle_mouse_click_codex_remove_opens_confirmation_dialog() {
        let mut app = make_app();
        app.add_click_area(
            Rect::new(0, 0, 10, 2),
            ClickAction::CodexRemoveAccount {
                account_id: "acct_work".to_string(),
            },
        );

        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);

        assert!(app.dialog_stack.is_active());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Confirm Codex account removal")
        );
    }

    #[test]
    fn test_handle_mouse_click_codex_remove_refuses_active_account() {
        let mut app = make_app();
        app.subscription_usage = sample_subscription_usage();
        app.add_click_area(
            Rect::new(0, 0, 10, 2),
            ClickAction::CodexRemoveAccount {
                account_id: "acct_work".to_string(),
            },
        );

        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);

        assert!(!app.dialog_stack.is_active());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Switch Codex accounts before removing the current account")
        );
    }

    #[test]
    fn test_handle_mouse_click_outside_areas() {
        let mut app = make_app();
        app.add_click_area(Rect::new(0, 0, 5, 5), ClickAction::Tab(Tab::Stats));

        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 50,
            row: 50,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);
        assert_eq!(app.current_tab, Tab::Overview);
    }

    #[test]
    fn test_handle_mouse_scroll_up() {
        let mut app = make_app_with_models(5);
        app.selected_index = 2;

        let event = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);
        assert_eq!(app.selected_index, 1);
    }

    #[test]
    fn test_handle_mouse_scroll_down() {
        let mut app = make_app_with_models(5);
        app.selected_index = 2;

        let event = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(event);
        assert_eq!(app.selected_index, 3);
    }

    // ── handle_resize ───────────────────────────────────────────────

    #[test]
    fn test_handle_resize() {
        let mut app = make_app();
        assert_eq!(app.terminal_width, 80);
        assert_eq!(app.terminal_height, 24);

        app.handle_resize(120, 40);
        assert_eq!(app.terminal_width, 120);
        assert_eq!(app.terminal_height, 40);
        assert_eq!(app.max_visible_items, 20);
    }

    #[test]
    fn test_handle_resize_small_terminal() {
        let mut app = make_app();
        app.handle_resize(40, 12);
        assert_eq!(app.terminal_width, 40);
        assert_eq!(app.terminal_height, 12);
        assert_eq!(app.max_visible_items, 20);
    }

    #[test]
    fn test_handle_resize_preserves_rendered_capacity() {
        let mut app = make_app_with_models(5);
        app.selected_index = 4;
        app.scroll_offset = 2;
        app.max_visible_items = 3;

        app.handle_resize(80, 24);

        assert_eq!(app.max_visible_items, 3);
        assert_eq!(app.selected_index, 4);
        assert_eq!(app.scroll_offset, 2);
    }

    #[test]
    fn test_set_max_visible_items_clamps_scroll_offset() {
        let mut app = make_app_with_models(10);
        app.selected_index = 9;
        app.scroll_offset = 9;

        app.set_max_visible_items(3);

        assert_eq!(app.max_visible_items, 3);
        assert_eq!(app.selected_index, 9);
        assert_eq!(app.scroll_offset, 7);
    }

    // ── on_tick ─────────────────────────────────────────────────────

    #[test]
    fn test_on_tick_increments_frame() {
        let mut app = make_app();
        assert_eq!(app.spinner_frame, 0);

        app.on_tick();
        assert_eq!(app.spinner_frame, 1);

        app.on_tick();
        assert_eq!(app.spinner_frame, 2);
    }

    #[test]
    fn test_on_tick_wraps_spinner_frame() {
        let mut app = make_app();
        app.spinner_frame = 19;
        app.on_tick();
        assert_eq!(app.spinner_frame, 0);
    }

    #[test]
    fn test_on_tick_clears_expired_status() {
        let mut app = make_app();
        app.set_status("test message");
        assert!(app.status_message.is_some());

        app.status_message_time = Some(Instant::now() - Duration::from_secs(5));
        app.auto_refresh = false;

        app.on_tick();
        assert!(app.status_message.is_none());
        assert!(app.status_message_time.is_none());
    }

    #[test]
    fn test_on_tick_keeps_fresh_status() {
        let mut app = make_app();
        app.auto_refresh = false;
        app.set_status("fresh message");

        app.on_tick();
        assert!(app.status_message.is_some());
        assert_eq!(app.status_message.as_ref().unwrap(), "fresh message");
    }

    #[test]
    fn test_on_tick_collects_codex_login_events() {
        let mut app = make_app();
        let (tx, rx) = std::sync::mpsc::channel();
        app.codex_login_rx = Some(rx);
        tx.send(CodexLoginEvent::Output(
            "Open https://example.com/device".to_string(),
        ))
        .unwrap();
        tx.send(CodexLoginEvent::Finished(CodexLoginOutcome::Failed(
            "expired".to_string(),
        )))
        .unwrap();
        drop(tx);

        app.on_tick();

        assert!(app.codex_login_rx.is_none());
        assert_eq!(
            app.codex_login_lines.last().map(String::as_str),
            Some("Open https://example.com/device")
        );
        assert!(matches!(
            app.codex_login_outcome,
            Some(CodexLoginOutcome::Failed(ref error)) if error == "expired"
        ));
        assert_eq!(
            app.status_message.as_deref(),
            Some("Codex login failed: expired")
        );
    }

    #[test]
    fn test_on_tick_clears_codex_login_panel_after_import() {
        let mut app = make_app();
        app.background_loading = true;
        let (tx, rx) = std::sync::mpsc::channel();
        app.codex_login_rx = Some(rx);
        tx.send(CodexLoginEvent::Output(
            "Starting Codex browser login".to_string(),
        ))
        .unwrap();
        tx.send(CodexLoginEvent::Finished(CodexLoginOutcome::Imported(
            crate::commands::usage::codex::CodexAccountInfo {
                id: "acct_work".to_string(),
                label: Some("work".to_string()),
                account_id: Some("acct_work".to_string()),
                created_at: "2026-06-09T00:00:00Z".to_string(),
                is_active: true,
            },
        )))
        .unwrap();
        drop(tx);

        app.on_tick();

        assert!(app.codex_login_rx.is_none());
        assert!(app.codex_login_lines.is_empty());
        assert!(app.codex_login_outcome.is_none());
        assert!(!app.should_show_codex_login_panel());
    }

    #[cfg(unix)]
    #[test]
    fn test_dismiss_codex_login_while_running_kills_child() {
        let mut app = make_app();
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let (_tx, rx) = std::sync::mpsc::channel();
        app.codex_login_rx = Some(rx);
        app.codex_login_lines.push("waiting".to_string());
        let slot = CodexLoginChildSlot::default();
        crate::tui::codex_login::put_codex_login_child_for_test(&slot, child).unwrap();
        app.codex_login_child = Some(std::sync::Arc::clone(&slot));

        app.dismiss_codex_login();

        assert!(app.codex_login_rx.is_none());
        assert!(app.codex_login_child.is_none());
        assert!(app.codex_login_lines.is_empty());
        assert!(app.codex_login_outcome.is_none());
        assert!(crate::tui::codex_login::codex_login_slot_child_is_none_for_test(&slot));
        assert_eq!(app.status_message.as_deref(), Some("Codex login cancelled"));
    }

    #[test]
    fn test_dismiss_codex_login_when_idle_clears_panel() {
        let mut app = make_app();
        app.codex_login_lines.push("stale".to_string());
        app.codex_login_outcome = Some(CodexLoginOutcome::Failed("expired".to_string()));

        app.dismiss_codex_login();

        assert!(app.codex_login_lines.is_empty());
        assert!(app.codex_login_outcome.is_none());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Codex login panel dismissed")
        );
    }

    // ── click area management ───────────────────────────────────────

    #[test]
    fn test_clear_click_areas() {
        let mut app = make_app();
        app.add_click_area(Rect::new(0, 0, 10, 10), ClickAction::Tab(Tab::Models));
        app.add_click_area(Rect::new(10, 0, 10, 10), ClickAction::Tab(Tab::Daily));
        assert_eq!(app.click_areas.len(), 2);

        app.clear_click_areas();
        assert_eq!(app.click_areas.len(), 0);
    }

    // ── narrow detection ────────────────────────────────────────────

    #[test]
    fn test_is_narrow() {
        let mut app = make_app();
        app.terminal_width = 79;
        assert!(app.is_narrow());

        app.terminal_width = 80;
        assert!(!app.is_narrow());
    }

    #[test]
    fn test_is_very_narrow() {
        let mut app = make_app();
        app.terminal_width = 59;
        assert!(app.is_very_narrow());

        app.terminal_width = 60;
        assert!(!app.is_very_narrow());
    }

    // ── HourlyViewMode tests ─────────────────────────────────────────

    #[test]
    fn test_hourly_view_mode_default() {
        let mode = HourlyViewMode::default();
        assert_eq!(mode, HourlyViewMode::Table);
    }

    #[test]
    fn test_hourly_view_mode_toggle() {
        let mut app = make_app();
        assert_eq!(app.hourly_view_mode, HourlyViewMode::Table);

        // Toggle to Profile when on Hourly tab
        app.current_tab = Tab::Hourly;
        app.handle_key_event(key(KeyCode::Char('v')));
        assert_eq!(app.hourly_view_mode, HourlyViewMode::Profile);

        // Toggle back to Table
        app.handle_key_event(key(KeyCode::Char('v')));
        assert_eq!(app.hourly_view_mode, HourlyViewMode::Table);
    }

    #[test]
    fn test_hourly_view_mode_no_toggle_on_other_tabs() {
        let mut app = make_app();
        assert_eq!(app.hourly_view_mode, HourlyViewMode::Table);

        // 'v' should not toggle when not on Hourly tab
        app.current_tab = Tab::Overview;
        app.handle_key_event(key(KeyCode::Char('v')));
        assert_eq!(app.hourly_view_mode, HourlyViewMode::Table);

        app.current_tab = Tab::Daily;
        app.handle_key_event(key(KeyCode::Char('v')));
        assert_eq!(app.hourly_view_mode, HourlyViewMode::Table);
    }

    // ── build_model_shade_map ───────────────────────────────────────

    fn model_usage(name: &str, cost: f64, workspace: Option<&str>) -> ModelUsage {
        ModelUsage {
            model: name.to_string(),
            provider: "anthropic".to_string(),
            client: "claude".to_string(),
            workspace_key: workspace.map(String::from),
            workspace_label: workspace.map(String::from),
            tokens: TokenBreakdown::default(),
            cost,
            performance: Default::default(),
            session_count: 1,
        }
    }

    fn shade_key(provider: &str, model: &str) -> String {
        super::super::colors::model_shade_key(provider, model)
    }

    #[test]
    fn test_shade_map_ranks_by_family_hierarchy() {
        let mut app = make_app();
        app.data.models = vec![
            model_usage("claude-haiku-4-5", 10.0, None),
            model_usage("claude-opus-4-5", 100.0, None),
            model_usage("claude-sonnet-4-5", 50.0, None),
        ];
        app.build_model_shade_map();

        let opus = app
            .model_shade_map
            .get(&shade_key("anthropic", "claude-opus-4-5"))
            .copied()
            .unwrap();
        let sonnet = app
            .model_shade_map
            .get(&shade_key("anthropic", "claude-sonnet-4-5"))
            .copied()
            .unwrap();
        let haiku = app
            .model_shade_map
            .get(&shade_key("anthropic", "claude-haiku-4-5"))
            .copied()
            .unwrap();

        // Rank 0 is the base Anthropic coral; ranks below lighten toward white.
        assert_eq!(opus, get_provider_shade("anthropic", 0));
        assert_eq!(sonnet, get_provider_shade("anthropic", 1));
        assert_eq!(haiku, get_provider_shade("anthropic", 2));
    }

    #[test]
    fn test_shade_map_dedupes_same_model_across_workspaces() {
        // Same model appearing N times in different workspaces (as happens
        // under GroupBy::WorkspaceModel) must not inflate the rank count.
        let mut app = make_app();
        app.data.models = vec![
            model_usage("claude-sonnet-4-5", 20.0, Some("ws-a")),
            model_usage("claude-sonnet-4-5", 20.0, Some("ws-b")),
            model_usage("claude-sonnet-4-5", 20.0, Some("ws-c")),
            model_usage("claude-haiku-4-5", 5.0, None),
        ];
        app.build_model_shade_map();

        // Only two distinct model names should be in the map; sonnet takes
        // rank 0 (aggregate cost 60 > haiku cost 5).
        assert_eq!(app.model_shade_map.len(), 2);
        assert_eq!(
            app.model_shade_map
                .get(&shade_key("anthropic", "claude-sonnet-4-5"))
                .copied(),
            Some(get_provider_shade("anthropic", 0))
        );
        assert_eq!(
            app.model_shade_map
                .get(&shade_key("anthropic", "claude-haiku-4-5"))
                .copied(),
            Some(get_provider_shade("anthropic", 1))
        );
    }

    #[test]
    fn test_shade_map_is_deterministic_on_cost_ties() {
        // All-zero costs (fresh data) must produce a stable shade assignment
        // across refreshes so the chart doesn't flicker.
        let ranks = |app: &App| {
            let a = app
                .model_shade_map
                .get(&shade_key("anthropic", "claude-alpha"))
                .copied();
            let b = app
                .model_shade_map
                .get(&shade_key("anthropic", "claude-beta"))
                .copied();
            let c = app
                .model_shade_map
                .get(&shade_key("anthropic", "claude-gamma"))
                .copied();
            (a, b, c)
        };

        let mut app1 = make_app();
        app1.data.models = vec![
            model_usage("claude-gamma", 0.0, None),
            model_usage("claude-alpha", 0.0, None),
            model_usage("claude-beta", 0.0, None),
        ];
        app1.build_model_shade_map();

        let mut app2 = make_app();
        app2.data.models = vec![
            model_usage("claude-beta", 0.0, None),
            model_usage("claude-gamma", 0.0, None),
            model_usage("claude-alpha", 0.0, None),
        ];
        app2.build_model_shade_map();

        assert_eq!(ranks(&app1), ranks(&app2));
        // alpha sorts first by name so it gets rank 0 on ties.
        assert_eq!(
            app1.model_shade_map
                .get(&shade_key("anthropic", "claude-alpha"))
                .copied(),
            Some(get_provider_shade("anthropic", 0))
        );
    }

    #[test]
    fn test_shade_map_handles_nan_cost() {
        // NaN costs must not propagate into total_cmp ordering surprises or
        // crash the builder.
        let mut app = make_app();
        app.data.models = vec![
            model_usage("claude-nan", f64::NAN, None),
            model_usage("claude-normal", 1.0, None),
        ];
        app.build_model_shade_map();

        assert_eq!(app.model_shade_map.len(), 2);
        // Normal model outranks NaN (which is coerced to 0).
        assert_eq!(
            app.model_shade_map
                .get(&shade_key("anthropic", "claude-normal"))
                .copied(),
            Some(get_provider_shade("anthropic", 0))
        );
    }

    #[test]
    fn test_shade_map_separates_providers() {
        let mut app = make_app();
        app.data.models = vec![
            ModelUsage {
                model: "claude-opus-4-5".to_string(),
                provider: "anthropic".to_string(),
                client: "claude".to_string(),
                workspace_key: None,
                workspace_label: None,
                tokens: TokenBreakdown::default(),
                cost: 10.0,
                performance: Default::default(),
                session_count: 1,
            },
            ModelUsage {
                model: "gpt-5".to_string(),
                provider: "openai".to_string(),
                client: "codex".to_string(),
                workspace_key: None,
                workspace_label: None,
                tokens: TokenBreakdown::default(),
                cost: 1.0,
                performance: Default::default(),
                session_count: 1,
            },
        ];
        app.build_model_shade_map();

        // Each provider ranks independently — both get rank-0 shades.
        assert_eq!(
            app.model_shade_map
                .get(&shade_key("anthropic", "claude-opus-4-5"))
                .copied(),
            Some(get_provider_shade("anthropic", 0))
        );
        assert_eq!(
            app.model_shade_map
                .get(&shade_key("openai", "gpt-5"))
                .copied(),
            Some(get_provider_shade("openai", 0))
        );
    }

    #[test]
    fn test_shade_map_rebuilds_on_update_data() {
        let mut app = make_app();
        app.data.models = vec![model_usage("claude-opus-4-5", 10.0, None)];
        app.build_model_shade_map();
        assert!(app
            .model_shade_map
            .contains_key(&shade_key("anthropic", "claude-opus-4-5")));

        let fresh = UsageData {
            models: vec![model_usage("claude-sonnet-4-5", 5.0, None)],
            ..UsageData::default()
        };
        app.update_data(fresh);

        assert!(!app
            .model_shade_map
            .contains_key(&shade_key("anthropic", "claude-opus-4-5")));
        assert!(app
            .model_shade_map
            .contains_key(&shade_key("anthropic", "claude-sonnet-4-5")));
    }

    #[test]
    fn test_same_model_name_keeps_distinct_provider_colors() {
        let mut app = make_app();
        app.data.models = vec![
            ModelUsage {
                model: "sonnet-shared".to_string(),
                provider: "anthropic".to_string(),
                client: "claude".to_string(),
                workspace_key: None,
                workspace_label: None,
                tokens: TokenBreakdown::default(),
                cost: 10.0,
                performance: Default::default(),
                session_count: 1,
            },
            ModelUsage {
                model: "sonnet-shared".to_string(),
                provider: "openai".to_string(),
                client: "codex".to_string(),
                workspace_key: None,
                workspace_label: None,
                tokens: TokenBreakdown::default(),
                cost: 5.0,
                performance: Default::default(),
                session_count: 1,
            },
        ];
        app.build_model_shade_map();

        assert_eq!(
            app.model_color_for("anthropic", "sonnet-shared"),
            app.theme.color(get_provider_shade("anthropic", 0))
        );
        assert_eq!(
            app.model_color_for("openai", "sonnet-shared"),
            app.theme.color(get_provider_shade("openai", 0))
        );
        assert_ne!(
            app.model_color_for("anthropic", "sonnet-shared"),
            app.model_color_for("openai", "sonnet-shared")
        );
    }

    #[test]
    fn test_gateway_provider_model_uses_model_vendor_color() {
        // Regression: a Claude model served through the github-copilot gateway
        // must render in the Anthropic ramp, not the neutral "unknown" gray.
        // The gateway has no vendor palette of its own, so the model's own
        // vendor decides the color.
        let mut app = make_app();
        let copilot_fable = ModelUsage {
            model: "claude-fable-5".to_string(),
            provider: "github-copilot".to_string(),
            client: "opencode".to_string(),
            workspace_key: None,
            workspace_label: None,
            tokens: TokenBreakdown::default(),
            cost: 3.0,
            performance: Default::default(),
            session_count: 1,
        };
        app.data.models = vec![
            ModelUsage {
                provider: "anthropic".to_string(),
                cost: 5.0,
                ..copilot_fable.clone()
            },
            copilot_fable,
        ];
        app.build_model_shade_map();

        // Fable is the flagship tier -> base Anthropic shade.
        let anthropic_base = app.theme.color(get_provider_shade("anthropic", 0));
        assert_eq!(
            app.model_color_for("github-copilot", "claude-fable-5"),
            anthropic_base,
            "copilot-served Claude must use the Anthropic ramp, not unknown gray"
        );
        // Identical to the natively-served model.
        assert_eq!(
            app.model_color_for("anthropic", "claude-fable-5"),
            app.model_color_for("github-copilot", "claude-fable-5")
        );
        // And definitively not the neutral gray a raw github-copilot key yields.
        assert_ne!(
            app.model_color_for("github-copilot", "claude-fable-5"),
            app.theme.color(get_provider_shade("github-copilot", 0))
        );
    }
}
