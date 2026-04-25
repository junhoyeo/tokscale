use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use clap::ValueEnum;
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::client_ui;
use crate::tui::themes::Theme;
use crate::ClientFilter;

use super::{DialogContent, DialogResult};

/// Hotkey assigned to the Synthetic option in the dialog.
///
/// NOTE: `'x'` collides with `client_ui::CLIENT_UI[Mux]`. The collision is
/// pre-existing — the toggle path checks `client_ui::from_hotkey` first, so
/// pressing `x` toggles Mux and the Synthetic hotkey display is purely
/// cosmetic. Left as-is here so this refactor stays scope-clean; tracked
/// for a follow-up that picks a free letter.
const SYNTHETIC_HOTKEY: char = 'x';

/// TUI dialog that lets the user toggle which clients (and Synthetic) are
/// included in reports. Backed by the same unified
/// `Rc<RefCell<HashSet<ClientFilter>>>` the rest of the app sees, so
/// toggles propagate without a separate sync step.
pub struct ClientPickerDialog {
    /// Every selectable filter in the same order they appear on screen.
    /// Mirrors `ClientFilter::value_variants()` so the listing order is
    /// the canonical chronological order across the whole CLI/TUI.
    sources: Vec<ClientFilter>,
    enabled: Rc<RefCell<HashSet<ClientFilter>>>,
    needs_reload: Rc<RefCell<bool>>,
    selected: usize,
    filter: String,
    /// Indices into `sources` that match the current type-to-filter
    /// substring. `selected` indexes into this vec, not into `sources`.
    filtered_indices: Vec<usize>,
}

