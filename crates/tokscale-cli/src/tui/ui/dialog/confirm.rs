use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::themes::Theme;
use crate::tui::ui::widgets::truncate_ellipsis as truncate;

use super::{DialogContent, DialogResult};

#[derive(Clone, Copy)]
enum ConfirmTone {
    Accent,
    Warning,
    Danger,
}

pub struct ConfirmDialog {
    value: String,
    title: &'static str,
    message: &'static str,
    target_label: String,
    effect: &'static str,
    confirm_label: &'static str,
    confirm_verb: &'static str,
    tone: ConfirmTone,
    confirmed_value: Rc<RefCell<Option<String>>>,
}

struct ButtonLayout {
    confirm: Option<Rect>,
    cancel: Option<Rect>,
}

impl ConfirmDialog {
    pub fn codex_switch(
        account_id: String,
        account_label: String,
        confirmed_value: Rc<RefCell<Option<String>>>,
    ) -> Self {
        Self {
            value: account_id,
            title: " Switch Codex Account ",
            message: "This will replace the active Codex auth.json account.",
            target_label: account_label,
            effect: "New Codex requests use this account",
            confirm_label: "Confirm",
            confirm_verb: "confirm",
            tone: ConfirmTone::Accent,
            confirmed_value,
        }
    }

    pub fn codex_remove(
        account_id: String,
        account_label: String,
        confirmed_value: Rc<RefCell<Option<String>>>,
    ) -> Self {
        Self {
            value: account_id,
            title: " Remove Codex Account ",
            message: "This will remove the saved Codex account from Tokscale.",
            target_label: account_label,
            effect: "Saved account is deleted; codex CLI login is unchanged",
            confirm_label: "Remove",
            confirm_verb: "remove",
            tone: ConfirmTone::Danger,
            confirmed_value,
        }
    }

    pub fn codex_reset(
        account_id: String,
        account_label: String,
        confirmed_value: Rc<RefCell<Option<String>>>,
    ) -> Self {
        Self {
            value: account_id,
            title: " Reset Codex Limits ",
            message: "This will consume one available Codex reset credit.",
            target_label: account_label,
            effect: "Codex rate-limit windows reset for this account",
            confirm_label: "Reset",
            confirm_verb: "reset",
            tone: ConfirmTone::Warning,
            confirmed_value,
        }
    }

    fn confirm(&self) {
        *self.confirmed_value.borrow_mut() = Some(self.value.clone());
    }

    fn tone_color(&self, theme: &Theme) -> Color {
        match self.tone {
            ConfirmTone::Accent => theme.accent,
            ConfirmTone::Warning => Color::Yellow,
            ConfirmTone::Danger => Color::Red,
        }
    }

    fn confirm_button_style(&self, theme: &Theme) -> Style {
        match self.tone {
            ConfirmTone::Accent => Style::default().fg(theme.background).bg(theme.accent),
            ConfirmTone::Warning => Style::default().fg(Color::Black).bg(Color::Yellow),
            ConfirmTone::Danger => Style::default().fg(Color::Black).bg(Color::Red),
        }
        .add_modifier(Modifier::BOLD)
    }

    fn content_area(area: Rect) -> Rect {
        Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(1),
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        )
    }

    fn button_y(inner: Rect) -> Option<u16> {
        match inner.height {
            0..=4 => None,
            5 => Some(inner.y.saturating_add(4)),
            _ => Some(inner.y.saturating_add(inner.height.saturating_sub(2))),
        }
    }

    fn button_layout(&self, inner: Rect) -> ButtonLayout {
        let Some(y) = Self::button_y(inner) else {
            return ButtonLayout {
                confirm: None,
                cancel: None,
            };
        };

        let confirm_width = self.confirm_button().chars().count() as u16;
        if inner.width < confirm_width {
            return ButtonLayout {
                confirm: None,
                cancel: None,
            };
        }

        let cancel_width = 10u16;
        let gap = 2u16;
        let total = confirm_width
            .saturating_add(gap)
            .saturating_add(cancel_width);
        if inner.width >= total {
            let x = inner.x + inner.width.saturating_sub(total) / 2;
            return ButtonLayout {
                confirm: Some(Rect::new(x, y, confirm_width, 1)),
                cancel: Some(Rect::new(x + confirm_width + gap, y, cancel_width, 1)),
            };
        }

        let x = inner.x + inner.width.saturating_sub(confirm_width) / 2;
        ButtonLayout {
            confirm: Some(Rect::new(x, y, confirm_width, 1)),
            cancel: None,
        }
    }

    fn confirm_button(&self) -> String {
        format!("[ {} ]", self.confirm_label)
    }

    fn button_line(
        &self,
        layout: &ButtonLayout,
        inner: Rect,
        theme: &Theme,
    ) -> Option<Line<'static>> {
        let confirm = layout.confirm?;
        let mut spans = vec![
            Span::raw(" ".repeat(confirm.x.saturating_sub(inner.x) as usize)),
            Span::styled(self.confirm_button(), self.confirm_button_style(theme)),
        ];
        if let Some(cancel) = layout.cancel {
            spans.push(Span::raw(
                " ".repeat(cancel.x.saturating_sub(confirm.right()) as usize),
            ));
            spans.push(Span::styled("[ Cancel ]", Style::default().fg(theme.muted)));
        }
        Some(Line::from(spans))
    }

    fn contains(rect: Rect, column: u16, row: u16) -> bool {
        column >= rect.x
            && column < rect.x.saturating_add(rect.width)
            && row >= rect.y
            && row < rect.y.saturating_add(rect.height)
    }
}

