use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Tabs};

use crate::tui::app::{App, ClickAction, Tab};

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
            let name = if is_very_narrow {
                t.short_name()
            } else {
                t.as_str()
            };
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
    let is_very_narrow = app.is_very_narrow();
    let inner_x = area.x + 12;
    let y = area.y + 1;
    let mut x = inner_x;

    let visible_tabs: Vec<Tab> = Tab::all()
        .iter()
        .copied()
        .filter(|t| app.is_tab_visible(*t))
        .collect();

    for tab in visible_tabs {
        let name_len = if is_very_narrow {
            tab.short_name().len()
        } else {
            tab.as_str().len()
        };
        let width = name_len as u16 + 3;
        app.add_click_area(Rect::new(x, y, width, 1), ClickAction::Tab(tab));
        x += width;
    }
}
