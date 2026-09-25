use chrono::{Local, NaiveDate, Timelike};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::hourly_profile;
use super::widgets::{
    ambient_stable_scrollbar, format_cache_hit_rate, format_cost, format_cost_per_million,
    format_tokens, total_tokens_cell, viewport_scrollbar_state, AMBIENT_STABLE_BORDER_SET,
};
use crate::tui::app::{App, HourlyViewMode, SortDirection, SortField};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

/// The Hourly table's header labels for a layout, in display order.
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
        return vec![tr(lang, MessageKey::ColHour), tr(lang, MessageKey::ColCost)];
    }

    let mut labels = vec![
        tr(lang, MessageKey::ColHour),
        tr(lang, MessageKey::ColSource),
    ];
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

/// The Hourly table's column widths, index-aligned with [`header_labels`].
///
/// A function rather than an inline `vec!` so
/// `no_header_overflows_its_budget_in_any_language` checks the widths the
/// renderer actually lays out with, not a copy of them. The narrow layouts are
/// percentage-based and have no declared budget; the wide ones are `Length`, so
/// a header longer than its `Length` is a clip the arithmetic can name.
fn header_widths(is_narrow: bool, is_very_narrow: bool, has_turn_data: bool) -> Vec<Constraint> {
    if is_very_narrow {
        return vec![Constraint::Percentage(60), Constraint::Percentage(40)];
    }
    if is_narrow {
        return if has_turn_data {
            vec![
                Constraint::Percentage(25),
                Constraint::Percentage(20),
                Constraint::Percentage(12),
                Constraint::Percentage(13),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
            ]
        } else {
            vec![
                Constraint::Percentage(30),
                Constraint::Percentage(25),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
            ]
        };
    }
    let mut widths = vec![Constraint::Length(7), Constraint::Length(14)];
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

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.hourly_view_mode {
        HourlyViewMode::Table => render_table(frame, app, area),
        HourlyViewMode::Profile => hourly_profile::render(frame, app, area),
    }
}