impl DialogContent for ConfirmDialog {
    fn desired_size(&self, viewport: Rect) -> (u16, u16) {
        (
            68u16.min(viewport.width.saturating_sub(4)),
            10u16.min(viewport.height.saturating_sub(4)),
        )
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let tone = self.tone_color(theme);
        let block = Block::default()
            .title(self.title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(tone));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let mut lines = vec![
            Line::from(Span::styled(
                self.message,
                Style::default().fg(theme.foreground),
            )),
            Line::from(""),
            labeled_line(
                "Target",
                &self.target_label,
                Style::default().fg(tone).add_modifier(Modifier::BOLD),
                inner.width,
                theme,
            ),
            labeled_line(
                "Effect",
                self.effect,
                theme.secondary_text_style(),
                inner.width,
                theme,
            ),
        ];

        let layout = self.button_layout(inner);
        if let Some(button_line) = self.button_line(&layout, inner, theme) {
            let button_row = layout
                .confirm
                .map(|rect| rect.y.saturating_sub(inner.y) as usize)
                .unwrap_or(lines.len());
            while lines.len() < button_row {
                lines.push(Line::from(""));
            }
            lines.push(button_line);
        }

        let hint = Line::from(Span::styled(
            format!("Enter/y {} - n/Esc cancel", self.confirm_verb),
            Style::default().fg(theme.muted),
        ))
        .centered();
        if lines.len() < inner.height as usize {
            lines.push(hint);
        }

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn handle_key(&mut self, key: KeyCode) -> DialogResult {
        // y/n are commands, not text, so remap them for non-Latin layouts.
        let key = crate::tui::keymap::normalize_hotkey(key);
        match key {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm();
                DialogResult::Close
            }
            KeyCode::Char('n') | KeyCode::Char('N') => DialogResult::Close,
            _ => DialogResult::None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> DialogResult {
        if !matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
            return DialogResult::None;
        }

        let layout = self.button_layout(Self::content_area(area));
        if layout
            .confirm
            .is_some_and(|rect| Self::contains(rect, event.column, event.row))
        {
            self.confirm();
            DialogResult::Close
        } else if layout
            .cancel
            .is_some_and(|rect| Self::contains(rect, event.column, event.row))
        {
            DialogResult::Close
        } else {
            DialogResult::None
        }
    }
}

fn labeled_line(
    label: &'static str,
    value: &str,
    value_style: Style,
    width: u16,
    theme: &Theme,
) -> Line<'static> {
    let width = width as usize;
    let label_text = format!("{label:<8}");
    let label_width = label_text.chars().count();
    if width <= label_width {
        return Line::from(Span::styled(
            truncate(&label_text, width),
            Style::default().fg(theme.muted),
        ));
    }

    Line::from(vec![
        Span::styled(label_text, Style::default().fg(theme.muted)),
        Span::styled(truncate(value, width - label_width), value_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::themes::ThemeName;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_dialog(width: u16, height: u16) -> String {
        let confirmed = Rc::new(RefCell::new(None));
        let dialog = ConfirmDialog::codex_switch(
            "acct_123".to_string(),
            "very-long-account-label".to_string(),
            confirmed,
        );
        let theme = Theme::from_name_for_current_terminal(ThemeName::Blue);
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dialog.render(frame, Rect::new(0, 0, width, height), &theme))
            .unwrap();

        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn narrow_confirm_dialog_does_not_render_orphan_ellipsis_for_zero_target_width() {
        let body = render_dialog(10, 8);

        assert!(body.contains("Target"), "{body}");
        assert!(!body.contains("..."), "{body}");
    }
}
