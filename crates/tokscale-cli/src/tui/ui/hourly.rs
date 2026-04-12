use chrono::{Local, Timelike};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

use super::hourly_profile;
use super::widgets::{format_cache_hit_rate, format_cost, format_tokens};
use crate::tui::app::{App, HourlyViewMode, SortDirection, SortField};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.hourly_view_mode {
        HourlyViewMode::Table => render_table(frame, app, area),
        HourlyViewMode::Profile => hourly_profile::render(frame, app, area),
    }
}

fn render_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Hourly Usage ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.max_visible_items = visible_height;

    let hourly = app.get_sorted_hourly();
    if hourly.is_empty() {
        let empty_msg = Paragraph::new("No hourly usage data found. Press 'r' to refresh.")
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let has_turn_data = hourly.iter().any(|h| h.turn_count > 0);
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_selection = app.theme.selection;
    let now = Local::now().naive_local();
    let current_hour = now.date().and_hms_opt(now.hour(), 0, 0).unwrap_or(now);

    let header_cells = if is_very_narrow {
        vec!["Hour", "Cost"]
    } else if is_narrow {
        if has_turn_data {
            vec!["Hour", "Source", "Turn", "Msgs", "Tokens", "Cost"]
        } else {
            vec!["Hour", "Source", "Msgs", "Tokens", "Cost"]
        }
    } else if has_turn_data {
        vec![
            "Hour", "Source", "Turn", "Msgs", "Input", "Output", "Cache R", "Cache W", "Cache×",
            "Total", "Cost",
        ]
    } else {
        vec![
            "Hour", "Source", "Msgs", "Input", "Output", "Cache R", "Cache W", "Cache×", "Total",
            "Cost",
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
                    (9, false, false) if has_turn_data => sort_indicator(SortField::Tokens),
                    (8, false, false) if !has_turn_data => sort_indicator(SortField::Tokens),
                    (4, true, false) if has_turn_data => sort_indicator(SortField::Tokens),
                    (3, true, false) if !has_turn_data => sort_indicator(SortField::Tokens),
                    (10, false, false) if has_turn_data => sort_indicator(SortField::Cost),
                    (9, false, false) if !has_turn_data => sort_indicator(SortField::Cost),
                    (5, true, false) if has_turn_data => sort_indicator(SortField::Cost),
                    (4, true, false) if !has_turn_data => sort_indicator(SortField::Cost),
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

    let hourly_len = hourly.len();
    let start = scroll_offset.min(hourly_len);
    let end = (start + visible_height).min(hourly_len);

    if start >= hourly_len {
        return;
    }

    let rows: Vec<Row> = hourly[start..end]
        .iter()
        .enumerate()
        .map(|(i, hour)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;
            let is_current = hour.datetime == current_hour;

            let clients_str: String = {
                let mut c: Vec<&str> = hour.clients.iter().map(String::as_str).collect();
                c.sort();
                c.join(", ")
            };

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(hour.datetime.format("%m/%d %H:%M").to_string()).style(
                        if is_current {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ),
                    Cell::from(format_cost(hour.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                let mut cells = vec![
                    Cell::from(hour.datetime.format("%Y-%m-%d %H:%M").to_string()).style(
                        if is_current {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ),
                    Cell::from(clients_str),
                ];
                if has_turn_data {
                    let turn_str = if hour.turn_count > 0 {
                        hour.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(hour.message_count.to_string()),
                    Cell::from(format_tokens(hour.tokens.total())),
                    Cell::from(format_cost(hour.cost)).style(Style::default().fg(Color::Green)),
                ]);
                cells
            } else {
                let mut cells = vec![
                    Cell::from(hour.datetime.format("%Y-%m-%d %H:%M").to_string()).style(
                        if is_current {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().add_modifier(Modifier::BOLD)
                        },
                    ),
                    Cell::from(clients_str),
                ];
                if has_turn_data {
                    let turn_str = if hour.turn_count > 0 {
                        hour.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(hour.message_count.to_string()),
                    Cell::from(format_tokens(hour.tokens.input))
                        .style(Style::default().fg(Color::Rgb(100, 200, 100))),
                    Cell::from(format_tokens(hour.tokens.output))
                        .style(Style::default().fg(Color::Rgb(200, 100, 100))),
                    Cell::from(format_tokens(hour.tokens.cache_read))
                        .style(Style::default().fg(Color::Rgb(100, 150, 200))),
                    Cell::from(format_tokens(hour.tokens.cache_write))
                        .style(Style::default().fg(Color::Rgb(200, 150, 100))),
                    Cell::from(format_cache_hit_rate(
                        hour.tokens.cache_read,
                        hour.tokens.input,
                        hour.tokens.cache_write,
                    ))
                    .style(Style::default().fg(Color::Cyan)),
                    Cell::from(format_tokens(hour.tokens.total())),
                    Cell::from(format_cost(hour.cost)).style(Style::default().fg(Color::Green)),
                ]);
                cells
            };

            let row_style = if is_selected {
                Style::default().bg(theme_selection)
            } else if is_current {
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
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(12),
            Constraint::Percentage(13),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ]
    } else if is_narrow {
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(25),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ]
    } else if has_turn_data {
        vec![
            Constraint::Length(18),
            Constraint::Length(14),
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
            Constraint::Length(18),
            Constraint::Length(14),
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

    if hourly_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state = ScrollbarState::new(hourly_len).position(scroll_offset);

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
