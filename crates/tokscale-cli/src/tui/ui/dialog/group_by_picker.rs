use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::KeyCode;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use tokscale_core::GroupBy;

use crate::tui::themes::Theme;

use super::{DialogContent, DialogResult};

pub struct GroupByPickerDialog {
    options: Vec<GroupByOption>,
    selected: Rc<RefCell<GroupBy>>,
    needs_reload: Rc<RefCell<bool>>,
    cursor: usize,
}

struct GroupByOption {
    value: GroupBy,
    label: &'static str,
    description: &'static str,
}

impl GroupByPickerDialog {
    pub fn new(selected: Rc<RefCell<GroupBy>>, needs_reload: Rc<RefCell<bool>>) -> Self {
        let current = selected.borrow().clone();
        let options = vec![
            GroupByOption {
                value: GroupBy::Model,
                label: "Model",
                description: "One row per model (merge clients & providers)",
            },
            GroupByOption {
                value: GroupBy::ClientModel,
                label: "Client + Model",
                description: "One row per client-model pair (default)",
            },
            GroupByOption {
                value: GroupBy::ClientProviderModel,
                label: "Client + Provider + Model",
                description: "Most granular — no merging",
            },
            GroupByOption {
                value: GroupBy::WorkspaceModel,
                label: "Workspace + Model",
                description: "Group local usage by workspace key, then model",
            },
        ];

        let cursor = options.iter().position(|o| o.value == current).unwrap_or(1);

        Self {
            options,
            selected,
            needs_reload,
            cursor,
        }
    }

    fn select_current(&mut self) {
        let new_value = self.options[self.cursor].value.clone();
        let changed = *self.selected.borrow() != new_value;
        if changed {
            *self.selected.borrow_mut() = new_value;
            *self.needs_reload.borrow_mut() = true;
        }
    }
}

impl DialogContent for GroupByPickerDialog {
    fn desired_size(&self, viewport: Rect) -> (u16, u16) {
        let width = 52u16.min(viewport.width.saturating_sub(4));
        let height = 14u16.min(viewport.height.saturating_sub(4));
        (width, height)
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .title(" Group By ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(inner);

        let current = self.selected.borrow();
        let header = Paragraph::new(Line::from(vec![
            Span::styled("Current: ", Style::default().fg(theme.muted)),
            Span::styled(current.to_string(), Style::default().fg(theme.accent)),
        ]));
        frame.render_widget(header, rows[0]);

        let divider = Paragraph::new("-".repeat(rows[1].width as usize))
            .style(Style::default().fg(theme.border));
        frame.render_widget(divider, rows[1]);

        let list_area = rows[2];
        let mut items: Vec<ListItem> = Vec::new();

        for (i, opt) in self.options.iter().enumerate() {
            let is_cursor = i == self.cursor;
            let is_active = *current == opt.value;

            let radio = if is_active { "(●)" } else { "( )" };
            let usable = list_area.width.saturating_sub(4) as usize;
            let left = format!("{} {}", radio, opt.label);
            let desc = format!("    {}", opt.description);

            let base_style = if is_cursor {
                Style::default()
                    .bg(theme.accent)
                    .fg(theme.background)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default().fg(theme.foreground)
            } else {
                Style::default().fg(theme.muted)
            };

            let desc_style = if is_cursor {
                Style::default().bg(theme.accent).fg(theme.background)
            } else {
                Style::default().fg(theme.muted)
            };

            let padding = usable.saturating_sub(left.chars().count());
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", left), base_style),
                Span::styled(" ".repeat(padding), base_style),
            ])));

            let desc_padding = usable.saturating_sub(desc.chars().count());
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", desc), desc_style),
                Span::styled(" ".repeat(desc_padding), desc_style),
            ])));
        }

        frame.render_widget(List::new(items), list_area);

        let hint = Paragraph::new("↑↓ navigate • Enter select • Esc close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.muted));
        frame.render_widget(hint, rows[3]);
    }

    fn handle_key(&mut self, key: KeyCode) -> DialogResult {
        match key {
            KeyCode::Esc => DialogResult::Close,
            KeyCode::Up => {
                if self.cursor == 0 {
                    self.cursor = self.options.len() - 1;
                } else {
                    self.cursor -= 1;
                }
                DialogResult::None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1) % self.options.len();
                DialogResult::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.select_current();
                DialogResult::Close
            }
            _ => DialogResult::None,
        }
    }
}
