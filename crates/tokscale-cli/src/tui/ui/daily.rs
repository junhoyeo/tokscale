use chrono::Local;
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

use super::widgets::{format_cache_hit_rate, format_cost, format_tokens};
use crate::tui::app::{App, SortDirection, SortField};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Daily Usage ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.max_visible_items = visible_height;

    let daily = app.get_sorted_daily();
    if daily.is_empty() {
        let empty_msg = Paragraph::new("No daily usage data found. Press 'r' to refresh.")
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let has_turn_data = daily.iter().any(|d| d.turn_count > 0);
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_selection = app.theme.selection;
    let today = Local::now().date_naive();

    let header_cells = if is_very_narrow {
        vec!["Date", "Cost"]
    } else if is_narrow {
        if has_turn_data {
            vec!["Date", "Turn", "Msgs", "Tokens", "Cost"]
        } else {
            vec!["Date", "Msgs", "Tokens", "Cost"]
        }
    } else if has_turn_data {
        vec![
            "Date", "Turn", "Msgs", "Input", "Output", "Cache R", "Cache W", "Cache×", "Total",
            "Cost",
        ]
    } else {
        vec![
            "Date", "Msgs", "Input", "Output", "Cache R", "Cache W", "Cache×", "Total", "Cost",
        ]
    };

    let sort_indicator = |field: SortField| -> &'static str {
        if sort_field == field {
            match sort_direction {
                SortDirection::Ascending => " ▲",
                SortDirection::Descending => " ▼",
            }
        } else {
            ""
        }
    };

    let header = Row::new(
        header_cells
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let indicator = match (i, is_narrow, is_very_narrow) {
                    (0, _, _) => sort_indicator(SortField::Date),
                    (8, false, false) if has_turn_data => sort_indicator(SortField::Tokens),
                    (7, false, false) if !has_turn_data => sort_indicator(SortField::Tokens),
                    (3, true, false) if has_turn_data => sort_indicator(SortField::Tokens),
                    (2, true, false) if !has_turn_data => sort_indicator(SortField::Tokens),
                    (9, false, false) if has_turn_data => sort_indicator(SortField::Cost),
                    (8, false, false) if !has_turn_data => sort_indicator(SortField::Cost),
                    (4, true, false) if has_turn_data => sort_indicator(SortField::Cost),
                    (3, true, false) if !has_turn_data => sort_indicator(SortField::Cost),
                    (1, _, true) => sort_indicator(SortField::Cost),
                    _ => "",
                };
                Cell::from(format!("{}{}", h, indicator))
            })
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(theme_accent)
            .add_modifier(Modifier::BOLD),
    )
    .height(1);

    let daily_len = daily.len();
    let start = scroll_offset.min(daily_len);
    let end = (start + visible_height).min(daily_len);

    if start >= daily_len {
        return;
    }

    let rows: Vec<Row> = daily[start..end]
        .iter()
        .enumerate()
        .map(|(i, day)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;
            let is_today = day.date == today;

            let cells: Vec<Cell> =
                if is_very_narrow {
                    vec![
                        Cell::from(day.date.format("%m/%d").to_string()).style(if is_today {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        }),
                        Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green)),
                    ]
                } else if is_narrow {
                    let mut cells = vec![Cell::from(day.date.format("%Y-%m-%d").to_string())
                        .style(if is_today {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        })];
                    if has_turn_data {
                        let turn_str = if day.turn_count > 0 {
                            day.turn_count.to_string()
                        } else {
                            "\u{2014}".to_string()
                        };
                        cells.push(Cell::from(turn_str));
                    }
                    cells.extend([
                        Cell::from(day.message_count.to_string()),
                        Cell::from(format_tokens(day.tokens.total())),
                        Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green)),
                    ]);
                    cells
                } else {
                    let mut cells = vec![Cell::from(day.date.format("%Y-%m-%d").to_string())
                        .style(if is_today {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().add_modifier(Modifier::BOLD)
                        })];
                    if has_turn_data {
                        let turn_str = if day.turn_count > 0 {
                            day.turn_count.to_string()
                        } else {
                            "\u{2014}".to_string()
                        };
                        cells.push(Cell::from(turn_str));
                    }
                    cells.extend([
                        Cell::from(day.message_count.to_string()),
                        Cell::from(format_tokens(day.tokens.input))
                            .style(Style::default().fg(Color::Rgb(100, 200, 100))),
                        Cell::from(format_tokens(day.tokens.output))
                            .style(Style::default().fg(Color::Rgb(200, 100, 100))),
                        Cell::from(format_tokens(day.tokens.cache_read))
                            .style(Style::default().fg(Color::Rgb(100, 150, 200))),
                        Cell::from(format_tokens(day.tokens.cache_write))
                            .style(Style::default().fg(Color::Rgb(200, 150, 100))),
                        Cell::from(format_cache_hit_rate(
                            day.tokens.cache_read,
                            day.tokens.input,
                            day.tokens.cache_write,
                        ))
                        .style(Style::default().fg(Color::Cyan)),
                        Cell::from(format_tokens(day.tokens.total())),
                        Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green)),
                    ]);
                    cells
                };

            let row_style = if is_selected {
                Style::default().bg(theme_selection)
            } else if is_today {
                Style::default().bg(Color::Rgb(28, 42, 34))
            } else if is_striped {
                Style::default().bg(Color::Rgb(20, 24, 30))
            } else {
                Style::default()
            };

            Row::new(cells).style(row_style).height(1)
        })
        .collect();

    let widths = if is_very_narrow {
        vec![Constraint::Percentage(60), Constraint::Percentage(40)]
    } else if is_narrow && has_turn_data {
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ]
    } else if is_narrow {
        vec![
            Constraint::Percentage(35),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
        ]
    } else if has_turn_data {
        vec![
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    } else {
        vec![
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if daily_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state = ScrollbarState::new(daily_len).position(scroll_offset);

        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}
