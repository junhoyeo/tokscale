use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::widgets::{
    ambient_stable_scrollbar, format_cache_hit_rate, format_cost, format_cost_per_million,
    format_tokens, total_tokens_cell, viewport_scrollbar_state, AMBIENT_STABLE_BORDER_SET,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

/// The Monthly table's header labels for a layout, in display order.
///
/// A function rather than an inline `vec!` so
/// `every_language_renders_its_full_header_once_the_layout_fits` asserts against
/// the labels the renderer actually writes. A test that restates the label set
/// passes while the renderer uses a different one, which is how the Korean
/// `메시지` clip in `sessions.rs` survived a header test that already existed.
fn header_labels(
    lang: TuiLanguage,
    is_narrow: bool,
    is_very_narrow: bool,
    has_turn_data: bool,
) -> Vec<&'static str> {
    if is_very_narrow {
        return vec![
            tr(lang, MessageKey::ColMonth),
            tr(lang, MessageKey::ColCost),
        ];
    }
    if is_narrow {
        return if has_turn_data {
            vec![
                tr(lang, MessageKey::ColMonth),
                tr(lang, MessageKey::ColTurn),
                tr(lang, MessageKey::ColMessages),
                tr(lang, MessageKey::ColTokens),
                tr(lang, MessageKey::ColCost),
            ]
        } else {
            vec![
                tr(lang, MessageKey::ColMonth),
                tr(lang, MessageKey::ColMessages),
                tr(lang, MessageKey::ColTokens),
                tr(lang, MessageKey::ColCost),
            ]
        };
    }
    let mut labels = vec![tr(lang, MessageKey::ColMonth)];
    if has_turn_data {
        labels.push(tr(lang, MessageKey::ColTurn));
    }
    labels.extend([
        tr(lang, MessageKey::ColMessages),
        tr(lang, MessageKey::ColInput),
        tr(lang, MessageKey::ColOutput),
        tr(lang, MessageKey::ColCacheRead),
        tr(lang, MessageKey::ColCacheWrite),
        tr(lang, MessageKey::ColCacheHit),
        tr(lang, MessageKey::ColTotal),
        tr(lang, MessageKey::ColCost),
        tr(lang, MessageKey::ColCostPer1M),
    ]);
    labels
}

/// The Monthly table's column widths, index-aligned with [`header_labels`].
///
/// A function rather than an inline `vec!` so
/// `no_header_overflows_its_budget_in_any_language` checks the widths the
/// renderer actually lays out with, not a copy of them. The narrow layouts are
/// percentage-based and have no declared budget; the wide ones are `Length`, so
/// a header longer than its `Length` is a clip the arithmetic can name.
fn header_widths(
    month_col_width: u16,
    is_narrow: bool,
    is_very_narrow: bool,
    has_turn_data: bool,
) -> Vec<Constraint> {
    if is_very_narrow {
        return vec![Constraint::Percentage(60), Constraint::Percentage(40)];
    }
    if is_narrow {
        return if has_turn_data {
            vec![
                Constraint::Percentage(30),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
            ]
        } else {
            vec![
                Constraint::Percentage(35),
                Constraint::Percentage(20),
                Constraint::Percentage(25),
                Constraint::Percentage(20),
            ]
        };
    }
    let mut widths = vec![Constraint::Length(month_col_width)];
    if has_turn_data {
        widths.push(Constraint::Length(6));
    }
    widths.extend([
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
    ]);
    widths
}

/// The monthly *detail* (days-in-month) table's header labels, in display order.
/// Same rationale as [`header_labels`]; the only difference from the monthly
/// table is a Date column where that one has Month.
fn detail_header_labels(
    lang: TuiLanguage,
    is_narrow: bool,
    is_very_narrow: bool,
    has_turn_data: bool,
) -> Vec<&'static str> {
    if is_very_narrow {
        return vec![tr(lang, MessageKey::ColDate), tr(lang, MessageKey::ColCost)];
    }
    if is_narrow {
        return if has_turn_data {
            vec![
                tr(lang, MessageKey::ColDate),
                tr(lang, MessageKey::ColTurn),
                tr(lang, MessageKey::ColMessages),
                tr(lang, MessageKey::ColTokens),
                tr(lang, MessageKey::ColCost),
            ]
        } else {
            vec![
                tr(lang, MessageKey::ColDate),
                tr(lang, MessageKey::ColMessages),
                tr(lang, MessageKey::ColTokens),
                tr(lang, MessageKey::ColCost),
            ]
        };
    }
    let mut labels = vec![tr(lang, MessageKey::ColDate)];
    if has_turn_data {
        labels.push(tr(lang, MessageKey::ColTurn));
    }
    labels.extend([
        tr(lang, MessageKey::ColMessages),
        tr(lang, MessageKey::ColInput),
        tr(lang, MessageKey::ColOutput),
        tr(lang, MessageKey::ColCacheRead),
        tr(lang, MessageKey::ColCacheWrite),
        tr(lang, MessageKey::ColCacheHit),
        tr(lang, MessageKey::ColTotal),
        tr(lang, MessageKey::ColCost),
        tr(lang, MessageKey::ColCostPer1M),
    ]);
    labels
}

