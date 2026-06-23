use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Clear},
    Frame,
};

use crate::tui::themes::Theme;

/// Calculate a centered rectangle within the viewport
pub fn centered_rect(viewport: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(viewport.width.saturating_sub(2));
    let height = height.min(viewport.height.saturating_sub(2));
    let x = viewport.x + viewport.width.saturating_sub(width) / 2;
    let y = viewport.y + viewport.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

/// Render a semi-transparent dark backdrop over the viewport
pub fn render_backdrop(frame: &mut Frame, viewport: Rect, theme: &Theme) {
    // Clear the area first
    frame.render_widget(Clear, viewport);
    // Render dark backdrop
    let backdrop = Block::default().style(Style::default().bg(theme.background));
    frame.render_widget(backdrop, viewport);
}

/// Render the dialog surface (panel background)
pub fn render_dialog_surface(frame: &mut Frame, area: Rect, theme: &Theme) {
    frame.render_widget(Clear, area);
    let panel = Block::default().style(Style::default().bg(theme.background).fg(theme.foreground));
    frame.render_widget(panel, area);
}
