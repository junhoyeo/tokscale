use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use tokscale_core::ClientId;

use super::data::{AgentUsage, DailyUsage, DataLoader, ModelUsage, UsageData};
use super::settings::Settings;
use super::themes::{Theme, ThemeName};
use super::ui::dialog::{ClientPickerDialog, DialogStack};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Models,
    Daily,
    Stats,
    Agents,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[
            Tab::Overview,
            Tab::Models,
            Tab::Daily,
            Tab::Stats,
            Tab::Agents,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Models => "Models",
            Tab::Daily => "Daily",
            Tab::Stats => "Stats",
            Tab::Agents => "Agents",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Tab::Overview => "Ovw",
            Tab::Models => "Mod",
            Tab::Daily => "Day",
            Tab::Stats => "Sta",
            Tab::Agents => "Agt",
        }
    }

    pub fn next(self) -> Tab {
        match self {
            Tab::Overview => Tab::Models,
            Tab::Models => Tab::Daily,
            Tab::Daily => Tab::Stats,
            Tab::Stats => Tab::Agents,
            Tab::Agents => Tab::Overview,
        }
    }

    pub fn prev(self) -> Tab {
        match self {
            Tab::Overview => Tab::Agents,
            Tab::Models => Tab::Overview,
            Tab::Daily => Tab::Models,
            Tab::Stats => Tab::Daily,
            Tab::Agents => Tab::Stats,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Cost,
    Tokens,
    Date,
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

#[derive(Debug, Clone)]
pub enum ClickAction {
    Tab(Tab),
    Sort(SortField),
    GraphCell { week: usize, day: usize },
}

pub struct App {
    pub should_quit: bool,
    pub current_tab: Tab,
    pub theme: Theme,
    pub settings: Settings,
    pub data: UsageData,
    pub data_loader: DataLoader,

    pub enabled_clients: Rc<RefCell<HashSet<ClientId>>>,
    pub include_synthetic: Rc<RefCell<bool>>,
    pub group_by: Rc<RefCell<tokscale_core::GroupBy>>,
    pub sort_field: SortField,
    pub sort_direction: SortDirection,

    pub scroll_offset: usize,
    pub selected_index: usize,
    pub max_visible_items: usize,

    pub selected_graph_cell: Option<(usize, usize)>,
    pub stats_breakdown_total_lines: usize,

    pub auto_refresh: bool,
    pub auto_refresh_interval: Duration,
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
}

impl App {
    pub fn new_with_cached_data(config: TuiConfig, cached_data: Option<UsageData>) -> Result<Self> {
        let settings = Settings::load();
        let theme_name: ThemeName = config
            .theme
            .parse()
            .unwrap_or_else(|_| settings.theme_name());
        let theme = Theme::from_name(theme_name);

        let mut enabled_clients = HashSet::new();
        let mut include_synthetic = false;

        if let Some(ref cli_clients) = config.clients {
            for client_str in cli_clients {
                if client_str.eq_ignore_ascii_case("synthetic") {
                    include_synthetic = true;
                } else if let Some(client) = ClientId::from_str(client_str) {
                    enabled_clients.insert(client);
                }
            }
        } else {
            for client in ClientId::iter() {
                enabled_clients.insert(client);
            }
        }

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
        );

        let data = cached_data.unwrap_or_default();
        let has_data = !data.models.is_empty();
        let dialog_stack = DialogStack::new(theme.clone());
        let dialog_needs_reload = Rc::new(RefCell::new(false));

        Ok(Self {
            should_quit: false,
            current_tab: config.initial_tab.unwrap_or(Tab::Overview),
            theme,
            settings,
            data,
            data_loader,
            enabled_clients: Rc::new(RefCell::new(enabled_clients)),
            include_synthetic: Rc::new(RefCell::new(include_synthetic)),
            group_by: Rc::new(RefCell::new(tokscale_core::GroupBy::Model)),
            sort_field: SortField::Cost,
            sort_direction: SortDirection::Descending,
            scroll_offset: 0,
            selected_index: 0,
            max_visible_items: 20,
            selected_graph_cell: None,
            stats_breakdown_total_lines: 0,
            auto_refresh,
            auto_refresh_interval,
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
        })
    }

    pub fn set_background_loading(&mut self, loading: bool) {
        self.background_loading = loading;
        // Don't set data.loading - let cached data remain visible during background refresh
    }

    pub fn update_data(&mut self, data: UsageData) {
        self.data = data;
        self.last_refresh = Instant::now();
        self.clamp_selection();
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

        if self.auto_refresh
            && !self.background_loading
            && self.last_refresh.elapsed() >= self.auto_refresh_interval
        {
            self.needs_reload = true;
        }

        if *self.dialog_needs_reload.borrow() {
            *self.dialog_needs_reload.borrow_mut() = false;
            self.needs_reload = true;
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return true;
        }

        if self.dialog_stack.is_active() {
            self.dialog_stack.handle_key(key.code);
            return false;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return true;
            }
            KeyCode::Tab => {
                self.current_tab = self.current_tab.next();
                self.reset_selection();
            }
            KeyCode::BackTab => {
                self.current_tab = self.current_tab.prev();
                self.reset_selection();
            }
            KeyCode::Left => {
                self.current_tab = self.current_tab.prev();
                self.reset_selection();
            }
            KeyCode::Right => {
                self.current_tab = self.current_tab.next();
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
                if self.background_loading {
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
            KeyCode::Char('g') => {
                self.open_group_by_picker();
            }
            KeyCode::Enter => {
                if self.current_tab == Tab::Stats {
                    self.handle_graph_selection();
                }
            }
            KeyCode::Esc => {
                if self.selected_graph_cell.is_some() {
                    self.selected_graph_cell = None;
                    self.stats_breakdown_total_lines = 0;
                    self.selected_index = 0;
                    self.scroll_offset = 0;
                }
            }
            _ => {}
        }
        false
    }

    pub fn handle_mouse_event(&mut self, event: MouseEvent) {
        if self.dialog_stack.is_active() {
            self.dialog_stack.handle_mouse(event);
            return;
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let x = event.column;
                let y = event.row;

                for area in &self.click_areas {
                    if x >= area.rect.x
                        && x < area.rect.x + area.rect.width
                        && y >= area.rect.y
                        && y < area.rect.y + area.rect.height
                    {
                        match &area.action {
                            ClickAction::Tab(tab) => {
                                self.current_tab = *tab;
                                self.reset_selection();
                            }
                            ClickAction::Sort(field) => {
                                self.set_sort(*field);
                            }
                            ClickAction::GraphCell { week, day } => {
                                self.selected_graph_cell = Some((*week, *day));
                                self.stats_breakdown_total_lines = 0;
                                self.selected_index = 0;
                                self.scroll_offset = 0;
                            }
                        }
                        break;
                    }
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

    pub fn handle_resize(&mut self, width: u16, height: u16) {
        self.terminal_width = width;
        self.terminal_height = height;
        // Ensure at least 1 visible item to prevent division/slice issues
        self.max_visible_items = (height.saturating_sub(10) as usize).max(1);
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
        self.selected_graph_cell = None;
        self.stats_breakdown_total_lines = 0;
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
            Tab::Daily => self.data.daily.len(),
            Tab::Stats => {
                if self.selected_graph_cell.is_some() {
                    self.stats_breakdown_total_lines
                } else {
                    0
                }
            }
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
        self.reset_selection();
        self.set_status(&format!(
            "Sorted by {:?} {:?}",
            self.sort_field, self.sort_direction
        ));
    }

    fn jump_to_today(&mut self) {
        if self.current_tab != Tab::Daily {
            return;
        }

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
        self.theme = Theme::from_name(new_theme);
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
            self.include_synthetic.clone(),
            self.dialog_needs_reload.clone(),
        );
        self.dialog_stack.show(Box::new(dialog));
    }

    fn open_group_by_picker(&mut self) {
        use super::ui::dialog::GroupByPickerDialog;
        let dialog =
            GroupByPickerDialog::new(self.group_by.clone(), self.dialog_needs_reload.clone());
        self.dialog_stack.show(Box::new(dialog));
    }

    fn toggle_auto_refresh(&mut self) {
        self.auto_refresh = !self.auto_refresh;
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
            Tab::Daily => self
                .get_sorted_daily()
                .get(self.selected_index)
                .map(|d| format!("{}: {} tokens, ${:.4}", d.date, d.tokens.total(), d.cost)),
            Tab::Stats => None,
        };

        if let Some(text) = text {
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&text)) {
                Ok(_) => self.set_status("Copied to clipboard"),
                Err(_) => self.set_status("Failed to copy"),
            }
        }
    }

    fn export_to_json(&mut self) {
        let export_data = serde_json::json!({
            "models": self.data.models.iter().map(|m| serde_json::json!({
                "model": m.model,
                "provider": m.provider,
                "client": m.client,
                "tokens": {
                    "input": m.tokens.input,
                    "output": m.tokens.output,
                    "cacheRead": m.tokens.cache_read,
                    "cacheWrite": m.tokens.cache_write,
                    "total": m.tokens.total()
                },
                "cost": m.cost,
                "sessionCount": m.session_count
            })).collect::<Vec<_>>(),
            "agents": self.data.agents.iter().map(|a| serde_json::json!({
                "agent": a.agent,
                "clients": a.clients,
                "tokens": {
                    "input": a.tokens.input,
                    "output": a.tokens.output,
                    "cacheRead": a.tokens.cache_read,
                    "cacheWrite": a.tokens.cache_write,
                    "total": a.tokens.total()
                },
                "cost": a.cost,
                "messageCount": a.message_count
            })).collect::<Vec<_>>(),
            "daily": self.data.daily.iter().map(|d| serde_json::json!({
                "date": d.date.to_string(),
                "tokens": {
                    "input": d.tokens.input,
                    "output": d.tokens.output,
                    "cacheRead": d.tokens.cache_read,
                    "cacheWrite": d.tokens.cache_write,
                    "total": d.tokens.total()
                },
                "cost": d.cost
            })).collect::<Vec<_>>(),
            "totals": {
                "tokens": self.data.total_tokens,
                "cost": self.data.total_cost
            }
        });

        let filename = format!(
            "tokscale-export-{}.json",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );

        match serde_json::to_string_pretty(&export_data) {
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
                daily.sort_by(|a, b| b.date.cmp(&a.date))
            }
            (SortField::Date, SortDirection::Ascending) => {
                daily.sort_by(|a, b| a.date.cmp(&b.date))
            }
        }

        daily
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
    use super::*;
    use crate::tui::data::{ModelUsage, TokenBreakdown};

    #[test]
    fn test_tab_all() {
        let tabs = Tab::all();
        assert_eq!(tabs.len(), 5);
        assert_eq!(tabs[0], Tab::Overview);
        assert_eq!(tabs[1], Tab::Models);
        assert_eq!(tabs[2], Tab::Daily);
        assert_eq!(tabs[3], Tab::Stats);
        assert_eq!(tabs[4], Tab::Agents);
    }

    #[test]
    fn test_tab_next() {
        assert_eq!(Tab::Overview.next(), Tab::Models);
        assert_eq!(Tab::Models.next(), Tab::Daily);
        assert_eq!(Tab::Daily.next(), Tab::Stats);
        assert_eq!(Tab::Stats.next(), Tab::Agents);
        assert_eq!(Tab::Agents.next(), Tab::Overview);
    }

    #[test]
    fn test_tab_prev() {
        assert_eq!(Tab::Overview.prev(), Tab::Agents);
        assert_eq!(Tab::Models.prev(), Tab::Overview);
        assert_eq!(Tab::Daily.prev(), Tab::Models);
        assert_eq!(Tab::Stats.prev(), Tab::Daily);
        assert_eq!(Tab::Agents.prev(), Tab::Stats);
    }

    #[test]
    fn test_tab_as_str() {
        assert_eq!(Tab::Overview.as_str(), "Overview");
        assert_eq!(Tab::Models.as_str(), "Models");
        assert_eq!(Tab::Agents.as_str(), "Agents");
        assert_eq!(Tab::Daily.as_str(), "Daily");
        assert_eq!(Tab::Stats.as_str(), "Stats");
    }

    #[test]
    fn test_tab_short_name() {
        assert_eq!(Tab::Overview.short_name(), "Ovw");
        assert_eq!(Tab::Models.short_name(), "Mod");
        assert_eq!(Tab::Agents.short_name(), "Agt");
        assert_eq!(Tab::Daily.short_name(), "Day");
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
                session_count: 1,
            },
            ModelUsage {
                model: "model2".to_string(),
                provider: "provider2".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                session_count: 1,
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
                session_count: 1,
            },
            ModelUsage {
                model: "model2".to_string(),
                provider: "provider2".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                session_count: 1,
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
            session_count: 1,
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

        assert_eq!(app.should_quit, false);
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

    fn make_app_with_models(n: usize) -> App {
        let mut app = make_app();
        app.data.models = (0..n)
            .map(|i| ModelUsage {
                model: format!("model{}", i),
                provider: "provider".to_string(),
                client: "opencode".to_string(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                session_count: 1,
            })
            .collect();
        app
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

    // ── handle_key_event: tab switching ─────────────────────────────

    #[test]
    fn test_handle_key_tab_switch() {
        let mut app = make_app();
        assert_eq!(app.current_tab, Tab::Overview);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Models);

        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.current_tab, Tab::Daily);

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
        assert_eq!(app.current_tab, Tab::Daily);

        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.current_tab, Tab::Models);
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
        assert_eq!(app.current_tab, Tab::Models);

        app.handle_key_event(key(KeyCode::Left));
        assert_eq!(app.current_tab, Tab::Overview);
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

    // ── handle_key_event: theme ─────────────────────────────────────

    #[test]
    fn test_handle_key_theme_cycle() {
        let mut app = make_app();
        let initial_theme = app.theme.name;

        app.handle_key_event(key(KeyCode::Char('p')));
        assert_ne!(app.theme.name, initial_theme);

        for _ in 0..8 {
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
        assert_eq!(
            app.status_message.as_deref(),
            Some("Refresh already in progress")
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
        assert_eq!(app.max_visible_items, 30);
    }

    #[test]
    fn test_handle_resize_small_terminal() {
        let mut app = make_app();
        app.handle_resize(40, 12);
        assert_eq!(app.terminal_width, 40);
        assert_eq!(app.terminal_height, 12);
        assert_eq!(app.max_visible_items, 2);
    }

    #[test]
    fn test_handle_resize_clamps_selection() {
        let mut app = make_app_with_models(5);
        app.selected_index = 4;
        app.scroll_offset = 3;
        app.max_visible_items = 20;

        app.handle_resize(80, 24);
        assert!(app.selected_index <= 4);
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
}
