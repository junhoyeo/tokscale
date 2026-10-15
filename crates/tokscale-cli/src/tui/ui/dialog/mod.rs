pub mod confirm;
pub mod group_by_picker;
pub mod language_picker;
pub mod overlay;
pub mod session_detail;
pub mod source_picker;
pub mod stack;

use crossterm::event::{KeyCode, MouseEvent};
use ratatui::{layout::Rect, Frame};

use crate::tui::themes::Theme;

pub use confirm::ConfirmDialog;
pub use group_by_picker::GroupByPickerDialog;
pub use language_picker::LanguagePickerDialog;
pub use session_detail::SessionDetailDialog;
pub use source_picker::ClientPickerDialog;
pub use stack::DialogStack;

/// Result of handling a dialog event
pub enum DialogResult {
    /// No action, continue showing dialog
    None,
    /// Close the current dialog
    Close,
    /// Replace the current dialog with a new one
    #[allow(dead_code)]
    Replace(Box<dyn DialogContent>),
}

/// Trait for dialog content that can be rendered and handle events
pub trait DialogContent {
    /// Return the desired (width, height) for the dialog
    fn desired_size(&self, viewport: Rect) -> (u16, u16);

    /// Render the dialog content within the given area
    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme);

    /// Handle a key event, return the result
    fn handle_key(&mut self, _key: KeyCode) -> DialogResult {
        DialogResult::None
    }

    /// Handle a mouse event, return the result
    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect) -> DialogResult {
        DialogResult::None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    use ratatui::{backend::TestBackend, Terminal};
    use tokscale_core::GroupBy;
    use unicode_width::UnicodeWidthStr;

    use super::*;
    use crate::tui::i18n::TuiLanguage;
    use crate::tui::themes::{Theme, ThemeName};
    use crate::ClientFilter;

    fn assert_chrome_rows_are_width_stable(name: &str, dialog: &dyn DialogContent) {
        let width = 60;
        let height = 20;
        let theme = Theme::from_name_for_current_terminal(ThemeName::Blue);
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dialog.render(frame, frame.area(), &theme))
            .unwrap();

        let mut checked_rows = 0;
        for row in terminal.backend().buffer().content().chunks(width as usize) {
            let rendered = row.iter().map(|cell| cell.symbol()).collect::<String>();
            if rendered.chars().any(|ch| "●↑↓•*▴▾◂▸∙".contains(ch)) {
                checked_rows += 1;
                assert_eq!(
                    UnicodeWidthStr::width(rendered.as_str()),
                    UnicodeWidthStr::width_cjk(rendered.as_str()),
                    "{name} chrome width changed in a CJK locale: {rendered:?}"
                );
            }
        }
        assert!(
            checked_rows >= 2,
            "{name} did not render its marker and hint"
        );
    }

    #[test]
    fn picker_chrome_has_the_same_width_in_cjk_locales() {
        for lang in TuiLanguage::ALL {
            let language = LanguagePickerDialog::new(
                Rc::new(RefCell::new(lang)),
                Rc::new(RefCell::new(false)),
            );
            assert_chrome_rows_are_width_stable(
                &format!("language picker ({})", lang.code()),
                &language,
            );

            let group_by = GroupByPickerDialog::new(
                Rc::new(RefCell::new(GroupBy::ClientModel)),
                Rc::new(RefCell::new(false)),
                lang,
            );
            assert_chrome_rows_are_width_stable(
                &format!("group-by picker ({})", lang.code()),
                &group_by,
            );

            let clients = ClientPickerDialog::new(
                Rc::new(RefCell::new(HashSet::from([ClientFilter::Claude]))),
                Rc::new(RefCell::new(false)),
                lang,
            );
            assert_chrome_rows_are_width_stable(
                &format!("client picker ({})", lang.code()),
                &clients,
            );
        }
    }
}
