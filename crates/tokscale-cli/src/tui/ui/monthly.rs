use chrono::Local;
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

use super::widgets::{
    format_cost, format_tokens, get_client_display_name, get_model_color, get_provider_display_name,
};
use crate::tui::app::{App, SortDirection, SortField};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Monthly Usage ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let average_visible = app.data.total_months > 0;
    let areas = if average_visible {
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner)
    } else {
        Layout::vertical([Constraint::Min(0)]).split(inner)
    };

    let table_area = areas[0];
    let visible_height = table_area.height.saturating_sub(1) as usize;
    app.max_visible_items = visible_height;

    let monthly = app.get_sorted_monthly();
    if monthly.is_empty() {
        let empty_msg = Paragraph::new("No monthly usage data found. Press 'r' to refresh.")
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_muted = app.theme.muted;
    let theme_selection = app.theme.selection;
    let current_month = Local::now().format("%Y-%m").to_string();

    let header_cells = if is_very_narrow {
        vec!["Month", "Model", "Cost"]
    } else if is_narrow {
        vec!["Month", "Model", "Tokens", "Cost"]
    } else {
        vec![
            "Month", "Model", "Provider", "Client", "Input", "Output", "Cache", "Total", "Cost",
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
                    (2, _, true) => sort_indicator(SortField::Cost),
                    (2, true, false) => sort_indicator(SortField::Tokens),
                    (3, true, false) => sort_indicator(SortField::Cost),
                    (7, false, false) => sort_indicator(SortField::Tokens),
                    (8, false, false) => sort_indicator(SortField::Cost),
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

    let monthly_len = monthly.len();
    let start = scroll_offset.min(monthly_len.saturating_sub(1));
    let end = (start + visible_height).min(monthly_len);

    if start >= monthly_len {
        return;
    }

    let rows: Vec<Row> = monthly[start..end]
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;
            let is_current_month = entry.month == current_month;
            let month_style = if is_current_month {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            let model_style = Style::default()
                .fg(get_model_color(&entry.model))
                .add_modifier(Modifier::BOLD);

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(entry.month.clone()).style(month_style),
                    Cell::from(truncate(&entry.model, 18)).style(model_style),
                    Cell::from(format_cost(entry.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(entry.month.clone()).style(month_style),
                    Cell::from(truncate(&entry.model, 20)).style(model_style),
                    Cell::from(format_tokens(entry.tokens.total())),
                    Cell::from(format_cost(entry.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                vec![
                    Cell::from(entry.month.clone()).style(month_style),
                    Cell::from(truncate(&entry.model, 20)).style(model_style),
                    Cell::from(get_provider_display_name(&entry.provider)),
                    Cell::from(get_client_display_name(&entry.client))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(format_tokens(entry.tokens.input))
                        .style(Style::default().fg(Color::Rgb(100, 200, 100))),
                    Cell::from(format_tokens(entry.tokens.output))
                        .style(Style::default().fg(Color::Rgb(200, 100, 100))),
                    Cell::from(format_tokens(
                        entry.tokens.cache_read + entry.tokens.cache_write,
                    ))
                    .style(Style::default().fg(Color::Rgb(100, 150, 200))),
                    Cell::from(format_tokens(entry.tokens.total())),
                    Cell::from(format_cost(entry.cost)).style(Style::default().fg(Color::Green)),
                ]
            };

            let row_style = if is_selected {
                Style::default().bg(theme_selection)
            } else if is_current_month {
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
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(40),
            Constraint::Percentage(30),
        ]
    } else if is_narrow {
        vec![
            Constraint::Percentage(20),
            Constraint::Percentage(35),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ]
    } else {
        vec![
            Constraint::Length(8),
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(9),
        ]
    };

    let table = Table::new(rows, widths.clone())
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, table_area);

    if average_visible {
        let average_tokens = app.data.total_tokens / app.data.total_months as u64;
        let average_cost = app.data.total_cost / app.data.total_months as f64;
        let average_style = Style::default()
            .fg(Color::Cyan)
            .bg(Color::Rgb(25, 35, 45))
            .add_modifier(Modifier::BOLD);

        let average_cells = if is_very_narrow {
            vec![
                Cell::from("AVG/MO"),
                Cell::from(""),
                Cell::from(format_cost(average_cost)),
            ]
        } else if is_narrow {
            vec![
                Cell::from("AVG/MO"),
                Cell::from(""),
                Cell::from(format_tokens(average_tokens)),
                Cell::from(format_cost(average_cost)),
            ]
        } else {
            vec![
                Cell::from("AVG/MO"),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(format_tokens(average_tokens)),
                Cell::from(format_cost(average_cost)),
            ]
        };

        let average_table = Table::new(vec![Row::new(average_cells).style(average_style)], widths);
        frame.render_widget(average_table, areas[1]);
    }

    if monthly_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state = ScrollbarState::new(monthly_len).position(scroll_offset);

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

fn truncate(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else if max_chars <= 3 {
        s.chars().take(max_chars).collect()
    } else {
        let head: String = s.chars().take(max_chars - 3).collect();
        format!("{}...", head)
    }
}
