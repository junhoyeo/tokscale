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
use crate::ui_glyphs::Glyphs;

use super::{DialogContent, DialogResult};

pub struct GroupByPickerDialog {
    options: Vec<GroupByOption>,
    selected: Rc<RefCell<GroupBy>>,
    needs_reload: Rc<RefCell<bool>>,
    cursor: usize,
    lang: crate::tui::i18n::TuiLanguage,
}

struct GroupByOption {
    value: GroupBy,
    label: &'static str,
    description: &'static str,
}

impl GroupByPickerDialog {
    pub fn new(
        selected: Rc<RefCell<GroupBy>>,
        needs_reload: Rc<RefCell<bool>>,
        lang: crate::tui::i18n::TuiLanguage,
    ) -> Self {
        use crate::tui::i18n::{tr, MessageKey};
        let current = selected.borrow().clone();
        let options = vec![
            GroupByOption {
                value: GroupBy::Model,
                label: tr(lang, MessageKey::GroupByModelLabel),
                description: tr(lang, MessageKey::GroupByModelDesc),
            },
            GroupByOption {
                value: GroupBy::ClientModel,
                label: tr(lang, MessageKey::GroupByClientModelLabel),
                description: tr(lang, MessageKey::GroupByClientModelDesc),
            },
            GroupByOption {
                value: GroupBy::ClientProviderModel,
                label: tr(lang, MessageKey::GroupByClientProviderModelLabel),
                description: tr(lang, MessageKey::GroupByClientProviderModelDesc),
            },
            GroupByOption {
                value: GroupBy::WorkspaceModel,
                label: tr(lang, MessageKey::GroupByWorkspaceModelLabel),
                description: tr(lang, MessageKey::GroupByWorkspaceModelDesc),
            },
            GroupByOption {
                value: GroupBy::Session,
                label: tr(lang, MessageKey::GroupBySessionLabel),
                description: tr(lang, MessageKey::GroupBySessionDesc),
            },
            GroupByOption {
                value: GroupBy::ClientSession,
                label: tr(lang, MessageKey::GroupByClientSessionLabel),
                description: tr(lang, MessageKey::GroupByClientSessionDesc),
            },
            GroupByOption {
                value: GroupBy::MachineModel,
                label: tr(lang, MessageKey::GroupByMachineModelLabel),
                description: tr(lang, MessageKey::GroupByMachineModelDesc),
            },
        ];

        let cursor = options.iter().position(|o| o.value == current).unwrap_or(1);

        Self {
            options,
            selected,
            needs_reload,
            cursor,
            lang,
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
        // 7 options render as 2 lines each (label + description) = 14 rows,
        // plus header (1) + divider (1) + hint (1) + borders (2). Cap at 20
        // so every option stays visible without scrolling on a typical
        // terminal.
        let width = 52u16.min(viewport.width.saturating_sub(4));
        let height = 20u16.min(viewport.height.saturating_sub(4));
        (width, height)
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme, glyphs: Glyphs) {
        use crate::tui::i18n::{tr, tr_with_glyphs, MessageKey};
        use unicode_width::UnicodeWidthStr;
        let block = Block::default()
            .title(tr(self.lang, MessageKey::GroupByDialogTitle))
            .borders(Borders::ALL)
            .border_set(glyphs.border)
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
        let current_display = self
            .options
            .iter()
            .find(|o| o.value == *current)
            .map(|o| o.label.to_string())
            .unwrap_or_else(|| current.to_string());
        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                tr(self.lang, MessageKey::DialogCurrentLabel),
                Style::default().fg(theme.muted),
            ),
            Span::styled(current_display, Style::default().fg(theme.accent)),
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

            let radio = if is_active {
                glyphs.selected_radio
            } else {
                "( )"
            };
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

            let padding = usable.saturating_sub(UnicodeWidthStr::width(left.as_str()));
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", left), base_style),
                Span::styled(" ".repeat(padding), base_style),
            ])));

            let desc_padding = usable.saturating_sub(UnicodeWidthStr::width(desc.as_str()));
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", desc), desc_style),
                Span::styled(" ".repeat(desc_padding), desc_style),
            ])));
        }

        frame.render_widget(List::new(items), list_area);

        let hint = Paragraph::new(tr_with_glyphs(
            self.lang,
            MessageKey::GroupByDialogHint,
            glyphs,
        ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::i18n::TuiLanguage;

    fn picker(current: GroupBy) -> GroupByPickerDialog {
        GroupByPickerDialog::new(
            Rc::new(RefCell::new(current)),
            Rc::new(RefCell::new(false)),
            TuiLanguage::En,
        )
    }

    #[test]
    fn picker_offers_machine_and_model() {
        let dialog = picker(GroupBy::Model);
        let option = dialog
            .options
            .iter()
            .find(|option| option.value == GroupBy::MachineModel)
            .expect("machine,model is offered");
        assert_eq!(option.label, "Machine + Model");
    }

    #[test]
    fn choosing_machine_and_model_requests_a_reload() {
        let selected = Rc::new(RefCell::new(GroupBy::Model));
        let needs_reload = Rc::new(RefCell::new(false));
        let mut dialog =
            GroupByPickerDialog::new(selected.clone(), needs_reload.clone(), TuiLanguage::En);
        dialog.cursor = dialog
            .options
            .iter()
            .position(|option| option.value == GroupBy::MachineModel)
            .unwrap();
        dialog.select_current();
        assert_eq!(*selected.borrow(), GroupBy::MachineModel);
        assert!(*needs_reload.borrow());
    }

    #[test]
    fn every_option_fits_the_dialog_height() {
        let dialog = picker(GroupBy::MachineModel);
        let (_, height) = dialog.desired_size(Rect::new(0, 0, 200, 60));
        // Two lines per option, plus header, divider, hint and borders.
        assert!(height as usize >= dialog.options.len() * 2 + 5);
        assert_eq!(
            dialog.cursor,
            dialog.options.len() - 1,
            "the cursor starts on the active grouping"
        );
    }
}
