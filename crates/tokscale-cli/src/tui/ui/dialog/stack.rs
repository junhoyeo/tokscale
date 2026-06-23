use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{layout::Rect, Frame};

use crate::tui::themes::Theme;

use super::{overlay, DialogContent, DialogResult};

/// Manages a stack of dialogs, rendering them as overlays
pub struct DialogStack {
    stack: Vec<Box<dyn DialogContent>>,
    theme: Theme,
    /// The rectangle of the topmost dialog (for mouse click detection)
    top_rect: Option<Rect>,
}

impl DialogStack {
    pub fn new(theme: Theme) -> Self {
        Self {
            stack: Vec::new(),
            theme,
            top_rect: None,
        }
    }

    /// Update the theme
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Push a new dialog onto the stack
    pub fn show(&mut self, dialog: Box<dyn DialogContent>) {
        self.stack.push(dialog);
    }

    /// Replace all dialogs with a new one
    #[allow(dead_code)]
    pub fn replace(&mut self, dialog: Box<dyn DialogContent>) {
        self.stack.clear();
        self.stack.push(dialog);
    }

    /// Close the topmost dialog
    pub fn close(&mut self) {
        self.stack.pop();
        if self.stack.is_empty() {
            self.top_rect = None;
        }
    }

    /// Clear all dialogs
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.stack.clear();
        self.top_rect = None;
    }

    /// Check if any dialog is open
    pub fn is_active(&self) -> bool {
        !self.stack.is_empty()
    }

    /// Get the number of dialogs in the stack
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// Render the dialog stack (backdrop + topmost dialog)
    pub fn render(&mut self, frame: &mut Frame, viewport: Rect) {
        let Some(top) = self.stack.last() else {
            self.top_rect = None;
            return;
        };

        // Render backdrop
        overlay::render_backdrop(frame, viewport, &self.theme);

        // Calculate centered position
        let (width, height) = top.desired_size(viewport);
        let area = overlay::centered_rect(viewport, width, height);
        self.top_rect = Some(area);

        // Render dialog surface and content
        overlay::render_dialog_surface(frame, area, &self.theme);
        top.render(frame, area, &self.theme);
    }

    /// Handle a key event, returns true if the event was consumed
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        if self.stack.is_empty() {
            return false;
        }

        // Esc always closes the dialog
        if key == KeyCode::Esc {
            self.close();
            return true;
        }

        if let Some(top) = self.stack.last_mut() {
            let result = top.handle_key(key);
            self.apply_result(result);
            return true;
        }

        false
    }

    /// Handle a mouse event, returns true if the event was consumed
    pub fn handle_mouse(&mut self, event: MouseEvent) -> bool {
        if self.stack.is_empty() {
            return false;
        }

        // Click outside dialog closes it
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
            if let Some(rect) = self.top_rect {
                let inside = event.column >= rect.x
                    && event.column < rect.x + rect.width
                    && event.row >= rect.y
                    && event.row < rect.y + rect.height;
                if !inside {
                    self.close();
                    return true;
                }
            }
        }

        // Route event to topmost dialog
        if let (Some(top), Some(rect)) = (self.stack.last_mut(), self.top_rect) {
            let result = top.handle_mouse(event, rect);
            self.apply_result(result);
        }

        true
    }

    fn apply_result(&mut self, result: DialogResult) {
        match result {
            DialogResult::None => {}
            DialogResult::Close => self.close(),
            DialogResult::Replace(next) => {
                self.stack.clear();
                self.stack.push(next);
            }
        }
    }
}
