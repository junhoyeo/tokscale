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
use unicode_width::UnicodeWidthStr;

use crate::tui::i18n::{tr, MessageKey, TuiLanguage};
use crate::tui::themes::Theme;
use crate::tui::ui::widgets::AMBIENT_STABLE_BORDER_SET;

use super::{DialogContent, DialogResult};

pub struct LanguagePickerDialog {
    options: Vec<TuiLanguage>,
    selected: Rc<RefCell<TuiLanguage>>,
    needs_save: Rc<RefCell<bool>>,
    cursor: usize,
}

impl LanguagePickerDialog {
    pub fn new(selected: Rc<RefCell<TuiLanguage>>, needs_save: Rc<RefCell<bool>>) -> Self {
        let initial = *selected.borrow();
        let options = TuiLanguage::ALL.to_vec();
        let cursor = options.iter().position(|l| *l == initial).unwrap_or(0);

        Self {
            options,
            selected,
            needs_save,
            cursor,
        }
    }

    fn select_current(&mut self) {
        let new_val = self.options[self.cursor];
        *self.selected.borrow_mut() = new_val;
        *self.needs_save.borrow_mut() = true;
    }
}

impl DialogContent for LanguagePickerDialog {
    fn desired_size(&self, viewport: Rect) -> (u16, u16) {
        let width = 48u16.min(viewport.width.saturating_sub(4));
        let height = 15u16.min(viewport.height.saturating_sub(4));
        (width, height)
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let current_lang = *self.selected.borrow();
        let title = tr(current_lang, MessageKey::LanguageDialogTitle);
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_set(AMBIENT_STABLE_BORDER_SET)
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

        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                tr(current_lang, MessageKey::DialogCurrentLabel),
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                format!("{} ({})", current_lang.native_name(), current_lang.code()),
                Style::default().fg(theme.accent),
            ),
        ]));
        frame.render_widget(header, rows[0]);

        let divider = Paragraph::new("-".repeat(rows[1].width as usize))
            .style(Style::default().fg(theme.border));
        frame.render_widget(divider, rows[1]);

        let list_area = rows[2];
        let mut items: Vec<ListItem> = Vec::new();

        for (i, &lang) in self.options.iter().enumerate() {
            let is_cursor = i == self.cursor;
            let is_active = current_lang == lang;

            let radio = if is_active { "(●)" } else { "( )" };
            let left = format!("{} {} [{}]", radio, lang.native_name(), lang.code());
            let usable = list_area.width.saturating_sub(4) as usize;

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

            let str_width = UnicodeWidthStr::width(left.as_str());
            let padding = usable.saturating_sub(str_width);

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", left), base_style),
                Span::styled(" ".repeat(padding), base_style),
            ])));
        }

        frame.render_widget(List::new(items), list_area);

        let hint_text = tr(current_lang, MessageKey::LanguageDialogHint);
        let hint = Paragraph::new(hint_text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.muted));
        frame.render_widget(hint, rows[3]);
    }

    // Esc cancellation is handled directly by `DialogStack::handle_key`,
    // which closes the dialog without invoking select_current().
    fn handle_key(&mut self, key: KeyCode) -> DialogResult {
        match key {
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
