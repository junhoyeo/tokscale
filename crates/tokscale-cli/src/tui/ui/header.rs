use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Tabs};

use crate::tui::app::{App, ClickAction, Tab};

const TAB_PADDING_LEFT_WIDTH: u16 = 1;
const TAB_PADDING_RIGHT_WIDTH: u16 = 1;
const TAB_DIVIDER_WIDTH: u16 = 3;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let is_very_narrow = app.is_very_narrow();

    let visible_tabs: Vec<Tab> = Tab::all()
        .iter()
        .copied()
        .filter(|t| app.is_tab_visible(*t))
        .collect();

    let titles: Vec<Line> = visible_tabs
        .iter()
        .map(|t| {
            let name = tab_label(*t, is_very_narrow);
            let style = if *t == app.current_tab {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.muted)
            };
            Line::from(Span::styled(name, style))
        })
        .collect();

    let selected = visible_tabs
        .iter()
        .position(|t| *t == app.current_tab)
        .unwrap_or(0);

    let is_narrow = app.is_narrow();

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " tokscale ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Left)
        .style(Style::default().bg(app.theme.background));

    if !is_narrow {
        block = block.title_top(
            Line::from(vec![
                Span::styled(" | ", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled("GitHub ", Style::default().fg(Color::Rgb(102, 102, 102))),
            ])
            .right_aligned(),
        );
    }

    let tabs = Tabs::new(titles)
        .block(block)
        .select(selected)
        .highlight_style(
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled(" │ ", Style::default().fg(app.theme.border)));

    frame.render_widget(tabs, area);

    register_tab_click_areas(app, area);
}

fn register_tab_click_areas(app: &mut App, area: Rect) {
    if area.width <= 2 || area.height <= 2 {
        return;
    }

    let is_very_narrow = app.is_very_narrow();
    let y = area.y.saturating_add(1);
    let right = area.right().saturating_sub(1);
    let mut x = area.x.saturating_add(1);

    let visible_tabs: Vec<Tab> = Tab::all()
        .iter()
        .copied()
        .filter(|t| app.is_tab_visible(*t))
        .collect();
    let tab_count = visible_tabs.len();

    for (index, tab) in visible_tabs.into_iter().enumerate() {
        let remaining_width = right.saturating_sub(x);
        if remaining_width == 0 {
            break;
        }

        let left_padding_width = TAB_PADDING_LEFT_WIDTH.min(remaining_width);
        let remaining_width = remaining_width.saturating_sub(left_padding_width);
        let title_width = tab_label_width(tab, is_very_narrow).min(remaining_width);
        let remaining_width = remaining_width.saturating_sub(title_width);
        let right_padding_width = TAB_PADDING_RIGHT_WIDTH.min(remaining_width);
        let click_width = left_padding_width + title_width + right_padding_width;

        app.add_click_area(Rect::new(x, y, click_width, 1), ClickAction::Tab(tab));
        x = x.saturating_add(click_width);

        let remaining_width = right.saturating_sub(x);
        if remaining_width == 0 || index + 1 == tab_count {
            break;
        }

        x = x.saturating_add(TAB_DIVIDER_WIDTH.min(remaining_width));
    }
}

fn tab_label(tab: Tab, is_very_narrow: bool) -> &'static str {
    if is_very_narrow {
        tab.short_name()
    } else {
        tab.as_str()
    }
}

fn tab_label_width(tab: Tab, is_very_narrow: bool) -> u16 {
    Line::from(tab_label(tab, is_very_narrow)).width() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};

    use crate::tui::app::TuiConfig;

    fn make_app(width: u16) -> App {
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
        app.terminal_width = width;
        app
    }

    fn render_header(app: &mut App, width: u16) {
        let backend = TestBackend::new(width, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, app, Rect::new(0, 0, width, 3)))
            .unwrap();
    }

    fn click_header(app: &mut App, column: u16) {
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn daily_label_click_uses_rendered_normal_tab_position() {
        let mut app = make_app(80);

        render_header(&mut app, 80);
        click_header(&mut app, 26);

        assert_eq!(app.current_tab, Tab::Daily);
    }

    #[test]
    fn daily_short_label_click_uses_rendered_very_narrow_tab_position() {
        let mut app = make_app(59);

        render_header(&mut app, 59);
        click_header(&mut app, 18);

        assert_eq!(app.current_tab, Tab::Daily);
    }
}
