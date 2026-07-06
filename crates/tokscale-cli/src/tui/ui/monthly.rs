use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table,
};

use super::widgets::{
    format_cache_hit_rate, format_cost, format_cost_per_million, format_tokens,
    viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.is_monthly_detail_active() {
        render_detail(frame, app, area);
        return;
    }

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

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

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
    let has_turn_data = monthly.iter().any(|m| m.turn_count > 0);
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_selection = app.theme.selection;
    let metric_input_style = app.theme.metric_input_style();
    let metric_output_style = app.theme.metric_output_style();
    let metric_cache_read_style = app.theme.metric_cache_read_style();
    let metric_cache_write_style = app.theme.metric_cache_write_style();
    let striped_row_style = app.theme.striped_row_style();

    let full_layout_width: u16 = if has_turn_data { 112 } else { 105 };
    let compact_full_date = !is_narrow && !is_very_narrow && inner.width < full_layout_width;
    let month_col_width: u16 = if compact_full_date { 7 } else { 12 };

    let header_cells = if is_very_narrow {
        vec!["Month", "Cost"]
    } else if is_narrow {
        if has_turn_data {
            vec!["Month", "Turn", "Msgs", "Tokens", "Cost"]
        } else {
            vec!["Month", "Msgs", "Tokens", "Cost"]
        }
    } else if has_turn_data {
        vec![
            "Month", "Turn", "Msgs", "Input", "Output", "Cache R", "Cache W", "Cache×", "Total",
            "Cost", "Cost/1M",
        ]
    } else {
        vec![
            "Month", "Msgs", "Input", "Output", "Cache R", "Cache W", "Cache×", "Total", "Cost",
            "Cost/1M",
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

    let monthly_len = monthly.len();
    let start = scroll_offset.min(monthly_len);
    let end = (start + visible_height).min(monthly_len);

    if start >= monthly_len {
        return;
    }

    let rows: Vec<Row> = monthly[start..end]
        .iter()
        .enumerate()
        .map(|(i, month)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(month.month.clone()),
                    Cell::from(format_cost(month.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                let mut cells = vec![Cell::from(month.month.clone())];
                if has_turn_data {
                    let turn_str = if month.turn_count > 0 {
                        month.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(month.message_count.to_string()),
                    Cell::from(format_tokens(month.tokens.total())),
                    Cell::from(format_cost(month.cost)).style(Style::default().fg(Color::Green)),
                ]);
                cells
            } else {
                let mut cells = vec![Cell::from(month.month.clone())];
                if has_turn_data {
                    let turn_str = if month.turn_count > 0 {
                        month.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(month.message_count.to_string()),
                    Cell::from(format_tokens(month.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(month.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(month.tokens.cache_read))
                        .style(metric_cache_read_style),
                    Cell::from(format_tokens(month.tokens.cache_write))
                        .style(metric_cache_write_style),
                    Cell::from(format_cache_hit_rate(
                        month.tokens.cache_read,
                        month.tokens.input,
                        month.tokens.cache_write,
                    ))
                    .style(Style::default().fg(Color::Cyan)),
                    Cell::from(format_tokens(month.tokens.total())),
                    Cell::from(format_cost(month.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(format_cost_per_million(month.cost, month.tokens.total()))
                        .style(Style::default().fg(Color::Rgb(150, 200, 150))),
                ]);
                cells
            };

            let row_style = if is_selected {
                Style::default().bg(theme_selection)
            } else if is_striped {
                striped_row_style
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
            Constraint::Length(month_col_width),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    } else {
        vec![
            Constraint::Length(month_col_width),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if monthly_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state =
            viewport_scrollbar_state(monthly_len, scroll_offset, visible_height);

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

fn render_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = app
        .monthly_detail_month()
        .map(|month| format!(" Daily Breakdown: {} ", month))
        .unwrap_or_else(|| " Daily Breakdown ".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            title,
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let days = app.get_sorted_monthly_detail_days();
    if days.is_empty() {
        let empty_msg = Paragraph::new("No daily data found for this month. Press Esc to go back.")
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let has_turn_data = days.iter().any(|d| d.turn_count > 0);
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_selection = app.theme.selection;
    let metric_input_style = app.theme.metric_input_style();
    let metric_output_style = app.theme.metric_output_style();
    let metric_cache_read_style = app.theme.metric_cache_read_style();
    let metric_cache_write_style = app.theme.metric_cache_write_style();
    let striped_row_style = app.theme.striped_row_style();

    let full_layout_width: u16 = if has_turn_data { 112 } else { 105 };
    let compact_full_date = !is_narrow && !is_very_narrow && inner.width < full_layout_width;
    let date_col_width: u16 = if compact_full_date { 7 } else { 12 };
    let date_fmt: &str = if is_very_narrow {
        "%m/%d"
    } else if is_narrow || compact_full_date {
        "%m-%d"
    } else {
        "%Y-%m-%d"
    };

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
            "Cost", "Cost/1M",
        ]
    } else {
        vec![
            "Date", "Msgs", "Input", "Output", "Cache R", "Cache W", "Cache×", "Total", "Cost",
            "Cost/1M",
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

    let days_len = days.len();
    let start = scroll_offset.min(days_len);
    let end = (start + visible_height).min(days_len);

    if start >= days_len {
        return;
    }

    let rows: Vec<Row> = days[start..end]
        .iter()
        .enumerate()
        .map(|(i, day)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(day.date.format(date_fmt).to_string()),
                    Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                let mut cells = vec![Cell::from(day.date.format(date_fmt).to_string())];
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
                let mut cells = vec![Cell::from(day.date.format(date_fmt).to_string())];
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
                    Cell::from(format_tokens(day.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(day.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(day.tokens.cache_read)).style(metric_cache_read_style),
                    Cell::from(format_tokens(day.tokens.cache_write))
                        .style(metric_cache_write_style),
                    Cell::from(format_cache_hit_rate(
                        day.tokens.cache_read,
                        day.tokens.input,
                        day.tokens.cache_write,
                    ))
                    .style(Style::default().fg(Color::Cyan)),
                    Cell::from(format_tokens(day.tokens.total())),
                    Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(format_cost_per_million(day.cost, day.tokens.total()))
                        .style(Style::default().fg(Color::Rgb(150, 200, 150))),
                ]);
                cells
            };

            let row_style = if is_selected {
                Style::default().bg(theme_selection)
            } else if is_striped {
                striped_row_style
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
            Constraint::Length(date_col_width),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    } else {
        vec![
            Constraint::Length(date_col_width),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if days_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state = viewport_scrollbar_state(days_len, scroll_offset, visible_height);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{Tab, TuiConfig};
    use crate::tui::data::{DailyUsage, MonthlyUsage, TokenBreakdown};
    use chrono::NaiveDate;
    use ratatui::{backend::TestBackend, Terminal};
    use std::collections::BTreeMap;

    fn month(month: &str, input: u64, cost: f64) -> MonthlyUsage {
        MonthlyUsage {
            month: month.to_string(),
            tokens: TokenBreakdown {
                input,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            message_count: 1,
            turn_count: 0,
        }
    }

    fn day(date: &str, input: u64, cost: f64) -> DailyUsage {
        DailyUsage {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            tokens: TokenBreakdown {
                input,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            source_breakdown: BTreeMap::new(),
            message_count: 1,
            turn_count: 0,
        }
    }

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
        app.current_tab = Tab::Monthly;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app
    }

    fn render_body(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, app, Rect::new(0, 0, width, height)))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| {
                row.iter()
                    .map(|c| c.symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn wide_terminal_renders_full_monthly_columns() {
        let mut app = make_app(130);
        app.data.monthly = vec![month("2026-05", 1000, 1.5)];
        let body = render_body(&mut app, 130, 12);
        assert!(
            body.contains("Cache×"),
            "expected cache hit rate column\n{body}"
        );
        assert!(
            body.contains("Cost/1M"),
            "expected cost per million column\n{body}"
        );
        assert!(body.contains("2026-05"), "expected month row\n{body}");
    }

    #[test]
    fn monthly_detail_renders_daily_breakdown_title() {
        let mut app = make_app(130);
        app.data.monthly = vec![month("2026-05", 1000, 1.5)];
        app.data.daily = vec![day("2026-05-10", 500, 0.75), day("2026-04-05", 200, 0.25)];
        app.selected_monthly_detail_month = Some("2026-05".to_string());

        let body = render_body(&mut app, 130, 12);
        assert!(
            body.contains("Daily Breakdown: 2026-05"),
            "expected detail title\n{body}"
        );
        assert!(body.contains("2026-05-10"), "expected daily row\n{body}");
        assert!(
            !body.contains("2026-04-05"),
            "should not show other months\n{body}"
        );
    }
}