impl ClientPickerDialog {
    pub fn new(
        enabled: Rc<RefCell<HashSet<ClientFilter>>>,
        needs_reload: Rc<RefCell<bool>>,
    ) -> Self {
        let sources: Vec<ClientFilter> = ClientFilter::value_variants().to_vec();
        let filtered_indices: Vec<usize> = (0..sources.len()).collect();
        Self {
            sources,
            enabled,
            needs_reload,
            selected: 0,
            filter: String::new(),
            filtered_indices,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered_indices.is_empty() {
            self.selected = 0;
            return;
        }
        let max = self.filtered_indices.len() as isize;
        let mut next = self.selected as isize + delta;
        if next < 0 {
            next = max - 1;
        } else if next >= max {
            next = 0;
        }
        self.selected = next as usize;
    }

    /// Toggle the currently highlighted source. Refuses to disable the
    /// last enabled source (downstream code assumes at least one
    /// filter is active when the picker is in use).
    fn toggle_selected(&mut self) {
        if let Some(&idx) = self.filtered_indices.get(self.selected) {
            self.toggle(self.sources[idx]);
        }
    }

    fn toggle(&self, client: ClientFilter) {
        let mut enabled = self.enabled.borrow_mut();
        let total = enabled.len();
        let is_enabled = enabled.contains(&client);

        if is_enabled && total > 1 {
            enabled.remove(&client);
            *self.needs_reload.borrow_mut() = true;
        } else if !is_enabled {
            enabled.insert(client);
            *self.needs_reload.borrow_mut() = true;
        }
        // Refusing to disable the last source is intentional: an empty
        // filter set would mean "scan nothing", which is never what the
        // user wants from this dialog.
    }

    fn rebuild_filter(&mut self) {
        let needle = self.filter.to_lowercase();
        if needle.is_empty() {
            self.filtered_indices = (0..self.sources.len()).collect();
        } else {
            self.filtered_indices = self
                .sources
                .iter()
                .enumerate()
                .filter(|(_, c)| display_name(**c).to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect();
        }
        if self.selected >= self.filtered_indices.len() {
            self.selected = 0;
        }
    }
}

impl DialogContent for ClientPickerDialog {
    fn desired_size(&self, viewport: Rect) -> (u16, u16) {
        let width = 50u16.min(viewport.width.saturating_sub(4));
        let height = 18u16.min(viewport.height.saturating_sub(4));
        (width, height)
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .title(" Clients ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(inner);

        let filter_text = if self.filter.is_empty() {
            Span::styled("Type to filter...", Style::default().fg(theme.muted))
        } else {
            Span::styled(&self.filter, Style::default().fg(theme.foreground))
        };
        let filter_line = Paragraph::new(Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(theme.accent)),
            filter_text,
        ]));
        frame.render_widget(filter_line, rows[0]);

        let divider = Paragraph::new("-".repeat(rows[1].width as usize))
            .style(Style::default().fg(theme.border));
        frame.render_widget(divider, rows[1]);

        let list_area = rows[2];
        let visible_height = list_area.height as usize;
        let scroll = if self.selected >= visible_height && visible_height > 0 {
            self.selected.saturating_sub(visible_height - 1)
        } else {
            0
        };

        let mut items: Vec<ListItem> = Vec::new();
        for (flat_idx, &idx) in self.filtered_indices.iter().enumerate() {
            if flat_idx < scroll {
                continue;
            }
            if items.len() >= visible_height {
                break;
            }

            let source = self.sources[idx];
            let is_selected = flat_idx == self.selected;
            let is_enabled = self.enabled.borrow().contains(&source);

            let checkbox = if is_enabled { "[●]" } else { "[ ]" };
            let key_hint = format!("[{}]", hotkey(source));
            let name = display_name(source);

            let usable = list_area.width.saturating_sub(4) as usize;
            let left = format!("{} {} {}", checkbox, key_hint, name);
            let padding = usable.saturating_sub(left.chars().count());

            let base_style = if is_selected {
                Style::default()
                    .bg(theme.accent)
                    .fg(theme.background)
                    .add_modifier(Modifier::BOLD)
            } else if is_enabled {
                Style::default().fg(theme.foreground)
            } else {
                Style::default().fg(theme.muted)
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", left), base_style),
                Span::styled(" ".repeat(padding), base_style),
            ])));
        }

        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "  No results",
                Style::default().fg(theme.muted),
            ))));
        }

        frame.render_widget(List::new(items), list_area);

        let hint = Paragraph::new("↑↓ navigate • Enter toggle • Esc close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.muted));
        frame.render_widget(hint, rows[3]);
    }

    fn handle_key(&mut self, key: KeyCode) -> DialogResult {
        match key {
            KeyCode::Esc => DialogResult::Close,
            KeyCode::Up => {
                self.move_selection(-1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.move_selection(1);
                DialogResult::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.toggle_selected();
                DialogResult::None
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.rebuild_filter();
                DialogResult::None
            }
            KeyCode::Char(c) => {
                // Hotkey toggle: route through the centralized
                // `ClientFilter` mapping so adding a new hotkey only
                // requires editing `client_ui.rs` + (if it's a non-client
                // meta source) updating SYNTHETIC_HOTKEY here.
                if let Some(client_id) = client_ui::from_hotkey(c) {
                    self.toggle(ClientFilter::from_client_id(client_id));
                } else if c == SYNTHETIC_HOTKEY {
                    self.toggle(ClientFilter::Synthetic);
                } else {
                    self.filter.push(c);
                    self.rebuild_filter();
                }
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }
}

/// Display name for a `ClientFilter` row in the picker. Delegates to the
/// existing `client_ui` registry for `ClientId`-backed variants and adds
/// the meta-client label for `Synthetic`.
fn display_name(client: ClientFilter) -> &'static str {
    match client.to_client_id() {
        Some(id) => client_ui::display_name(id),
        None => "Synthetic",
    }
}

/// Hotkey for a `ClientFilter` row. Mirrors `display_name`'s split.
fn hotkey(client: ClientFilter) -> char {
    match client.to_client_id() {
        Some(id) => client_ui::hotkey(id),
        None => SYNTHETIC_HOTKEY,
    }
}
