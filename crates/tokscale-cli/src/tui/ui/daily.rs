use chrono::Local;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use tokscale_core::GroupBy;

use super::widgets::{
    format_cache_hit_rate, format_cost, format_cost_per_million, format_ms_per_1k, format_tokens,
    get_provider_display_name, period_source_display_name, period_source_header, total_tokens_cell,
    truncate_text, viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

/// The Daily table's header labels for a layout, in display order.
///
/// A function rather than an inline `vec!` so
/// `rendered_headers_survive_every_language_at_every_width` asserts against the
/// labels the renderer actually writes. A test that restates the label set
/// passes while the renderer uses a different one, which is how the Korean
/// `메시지` clip in `sessions.rs` survived a header test that already existed.
fn header_labels(
    lang: TuiLanguage,
    is_narrow: bool,
    is_very_narrow: bool,
    has_turn_data: bool,
) -> Vec<&'static str> {
    if is_very_narrow {
        return vec![tr(lang, MessageKey::ColDate), tr(lang, MessageKey::ColCost)];
    }

    let mut labels = vec![tr(lang, MessageKey::ColDate)];
    if has_turn_data {
        labels.push(tr(lang, MessageKey::ColTurn));
    }
    labels.push(tr(lang, MessageKey::ColMessages));
    if is_narrow {
        labels.extend([
            tr(lang, MessageKey::ColTokens),
            tr(lang, MessageKey::ColCost),
        ]);
        return labels;
    }
    labels.extend([
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

/// The Daily table's column widths for a layout, index-aligned with
/// [`header_labels`].
///
/// A function rather than an inline `vec!` so `no_header_overflows_its_budget`
/// checks the widths the renderer actually lays out with. The narrow layouts are
/// percentage-based and have no declared budget; the wide ones are `Length`, so
/// a header longer than its `Length` is a clip the arithmetic can name.
fn header_widths(
    date_col_width: u16,
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

    let mut widths = vec![Constraint::Length(date_col_width)];
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

/// The daily *detail* table's header labels, in display order. Same rationale
/// as [`header_labels`].
fn detail_header_labels(
    lang: TuiLanguage,
    group_by: &GroupBy,
    is_narrow: bool,
    is_very_narrow: bool,
    with_speed: bool,
) -> Vec<&'static str> {
    if is_very_narrow {
        return vec![
            tr(lang, MessageKey::ColModel),
            tr(lang, MessageKey::ColCost),
        ];
    }
    if is_narrow {
        return vec![
            tr(lang, MessageKey::ColModel),
            period_source_header(lang, group_by),
            tr(lang, MessageKey::ColMessages),
            tr(lang, MessageKey::ColTokens),
            tr(lang, MessageKey::ColCost),
        ];
    }
    let mut labels = vec![
        tr(lang, MessageKey::ColRank),
        tr(lang, MessageKey::ColModel),
        tr(lang, MessageKey::ColProvider),
        period_source_header(lang, group_by),
        tr(lang, MessageKey::ColMessages),
        tr(lang, MessageKey::ColInput),
        tr(lang, MessageKey::ColOutput),
        tr(lang, MessageKey::ColCacheRead),
        tr(lang, MessageKey::ColCacheWrite),
        tr(lang, MessageKey::ColCacheHit),
        tr(lang, MessageKey::ColTotal),
    ];
    if with_speed {
        labels.push(tr(lang, MessageKey::ColMsPer1k));
    }
    labels.push(tr(lang, MessageKey::ColCost));
    if with_speed {
        labels.push(tr(lang, MessageKey::ColCostPer1M));
    }
    labels
}

/// Width the full detail table needs with the `ms/1K` and `Cost/1M` columns:
/// 147 fixed/minimum cells, 13 column gaps, and the block border. Below it
/// those two columns are dropped so the original 140-cell layout still fits.
const DETAIL_SPEED_COLUMNS_MIN_WIDTH: u16 = 162;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.is_daily_detail_active() {
        render_detail(frame, app, area);
        return;
    }

    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(app.glyphs.border)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            tr(lang, MessageKey::TitleDailyUsage),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let daily = app.get_sorted_daily();
    if daily.is_empty() {
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoDailyData))
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
    let metric_input_style = app.theme.metric_input_style();
    let metric_output_style = app.theme.metric_output_style();
    let metric_cache_read_style = app.theme.metric_cache_read_style();
    let metric_cache_write_style = app.theme.metric_cache_write_style();
    let current_row_style = app.theme.current_row_style();
    let striped_row_style = app.theme.striped_row_style();
    let today = Local::now().date_naive();

    // Date format adapts to *available* width, not just the narrow breakpoint.
    // In full mode the table can still be wider than the terminal, so the year
    // would otherwise get compressed to "2026-0". When the full layout doesn't
    // fit we drop the year (near-constant in a by-day list) to "%m-%d" and
    // shrink the date column, freeing 5 columns. `full_layout_width` is the
    // ideal full-mode total (Length(12) date + spacing); keep it in sync with
    // the `widths` block below.
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
    let header_cells = header_labels(lang, is_narrow, is_very_narrow, has_turn_data);

    let sort_indicator = |field: SortField| -> &'static str {
        if sort_field == field {
            match sort_direction {
                SortDirection::Ascending => app.glyphs.sort_up,
                SortDirection::Descending => app.glyphs.sort_down,
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

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(day.date.format(date_fmt).to_string()).style(if is_today {
                        app.theme.hint_key_style().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    }),
                    Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                let mut cells =
                    vec![
                        Cell::from(day.date.format(date_fmt).to_string()).style(if is_today {
                            app.theme.hint_key_style().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        }),
                    ];
                if has_turn_data {
                    let turn_str = if day.turn_count > 0 {
                        day.turn_count.to_string()
                    } else {
                        app.glyphs.missing.to_string()
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
                let mut cells =
                    vec![
                        Cell::from(day.date.format(date_fmt).to_string()).style(if is_today {
                            app.theme.hint_key_style().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().add_modifier(Modifier::BOLD)
                        }),
                    ];
                if has_turn_data {
                    let turn_str = if day.turn_count > 0 {
                        day.turn_count.to_string()
                    } else {
                        app.glyphs.missing.to_string()
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
                        app.glyphs,
                    ))
                    .style(app.theme.count_style()),
                    total_tokens_cell(day.tokens.total(), &app.theme),
                    Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(format_cost_per_million(
                        day.cost,
                        day.tokens.total(),
                        app.glyphs,
                    ))
                    .style(Style::default().fg(Color::Rgb(150, 200, 150))),
                ]);
                cells
            };

            let row_style = if is_selected {
                Style::default().bg(theme_selection)
            } else if is_today {
                current_row_style
            } else if is_striped {
                striped_row_style
            } else {
                Style::default()
            };

            Row::new(cells).style(row_style).height(1)
        })
        .collect();

    let widths = header_widths(date_col_width, is_narrow, is_very_narrow, has_turn_data);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if daily_len > visible_height {
        let scrollbar = app.glyphs.scrollbar();

        let mut scrollbar_state =
            viewport_scrollbar_state(daily_len, scroll_offset, visible_height);

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

/// The daily *detail* table's column widths, index-aligned with
/// [`detail_header_labels`]. Same rationale as [`header_widths`].
fn detail_header_widths(
    is_narrow: bool,
    is_very_narrow: bool,
    with_speed: bool,
) -> Vec<Constraint> {
    if is_very_narrow {
        return vec![Constraint::Percentage(70), Constraint::Percentage(30)];
    }
    if is_narrow {
        return vec![
            Constraint::Percentage(42),
            Constraint::Percentage(18),
            Constraint::Percentage(12),
            Constraint::Percentage(15),
            Constraint::Percentage(13),
        ];
    }
    let mut widths = vec![
        Constraint::Length(3),
        Constraint::Min(20),
        Constraint::Length(16),
        Constraint::Length(14),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(10),
    ];
    if with_speed {
        // ms/1K and Cost/1M, both 10 cells like their neighbours.
        widths.extend([Constraint::Length(10), Constraint::Length(10)]);
    }
    widths
}

fn render_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let title = app
        .daily_detail_date()
        .map(|date| format!("{}{} ", tr(lang, MessageKey::TitleDailyDetailPrefix), date))
        .unwrap_or_else(|| tr(lang, MessageKey::TitleDailyDetail).to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(app.glyphs.border)
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

    let lang = app.settings.tui_language;
    let rows_data = app.get_sorted_daily_detail_rows();
    if rows_data.is_empty() {
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoModelDetailsDay))
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
    let metric_input_style = app.theme.metric_input_style();
    let metric_output_style = app.theme.metric_output_style();
    let metric_cache_read_style = app.theme.metric_cache_read_style();
    let metric_cache_write_style = app.theme.metric_cache_write_style();
    let striped_row_style = app.theme.striped_row_style();

    let with_speed = !is_narrow && inner.width.saturating_add(2) >= DETAIL_SPEED_COLUMNS_MIN_WIDTH;
    let group_by = app.group_by.borrow().clone();
    let header_cells = detail_header_labels(lang, &group_by, is_narrow, is_very_narrow, with_speed);
    let cost_column = if with_speed { 12 } else { 11 };

    let sort_indicator = |field: SortField| -> &'static str {
        if sort_field == field {
            match sort_direction {
                SortDirection::Ascending => app.glyphs.sort_up,
                SortDirection::Descending => app.glyphs.sort_down,
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
                    (10, false, false) => sort_indicator(SortField::Tokens),
                    (i, false, false) if i == cost_column => sort_indicator(SortField::Cost),
                    (3, true, false) => sort_indicator(SortField::Tokens),
                    (4, true, false) => sort_indicator(SortField::Cost),
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

    let detail_len = rows_data.len();
    let start = scroll_offset.min(detail_len);
    let end = (start + visible_height).min(detail_len);

    if start >= detail_len {
        return;
    }

    let rows: Vec<Row> = rows_data[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;
            let model_color = app.model_color_for(row.provider, row.color_key);

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate_text(row.model, 18)).style(
                        Style::default()
                            .fg(model_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(format_cost(row.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(truncate_text(row.model, 24)).style(
                        Style::default()
                            .fg(model_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(period_source_display_name(&group_by, row.source))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(row.messages.to_string()),
                    total_tokens_cell(row.tokens.total(), &app.theme),
                    Cell::from(format_cost(row.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                let mut cells = vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate_text(row.model, 30)).style(
                        Style::default()
                            .fg(model_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(get_provider_display_name(row.provider)),
                    Cell::from(period_source_display_name(&group_by, row.source))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(row.messages.to_string()),
                    Cell::from(format_tokens(row.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(row.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(row.tokens.cache_read)).style(metric_cache_read_style),
                    Cell::from(format_tokens(row.tokens.cache_write))
                        .style(metric_cache_write_style),
                    Cell::from(format_cache_hit_rate(
                        row.tokens.cache_read,
                        row.tokens.input,
                        row.tokens.cache_write,
                        app.glyphs,
                    ))
                    .style(app.theme.count_style()),
                    total_tokens_cell(row.tokens.total(), &app.theme),
                ];
                if with_speed {
                    cells.push(
                        Cell::from(format_ms_per_1k(row.ms_per_1k_tokens, app.glyphs))
                            .style(app.theme.hint_key_style()),
                    );
                }
                cells.push(
                    Cell::from(format_cost(row.cost)).style(Style::default().fg(Color::Green)),
                );
                if with_speed {
                    cells.push(
                        Cell::from(format_cost_per_million(
                            row.cost,
                            row.tokens.total(),
                            app.glyphs,
                        ))
                        .style(Style::default().fg(Color::Rgb(150, 200, 150))),
                    );
                }
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

    let widths = detail_header_widths(is_narrow, is_very_narrow, with_speed);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if detail_len > visible_height {
        let scrollbar = app.glyphs.scrollbar();

        let mut scrollbar_state =
            viewport_scrollbar_state(detail_len, scroll_offset, visible_height);

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
    use crate::tui::data::{DailyModelInfo, DailySourceInfo, DailyUsage, TokenBreakdown};
    use crate::tui::ui::header_budget::{
        assert_header_layout_fits, assert_headers_render_in_full, fixed_layout_width,
    };
    use chrono::NaiveDate;
    use ratatui::{backend::TestBackend, Terminal};
    use std::collections::BTreeMap;

    /// A grouping whose detail Source column lists clients, and the one that
    /// lists machines instead.
    const DETAIL_GROUPINGS: [GroupBy; 2] = [GroupBy::ClientModel, GroupBy::MachineModel];

    fn day(date: NaiveDate, cost: f64) -> DailyUsage {
        DailyUsage {
            date,
            tokens: TokenBreakdown::default(),
            cost,
            source_breakdown: BTreeMap::new(),
            message_count: 10,
            turn_count: 3,
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
        app.current_tab = Tab::Daily;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app.data.daily = vec![
            day(NaiveDate::from_ymd_opt(2026, 5, 29).unwrap(), 3.0),
            day(NaiveDate::from_ymd_opt(2026, 5, 28).unwrap(), 2.0),
        ];
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
    fn wide_terminal_keeps_year() {
        let mut app = make_app(130);
        let body = render_body(&mut app, 130, 12);
        assert!(
            body.contains("2026-05-29"),
            "a layout that fits should keep the full date\n{body}"
        );
    }

    #[test]
    fn full_mode_drops_year_when_layout_does_not_fit() {
        // 110 cols is full mode (>= 100) but narrower than the ~112-col full
        // layout, so the year is dropped — the date stays readable as "05-29"
        // instead of being compressed to "2026-0".
        let mut app = make_app(110);
        let body = render_body(&mut app, 110, 12);
        assert!(
            !body.contains("2026-05-29"),
            "year should be dropped when the full layout does not fit\n{body}"
        );
        assert!(body.contains("05-29"), "expected compact date\n{body}");
    }
    /// A day with one model row, so the *detail* table has something to render.
    fn day_with_detail(date: NaiveDate, cost: f64) -> DailyUsage {
        let mut day = day(date, cost);
        let mut models = BTreeMap::new();
        models.insert(
            "claude-sonnet-4".to_string(),
            DailyModelInfo {
                provider: "anthropic".to_string(),
                display_name: "claude-sonnet-4".to_string(),
                color_key: "claude-sonnet-4".to_string(),
                tokens: TokenBreakdown::default(),
                cost,
                messages: 10,
                performance: Default::default(),
            },
        );
        day.source_breakdown.insert(
            "claude-code".to_string(),
            DailySourceInfo {
                tokens: TokenBreakdown::default(),
                cost,
                models,
            },
        );
        day
    }

    #[test]
    fn detail_rows_show_speed_and_cost_per_million() {
        let mut app = make_app(200);
        let date = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        let mut day = day_with_detail(date, 3.0);
        let model = day
            .source_breakdown
            .get_mut("claude-code")
            .unwrap()
            .models
            .get_mut("claude-sonnet-4")
            .unwrap();
        // 2M tokens for $3.00, so Cost/1M ($1.50) differs from the raw cost
        // and the assertion proves that column independently.
        model.tokens.input = 2_000_000;
        model.performance = tokscale_core::ModelPerformance::from_totals(90, 2_000, 1);
        app.data.daily = vec![day.clone()];
        app.selected_daily_detail_date = Some(date);

        let body = render_body(&mut app, 200, 12);
        let row = body.lines().nth(2).unwrap_or_default();
        assert!(row.contains("45ms"), "expected ms/1K cell\n{body}");
        assert!(row.contains("$3.00"), "expected Cost cell\n{body}");
        assert!(row.contains("$1.50"), "expected Cost/1M cell\n{body}");

        // Below the speed layout's width the two new columns drop out instead
        // of clipping the original 140-cell table.
        let mut narrow = make_app(150);
        narrow.data.daily = vec![day];
        narrow.selected_daily_detail_date = Some(date);
        let body = render_body(&mut narrow, 150, 12);
        let row = body.lines().nth(2).unwrap_or_default();
        assert!(
            !row.contains("45ms"),
            "speed column must drop below 162\n{body}"
        );
        assert!(row.contains("$3.00"), "Cost stays visible\n{body}");
    }

    #[test]
    fn detail_names_machines_for_machine_model() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        let mut day = day_with_detail(date, 3.0);
        let source = day.source_breakdown.remove("claude-code").unwrap();
        day.source_breakdown.insert("steven".to_string(), source);
        let mut app = make_app(200);
        *app.group_by.borrow_mut() = GroupBy::MachineModel;
        app.data.daily = vec![day];
        app.selected_daily_detail_date = Some(date);

        let body = render_body(&mut app, 200, 12);
        let mut lines = body.lines().skip(1);
        let header = lines.next().unwrap_or_default();
        assert!(header.contains("Machine"), "header: {header}");
        assert!(!header.contains("Source"), "header: {header}");
        let row = lines.next().unwrap_or_default();
        assert!(row.contains("steven"), "row: {row}");
    }

    /// The rendered header row of the main table.
    fn header_for(lang: TuiLanguage, width: u16, has_turn: bool) -> String {
        let mut app = make_app(width);
        app.settings.tui_language = lang;
        for day in &mut app.data.daily {
            day.turn_count = if has_turn { 3 } else { 0 };
        }
        render_body(&mut app, width, 12)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    /// The rendered header row of the detail table.
    fn detail_header_for(lang: TuiLanguage, group_by: &GroupBy, width: u16) -> String {
        let mut app = make_app(width);
        app.settings.tui_language = lang;
        *app.group_by.borrow_mut() = group_by.clone();
        let date = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        app.data.daily = vec![day_with_detail(date, 3.0)];
        app.selected_daily_detail_date = Some(date);
        render_body(&mut app, width, 12)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    /// No header label exceeds the `Constraint::Length` its own layout declares,
    /// in any language, on either table.
    ///
    /// #1367 translated every header without re-checking the budgets these
    /// layouts were solved against, and in `sessions.rs` that shipped Korean
    /// `메시지` clipped to `메시` — a truncated word with no ellipsis, in the
    /// one column whose job is saying how many messages a row has. Nothing here
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
                // Both date-column widths, since `compact_full_date` narrows it
                // to 7 when the full layout does not fit.
                for date_col_width in [7u16, 12] {
                    assert_header_layout_fits(
                        &format!("daily/wide(turn={has_turn},date={date_col_width})"),
                        lang,
                        &header_labels(lang, false, false, has_turn),
                        &header_widths(date_col_width, false, false, has_turn),
                        &sortable_flags(has_turn),
                    );
                }
            }
            for group_by in &DETAIL_GROUPINGS {
                assert_header_layout_fits(
                    &format!("daily-detail/wide(group={group_by:?})"),
                    lang,
                    &detail_header_labels(lang, group_by, false, false, true),
                    &detail_header_widths(false, false, true),
                    // Total and Cost carry the arrows in the detail table.
                    &[
                        false, false, false, false, false, false, false, false, false, false, true,
                        false, true, false,
                    ],
                );
                assert_header_layout_fits(
                    &format!("daily-detail/wide-without-speed(group={group_by:?})"),
                    lang,
                    &detail_header_labels(lang, group_by, false, false, false),
                    &detail_header_widths(false, false, false),
                    &[
                        false, false, false, false, false, false, false, false, false, false, true,
                        true,
                    ],
                );
            }
        }
    }

    /// Which wide-layout columns carry a sort arrow, index-aligned with
    /// `header_labels`. Date, Total and Cost, mirroring the `sort_indicator`
    /// match in `render_table`.
    fn sortable_flags(has_turn: bool) -> Vec<bool> {
        let mut flags = vec![true]; // Date
        if has_turn {
            flags.push(false); // Turn
        }
        // Msgs, Input, Output, Cache R, Cache W, Cache✕
        flags.extend([false; 6]);
        // Total, Cost, Cost/1M
        flags.extend([true, true, false]);
        flags
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
            for width in [needed + 2, needed + 20, 200] {
                for lang in TuiLanguage::ALL {
                    assert_headers_render_in_full(
                        &format!("daily(width={width},turn={has_turn})"),
                        lang,
                        &header_for(lang, width, has_turn),
                        &header_labels(lang, false, false, has_turn),
                    );
                }
            }
        }
    }

    /// The same claim for the detail table, which has its own header set, its
    /// own constraints and a flexible Model column. `Min` is unbounded above, so
    /// the fitting width is measured from the `Length`s plus Model's minimum.
    #[test]
    fn every_language_renders_its_full_detail_header_once_the_layout_fits() {
        let needed_for = |with_speed: bool| {
            let widths = detail_header_widths(false, false, with_speed);
            let fixed: u16 = widths
                .iter()
                .map(|constraint| match constraint {
                    Constraint::Length(cells) => *cells,
                    // The sole `Min`, which is Model's floor.
                    Constraint::Min(cells) => *cells,
                    other => unreachable!("unexpected constraint {other:?}"),
                })
                .sum();
            fixed + widths.len().saturating_sub(1) as u16 + 2
        };
        // The speed columns appear exactly when their layout fits, and the
        // layout without them keeps the 140 cells it needed before.
        assert_eq!(needed_for(true), DETAIL_SPEED_COLUMNS_MIN_WIDTH);
        assert_eq!(needed_for(false), 140);
        let cases = [
            (false, needed_for(false)),
            (false, DETAIL_SPEED_COLUMNS_MIN_WIDTH - 1),
            (true, DETAIL_SPEED_COLUMNS_MIN_WIDTH),
            (true, 200),
        ];
        for (with_speed, width) in cases {
            for lang in TuiLanguage::ALL {
                for group_by in &DETAIL_GROUPINGS {
                    assert_headers_render_in_full(
                        &format!("daily-detail(width={width},group={group_by:?})"),
                        lang,
                        &detail_header_for(lang, group_by, width),
                        &detail_header_labels(lang, group_by, false, false, with_speed),
                    );
                }
            }
        }
    }
}