fn render_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            tr(lang, MessageKey::TitleHourlyUsage),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    // NB: unlike the other tabs we do NOT seed max_visible_items with
    // visible_height here. Separators make the effective page smaller than the
    // raw height, so the real page size is set at the end of the render loop
    // (data rows actually shown). Clamping with visible_height up front would
    // over-restrict scroll and make the last rows unreachable. Scroll/selection
    // are kept valid by the nav handlers and update_data(); a resize only ever
    // shrinks the window from the same start, which stays in-bounds.

    let hourly = app.get_sorted_hourly();
    if hourly.is_empty() {
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoHourlyData))
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
    let metric_input_style = app.theme.metric_input_style();
    let metric_output_style = app.theme.metric_output_style();
    let metric_cache_read_style = app.theme.metric_cache_read_style();
    let metric_cache_write_style = app.theme.metric_cache_write_style();
    let current_row_style = app.theme.current_row_style();
    let striped_row_style = app.theme.striped_row_style();
    let now = Local::now().naive_local();
    let current_hour = now.date().and_hms_opt(now.hour(), 0, 0).unwrap_or(now);

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

    // Day-boundary separators factor the date out of every data row: rows show
    // only the hour bucket ("%H:00"), and a muted separator row carrying the
    // "%m/%d" date is emitted at the top of the window and at each day change.
    // This keeps the time column ~5 wide instead of repeating "2026-05-29 " on
    // every line (which ratatui was compressing down to "2026-0").
    //
    // Separators consume vertical space and ratatui clips rows that overflow
    // the area (`get_row_bounds`), so we budget by *rendered lines* (data rows
    // + separators) rather than slicing a fixed data-row count, then report the
    // actual data rows shown via `max_visible_items` so paging/scroll stay in
    // sync.
    let hourly_len = hourly.len();
    let start = scroll_offset.min(hourly_len);
    if start >= hourly_len {
        return;
    }

    let sep_style = Style::default()
        .fg(theme_accent)
        .bg(Color::Rgb(24, 28, 36))
        .add_modifier(Modifier::BOLD);

    // Turn count → display string ("—" when the hour has no turn data).
    let turn_cell = |count: u32| -> String {
        if count > 0 {
            count.to_string()
        } else {
            "\u{2014}".to_string()
        }
    };

    let mut rows: Vec<Row> = Vec::with_capacity(visible_height + 1);
    let mut lines_used = 0usize;
    let mut prev_date: Option<NaiveDate> = None;
    let mut data_idx = start;

    while data_idx < hourly_len && lines_used < visible_height {
        let hour = &hourly[data_idx];
        let row_date = hour.datetime.date();

        // Separator at the window top (prev_date == None) and at each day
        // change. When only one line remains it cannot share the line budget
        // with its data row, so we DROP THE SEPARATOR but still render the data
        // row. This keeps the selected row visible even when it is the first
        // row of a new day in a one-line viewport: skipping the data row here
        // (the old behaviour) made the highlight disappear because
        // `max_visible_items` is driven by data rows actually rendered, so the
        // nav/scroll clamp would not advance the window to reveal it.
        if prev_date != Some(row_date) && lines_used + 1 < visible_height {
            rows.push(
                Row::new(vec![
                    Cell::from(row_date.format("%m/%d").to_string()).style(sep_style)
                ])
                .height(1),
            );
            lines_used += 1;
        }
        prev_date = Some(row_date);

        let idx = data_idx;
        let is_selected = idx == selected_index;
        let is_striped = idx % 2 == 1;
        let is_current = hour.datetime == current_hour;

        let clients_str: String = {
            let mut c: Vec<&str> = hour.clients.iter().map(String::as_str).collect();
            c.sort();
            c.join(", ")
        };

        let time_str = hour.datetime.format("%H:00").to_string();
        let time_style = if is_current {
            app.theme.hint_key_style().add_modifier(Modifier::BOLD)
        } else if !is_narrow && !is_very_narrow {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let cells: Vec<Cell> = if is_very_narrow {
            vec![
                Cell::from(time_str).style(time_style),
                Cell::from(format_cost(hour.cost)).style(Style::default().fg(Color::Green)),
            ]
        } else if is_narrow {
            let mut cells = vec![
                Cell::from(time_str).style(time_style),
                Cell::from(clients_str),
            ];
            if has_turn_data {
                cells.push(Cell::from(turn_cell(hour.turn_count)));
            }
            cells.extend([
                Cell::from(hour.message_count.to_string()),
                total_tokens_cell(hour.tokens.total(), &app.theme),
                Cell::from(format_cost(hour.cost)).style(Style::default().fg(Color::Green)),
            ]);
            cells
        } else {
            let mut cells = vec![
                Cell::from(time_str).style(time_style),
                Cell::from(clients_str),
            ];
            if has_turn_data {
                cells.push(Cell::from(turn_cell(hour.turn_count)));
            }
            cells.extend([
                Cell::from(hour.message_count.to_string()),
                Cell::from(format_tokens(hour.tokens.input)).style(metric_input_style),
                Cell::from(format_tokens(hour.tokens.output)).style(metric_output_style),
                Cell::from(format_tokens(hour.tokens.cache_read)).style(metric_cache_read_style),
                Cell::from(format_tokens(hour.tokens.cache_write)).style(metric_cache_write_style),
                Cell::from(format_cache_hit_rate(
                    hour.tokens.cache_read,
                    hour.tokens.input,
                    hour.tokens.cache_write,
                ))
                .style(app.theme.count_style()),
                total_tokens_cell(hour.tokens.total(), &app.theme),
                Cell::from(format_cost(hour.cost)).style(Style::default().fg(Color::Green)),
                Cell::from(format_cost_per_million(hour.cost, hour.tokens.total()))
                    .style(Style::default().fg(Color::Rgb(150, 200, 150))),
            ]);
            cells
        };

        let row_style = if is_selected {
            Style::default().bg(theme_selection)
        } else if is_current {
            current_row_style
        } else if is_striped {
            striped_row_style
        } else {
            Style::default()
        };

        rows.push(Row::new(cells).style(row_style).height(1));
        lines_used += 1;
        data_idx += 1;
    }

    // Effective data rows shown (separators excluded) drives paging & scroll on
    // the next frame. `hourly` is no longer borrowed past this point, so the
    // mutable field write is sound.
    let data_rows_shown = data_idx - start;
    app.set_max_visible_items(data_rows_shown.max(1));

    let widths = header_widths(is_narrow, is_very_narrow, has_turn_data);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if hourly_len > visible_height {
        let scrollbar = ambient_stable_scrollbar();

        let mut scrollbar_state =
            viewport_scrollbar_state(hourly_len, scroll_offset, data_rows_shown);

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
    use crate::tui::data::{HourlyUsage, TokenBreakdown};
    use crate::tui::ui::header_budget::{
        assert_header_layout_fits, assert_headers_render_in_full, fixed_layout_width,
    };
    use chrono::NaiveDate;
    use ratatui::{backend::TestBackend, Terminal};
    use std::collections::{BTreeMap, BTreeSet};

    fn hour(date: NaiveDate, h: u32) -> HourlyUsage {
        let mut clients = BTreeSet::new();
        clients.insert("claude".to_string());
        HourlyUsage {
            datetime: date.and_hms_opt(h, 0, 0).unwrap(),
            tokens: TokenBreakdown::default(),
            cost: 1.0,
            clients,
            models: BTreeMap::new(),
            message_count: 5,
            turn_count: 2,
        }
    }

    /// App with two days of hourly data (3 hours on 05-29, 2 on 05-28),
    /// sorted newest-first like the live default.
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
        app.current_tab = Tab::Hourly;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        let d1 = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        let d0 = NaiveDate::from_ymd_opt(2026, 5, 28).unwrap();
        app.data.hourly = vec![
            hour(d1, 14),
            hour(d1, 13),
            hour(d1, 12),
            hour(d0, 23),
            hour(d0, 22),
        ];
        app
    }

    fn render_lines(app: &mut App, width: u16, height: u16) -> Vec<String> {
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
            .collect()
    }

    #[test]
    fn compact_time_with_day_separators() {
        let mut app = make_app(120);
        let body = render_lines(&mut app, 120, 20).join("\n");

        // Hour buckets are compact "%H:00", not the old per-row timestamp.
        assert!(body.contains("14:00"), "expected HH:00 bucket\n{body}");
        assert!(
            !body.contains("2026-05-29 14"),
            "the full date must not repeat on every row\n{body}"
        );
        // The date lives on muted day-boundary separator rows as "%m/%d".
        assert!(body.contains("05/29"), "expected 05/29 separator\n{body}");
        assert!(body.contains("05/28"), "expected 05/28 separator\n{body}");
    }

    #[test]
    fn selected_row_visible_in_single_line_viewport() {
        // Regression guard for the day-boundary separator bug both cubic and
        // Codex flagged: when the selected row is the FIRST row of a new day and
        // the viewport has room for only one line, the separator and the data
        // row cannot share the budget. The fix drops the separator but still
        // renders the data row, so the selected row never disappears. The old
        // `break` form skipped the data row, leaving the highlight invisible
        // until another keypress (max_visible_items is driven by data rows
        // actually rendered, so the nav/scroll clamp would not advance).
        //
        // Data is newest-first: index 3 == 05/28 23:00 is the first row of the
        // older day (it needs a separator). Pin the window to it.
        let mut app = make_app(120);
        app.scroll_offset = 3;
        app.selected_index = 3;

        // height 4 → inner height 2 → visible_height = 2 - 1 = 1 (single line):
        // the separator and the data row cannot both fit on the one line left.
        let body = render_lines(&mut app, 120, 4).join("\n");

        // The old `break` form bailed here and rendered nothing, so the selected
        // hour was invisible until another keypress. The fix drops the separator
        // but still renders the data row.
        assert!(
            body.contains("23:00"),
            "selected row (05/28 23:00) must stay visible in a one-line viewport\n{body}"
        );
    }

    #[test]
    fn window_never_overflows_height_and_reports_data_rows() {
        let mut app = make_app(120);
        // Tight height forces the line budget to bite (separators + data rows).
        let height = 6u16;
        let lines = render_lines(&mut app, 120, height);

        // Buffer is exactly `height` rows — nothing rendered past the area.
        assert_eq!(lines.len(), height as usize);
        // max_visible_items counts DATA rows shown (separators excluded), and
        // can never exceed the rows area (height - borders - header).
        assert!(app.max_visible_items >= 1);
        assert!(app.max_visible_items <= (height as usize).saturating_sub(3));
    }

    /// The rendered header row of the Hourly table.
    fn header_for(lang: TuiLanguage, width: u16, has_turn: bool) -> String {
        let mut app = make_app(width);
        app.settings.tui_language = lang;
        for hour in &mut app.data.hourly {
            hour.turn_count = if has_turn { 2 } else { 0 };
        }
        render_lines(&mut app, width, 12)
            .into_iter()
            .nth(1)
            .unwrap_or_default()
    }

    /// Which wide-layout columns carry a sort arrow, index-aligned with
    /// `header_labels`. Read straight off the `sort_indicator` match in
    /// `render_table`: `(0, _, _) => Date` always, then Tokens and Cost at
    /// `(9, 10)` when the Turn column is present and at `(8, 9)` when it is not —
    /// i.e. the Total and Cost columns, whichever indices they land on. Cost/1M
    /// carries none.
    fn sortable_flags(has_turn: bool) -> Vec<bool> {
        // Hour, Source
        let mut flags = vec![true, false];
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
    /// in any language.
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
                assert_header_layout_fits(
                    &format!("hourly/wide(turn={has_turn})"),
                    lang,
                    &header_labels(lang, false, false, has_turn),
                    &header_widths(false, false, has_turn),
                    &sortable_flags(has_turn),
                );
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
            let widths = header_widths(false, false, has_turn);
            let needed = fixed_layout_width(&widths, 1).expect("wide layout is all Length");
            // +2 for the block borders the table draws inside.
            for width in [needed + 2, needed + 22, 200] {
                for lang in TuiLanguage::ALL {
                    assert_headers_render_in_full(
                        &format!("hourly(width={width},turn={has_turn})"),
                        lang,
                        &header_for(lang, width, has_turn),
                        &header_labels(lang, false, false, has_turn),
                    );
                }
            }
        }
    }
}
