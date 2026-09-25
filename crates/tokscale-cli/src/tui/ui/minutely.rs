use chrono::{Local, Timelike};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::widgets::{
    ambient_stable_scrollbar, format_cache_hit_rate, format_cost, format_tokens, total_tokens_cell,
    viewport_scrollbar_state, AMBIENT_STABLE_BORDER_SET,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

/// The Minutely table's header labels for a layout, in display order.
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
            tr(lang, MessageKey::ColMinute),
            tr(lang, MessageKey::ColCost),
        ];
    }

    let mut labels = vec![
        tr(lang, MessageKey::ColMinute),
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
    ]);
    labels
}

/// The Minutely table's column widths, index-aligned with [`header_labels`].
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
                Constraint::Percentage(32),
                Constraint::Percentage(18),
                Constraint::Percentage(10),
                Constraint::Percentage(10),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
            ]
        } else {
            vec![
                Constraint::Percentage(32),
                Constraint::Percentage(23),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
            ]
        };
    }
    let mut widths = vec![Constraint::Length(18), Constraint::Length(14)];
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
    ]);
    widths
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            tr(lang, MessageKey::TitleMinutelyUsage),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let minutely = app.get_sorted_minutely();
    if minutely.is_empty() {
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoMinutelyData))
            .style(Style::default().fg(app.theme.muted))
            .wrap(ratatui::widgets::Wrap { trim: true })
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let has_turn_data = minutely.iter().any(|m| m.turn_count > 0);
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
    let current_minute = now
        .date()
        .and_hms_opt(now.hour(), now.minute(), 0)
        .unwrap_or(now);

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

    let minutely_len = minutely.len();
    let start = scroll_offset.min(minutely_len);
    let end = (start + visible_height).min(minutely_len);

    if start >= minutely_len {
        return;
    }

    let rows: Vec<Row> = minutely[start..end]
        .iter()
        .enumerate()
        .map(|(i, minute)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;
            let is_current = minute.datetime == current_minute;

            let clients_str: String = {
                let mut c: Vec<&str> = minute.clients.iter().map(String::as_str).collect();
                c.sort();
                c.join(", ")
            };

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(minute.datetime.format("%m/%d %H:%M").to_string()).style(
                        if is_current {
                            app.theme.hint_key_style().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ),
                    Cell::from(format_cost(minute.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                let mut cells = vec![
                    Cell::from(minute.datetime.format("%m-%d %H:%M").to_string()).style(
                        if is_current {
                            app.theme.hint_key_style().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ),
                    Cell::from(clients_str),
                ];
                if has_turn_data {
                    let turn_str = if minute.turn_count > 0 {
                        minute.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(minute.message_count.to_string()),
                    total_tokens_cell(minute.tokens.total(), &app.theme),
                    Cell::from(format_cost(minute.cost)).style(Style::default().fg(Color::Green)),
                ]);
                cells
            } else {
                let mut cells = vec![
                    Cell::from(minute.datetime.format("%Y-%m-%d %H:%M").to_string()).style(
                        if is_current {
                            app.theme.hint_key_style().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().add_modifier(Modifier::BOLD)
                        },
                    ),
                    Cell::from(clients_str),
                ];
                if has_turn_data {
                    let turn_str = if minute.turn_count > 0 {
                        minute.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(minute.message_count.to_string()),
                    Cell::from(format_tokens(minute.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(minute.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(minute.tokens.cache_read))
                        .style(metric_cache_read_style),
                    Cell::from(format_tokens(minute.tokens.cache_write))
                        .style(metric_cache_write_style),
                    Cell::from(format_cache_hit_rate(
                        minute.tokens.cache_read,
                        minute.tokens.input,
                        minute.tokens.cache_write,
                    ))
                    .style(app.theme.count_style()),
                    total_tokens_cell(minute.tokens.total(), &app.theme),
                    Cell::from(format_cost(minute.cost)).style(Style::default().fg(Color::Green)),
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

            Row::new(cells).style(row_style).height(1)
        })
        .collect();

    let widths = header_widths(is_narrow, is_very_narrow, has_turn_data);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if minutely_len > visible_height {
        let scrollbar = ambient_stable_scrollbar();

        let mut scrollbar_state =
            viewport_scrollbar_state(minutely_len, scroll_offset, visible_height);

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
    use crate::tui::data::{MinutelyUsage, TokenBreakdown};
    use crate::tui::ui::header_budget::{
        assert_header_layout_fits, assert_headers_render_in_full, fixed_layout_width,
    };
    use chrono::NaiveDate;
    use ratatui::{backend::TestBackend, Terminal};
    use std::collections::{BTreeMap, BTreeSet};

    fn minute(date: NaiveDate, h: u32, m: u32) -> MinutelyUsage {
        let mut clients = BTreeSet::new();
        clients.insert("claude".to_string());
        MinutelyUsage {
            datetime: date.and_hms_opt(h, m, 0).unwrap(),
            tokens: TokenBreakdown::default(),
            cost: 1.0,
            clients,
            models: BTreeMap::new(),
            message_count: 5,
            turn_count: 2,
        }
    }

    /// App with two minute buckets, sorted newest-first like the live default.
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
        app.settings.tui_language = TuiLanguage::En;
        app.terminal_width = width;
        app.current_tab = Tab::Minutely;
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        let date = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        app.data.minutely = vec![minute(date, 14, 31), minute(date, 14, 30)];
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

    /// The rendered header row of the Minutely table.
    fn header_for(lang: TuiLanguage, width: u16, has_turn: bool) -> String {
        let mut app = make_app(width);
        app.settings.tui_language = lang;
        for m in &mut app.data.minutely {
            m.turn_count = if has_turn { 2 } else { 0 };
        }
        render_body(&mut app, width, 12)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    /// Which wide-layout columns carry a sort arrow, index-aligned with
    /// `header_labels`. Read straight off the `sort_indicator` match in `render`:
    /// `(0, _, _) => Date` always, then Tokens and Cost at `(9, 10)` when the
    /// Turn column is present and at `(8, 9)` when it is not — i.e. the Total and
    /// Cost columns, whichever indices they land on. This table has no Cost/1M
    /// column, so Cost is the last one.
    fn sortable_flags(has_turn: bool) -> Vec<bool> {
        // Minute, Source
        let mut flags = vec![true, false];
        if has_turn {
            flags.push(false); // Turn
        }
        // Msgs, Input, Output, Cache R, Cache W, Cache✕
        flags.extend([false; 6]);
        // Total, Cost
        flags.extend([true, true]);
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
                    &format!("minutely/wide(turn={has_turn})"),
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
                        &format!("minutely(width={width},turn={has_turn})"),
                        lang,
                        &header_for(lang, width, has_turn),
                        &header_labels(lang, false, false, has_turn),
                    );
                }
            }
        }
    }
}
