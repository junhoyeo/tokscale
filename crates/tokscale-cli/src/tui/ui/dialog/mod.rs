pub mod group_by_picker;
pub mod overlay;
pub mod source_picker;
pub mod stack;

use crossterm::event::{KeyCode, MouseEvent};
use ratatui::{layout::Rect, Frame};

use crate::tui::themes::Theme;

pub use group_by_picker::GroupByPickerDialog;
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