/// The monthly *detail* table's column widths, index-aligned with
/// [`detail_header_labels`]. Same rationale as [`header_widths`], and the same
/// numbers — the detail table narrows its Date column exactly as the monthly
/// table narrows its Month column.
fn detail_header_widths(
    date_col_width: u16,
    is_narrow: bool,
    is_very_narrow: bool,
    has_turn_data: bool,
) -> Vec<Constraint> {
    header_widths(date_col_width, is_narrow, is_very_narrow, has_turn_data)
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.is_monthly_detail_active() {
        render_detail(frame, app, area);
        return;
    }

    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            tr(lang, MessageKey::TitleMonthlyUsage),
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
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoMonthlyData))
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

    let lang = app.settings.tui_language;
    let header_cells = header_labels(lang, is_narrow, is_very_narrow, has_turn_data);

    let sort_indicator = |field: SortField| -> &'static str {
        if sort_field == field {
            match sort_direction {
                SortDirection::Ascending => " ▴",
                SortDirection::Descending => " ▾",
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
                    total_tokens_cell(month.tokens.total(), &app.theme),
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
                    .style(app.theme.count_style()),
                    total_tokens_cell(month.tokens.total(), &app.theme),
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

    let widths = header_widths(month_col_width, is_narrow, is_very_narrow, has_turn_data);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if monthly_len > visible_height {
        let scrollbar = ambient_stable_scrollbar();

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
    let lang = app.settings.tui_language;
    let title = app
        .monthly_detail_month()
        .map(|month| {
            format!(
                "{}{} ",
                tr(lang, MessageKey::TitleDailyBreakdownPrefix),
                month
            )
        })
        .unwrap_or_else(|| tr(lang, MessageKey::TitleDailyBreakdown).to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
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
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoDailyDataMonth))
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

    let lang = app.settings.tui_language;
    let header_cells = detail_header_labels(lang, is_narrow, is_very_narrow, has_turn_data);

    let sort_indicator = |field: SortField| -> &'static str {
        if sort_field == field {
            match sort_direction {
                SortDirection::Ascending => " ▴",
                SortDirection::Descending => " ▾",
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
                    total_tokens_cell(day.tokens.total(), &app.theme),
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
                    .style(app.theme.count_style()),
                    total_tokens_cell(day.tokens.total(), &app.theme),
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

    let widths = detail_header_widths(date_col_width, is_narrow, is_very_narrow, has_turn_data);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if days_len > visible_height {
        let scrollbar = ambient_stable_scrollbar();

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
    use crate::tui::ui::header_budget::{
        assert_header_layout_fits, assert_headers_render_in_full, fixed_layout_width,
    };
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
            ..Default::default()
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();
        app.settings.tui_language = crate::tui::i18n::TuiLanguage::En;
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
            body.contains("Cache✕"),
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

    /// A month with turn data, for the `has_turn_data` branch of both layouts.
    fn month_with_turns(m: &str, input: u64, cost: f64) -> MonthlyUsage {
        let mut m = month(m, input, cost);
        m.turn_count = 3;
        m
    }

    /// A day with turn data, for the detail table's `has_turn_data` branch.
    fn day_with_turns(date: &str, input: u64, cost: f64) -> DailyUsage {
        let mut d = day(date, input, cost);
        d.turn_count = 3;
        d
    }

    /// The rendered header row of the monthly table.
    fn header_for(lang: TuiLanguage, width: u16, has_turn: bool) -> String {
        let mut app = make_app(width);
        app.settings.tui_language = lang;
        app.data.monthly = vec![if has_turn {
            month_with_turns("2026-05", 1000, 1.5)
        } else {
            month("2026-05", 1000, 1.5)
        }];
        render_body(&mut app, width, 12)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    /// The rendered header row of the monthly *detail* (days-in-month) table.
    fn detail_header_for(lang: TuiLanguage, width: u16, has_turn: bool) -> String {
        let mut app = make_app(width);
        app.settings.tui_language = lang;
        app.data.monthly = vec![month("2026-05", 1000, 1.5)];
        app.data.daily = vec![if has_turn {
            day_with_turns("2026-05-10", 500, 0.75)
        } else {
            day("2026-05-10", 500, 0.75)
        }];
        app.selected_monthly_detail_month = Some("2026-05".to_string());
        render_body(&mut app, width, 12)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    /// Which wide-layout columns carry a sort arrow, index-aligned with
    /// `header_labels`. Read straight off the `sort_indicator` match, which both
    /// `render` and `render_detail` share verbatim: `(0, _, _) => Date` always,
    /// then Tokens and Cost at `(8, 9)` when the Turn column is present and at
    /// `(7, 8)` when it is not — i.e. the Total and Cost columns, whichever
    /// indices they land on. Cost/1M carries none.
    fn sortable_flags(has_turn: bool) -> Vec<bool> {
        let mut flags = vec![true]; // Month (or Date in the detail table)
        if has_turn {
            flags.push(false); // Turn
        }
        // Msgs, Input, Output, Cache R, Cache W, Cache✕
        flags.extend([false; 6]);
        // Total, Cost, Cost/1M
        flags.extend([true, true, false]);
        flags
    }

    /// No header label exceeds the `Constraint::Length` its own layout declares,
    /// in any language, on either table.
    ///
    /// #1367 translated every header without re-checking the budgets these
    /// layouts were solved against, and in `sessions.rs` that shipped Korean
    /// `메시지` clipped to `메시` — a truncated word with no ellipsis, in the one
    /// column whose job is saying how many messages a row has. Nothing here
    /// overflows today; this test is what keeps it that way when the next
    /// language or the next relabelling lands.
    ///
    /// Labels and widths both come from the renderer's own helpers, never
    /// restated here: a test that keeps its own copy of either passes while the
    /// renderer uses something else.
    #[test]
    fn no_header_overflows_its_budget_in_any_language() {
        for lang in TuiLanguage::ALL {
            for has_turn in [true, false] {
                // Both first-column widths, since `compact_full_date` narrows
                // Month/Date to 7 when the full layout does not fit.
                for col_width in [7u16, 12] {
                    assert_header_layout_fits(
                        &format!("monthly/wide(turn={has_turn},month={col_width})"),
                        lang,
                        &header_labels(lang, false, false, has_turn),
                        &header_widths(col_width, false, false, has_turn),
                        &sortable_flags(has_turn),
                    );
                    assert_header_layout_fits(
                        &format!("monthly-detail/wide(turn={has_turn},date={col_width})"),
                        lang,
                        &detail_header_labels(lang, false, false, has_turn),
                        &detail_header_widths(col_width, false, false, has_turn),
                        &sortable_flags(has_turn),
                    );
                }
            }
        }
    }

    /// At and above the width its own `Length`s add up to, the wide layout gets
    /// every cell it asked for, so every header must render in full — in every
    /// language. Below that ratatui shrinks every column and clips English
    /// headers too, which is the pre-existing #964-class over-ask this tab never
    /// solved, not a localization defect.
    #[test]
    fn every_language_renders_its_full_header_once_the_layout_fits() {
        for has_turn in [true, false] {
            let widths = header_widths(12, false, false, has_turn);
            let needed = fixed_layout_width(&widths, 1).expect("wide layout is all Length");
            // +2 for the block borders the table draws inside.
            for width in [needed + 2, needed + 22, 200] {
                for lang in TuiLanguage::ALL {
                    assert_headers_render_in_full(
                        &format!("monthly(width={width},turn={has_turn})"),
                        lang,
                        &header_for(lang, width, has_turn),
                        &header_labels(lang, false, false, has_turn),
                    );
                }
            }
        }
    }

    /// The same claim for the days-in-month detail table, which has its own
    /// header set (Date where the monthly table has Month) and its own renderer.
    #[test]
    fn every_language_renders_its_full_detail_header_once_the_layout_fits() {
        for has_turn in [true, false] {
            let widths = detail_header_widths(12, false, false, has_turn);
            let needed = fixed_layout_width(&widths, 1).expect("wide layout is all Length");
            for width in [needed + 2, needed + 22, 200] {
                for lang in TuiLanguage::ALL {
                    assert_headers_render_in_full(
                        &format!("monthly-detail(width={width},turn={has_turn})"),
                        lang,
                        &detail_header_for(lang, width, has_turn),
                        &detail_header_labels(lang, false, false, has_turn),
                    );
                }
            }
        }
    }
}
