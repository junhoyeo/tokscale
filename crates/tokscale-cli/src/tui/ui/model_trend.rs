//! The Models tab's per-model daily trend: one row per day for the selected
//! model, so its speed (ms/1K) and effective price (Cost/1M) can be read over
//! time rather than only as an all-time aggregate.

use chrono::Local;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::models::model_display_name;
use super::widgets::{
    format_cost, format_cost_per_million, format_ms_per_1k, format_tokens, total_tokens_cell,
    viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

/// The trend table's header labels, in display order. A function so the
/// header tests check the labels the renderer actually writes.
fn header_labels(lang: TuiLanguage, is_narrow: bool, is_very_narrow: bool) -> Vec<&'static str> {
    if is_very_narrow {
        return vec![tr(lang, MessageKey::ColDate), tr(lang, MessageKey::ColCost)];
    }
    if is_narrow {
        return vec![
            tr(lang, MessageKey::ColDate),
            tr(lang, MessageKey::ColMsPer1k),
            tr(lang, MessageKey::ColCost),
            tr(lang, MessageKey::ColCostPer1M),
        ];
    }
    vec![
        tr(lang, MessageKey::ColDate),
        tr(lang, MessageKey::ColMessages),
        tr(lang, MessageKey::ColInput),
        tr(lang, MessageKey::ColOutput),
        tr(lang, MessageKey::ColCacheRead),
        tr(lang, MessageKey::ColCacheWrite),
        tr(lang, MessageKey::ColTotal),
        tr(lang, MessageKey::ColMsPer1k),
        tr(lang, MessageKey::ColCost),
        tr(lang, MessageKey::ColCostPer1M),
    ]
}

/// Width the wide trend layout needs (see `header_widths`).
const MODEL_TREND_WIDE_MIN_WIDTH: u16 = 109;

/// The trend table's column widths, index-aligned with [`header_labels`].
fn header_widths(is_narrow: bool, is_very_narrow: bool) -> Vec<Constraint> {
    if is_very_narrow {
        return vec![Constraint::Percentage(60), Constraint::Percentage(40)];
    }
    if is_narrow {
        return vec![
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ];
    }
    vec![
        Constraint::Length(12),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
    ]
}

/// The sort field whose arrow belongs on header column `index`.
fn sort_column(index: usize, is_narrow: bool, is_very_narrow: bool) -> Option<SortField> {
    match (index, is_narrow, is_very_narrow) {
        (0, _, _) => Some(SortField::Date),
        (1, _, true) => Some(SortField::Cost),
        (2, true, false) => Some(SortField::Cost),
        (6, false, false) => Some(SortField::Tokens),
        (8, false, false) => Some(SortField::Cost),
        _ => None,
    }
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let group_by = app.group_by.borrow().clone();
    let title = app
        .model_trend_model()
        .map(|model| {
            format!(
                "{}{} ",
                tr(lang, MessageKey::TitleModelTrendPrefix),
                model_display_name(model, &group_by)
            )
        })
        .unwrap_or_else(|| tr(lang, MessageKey::TitleModelTrendPrefix).to_string());

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

    let days = app.get_sorted_model_trend_rows();
    if days.is_empty() {
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoModelTrendData))
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    // The wide layout needs 98 fixed cells, 9 column gaps and the 2-cell
    // block border; below that ratatui would shrink and clip every column.
    let is_narrow = area.width < MODEL_TREND_WIDE_MIN_WIDTH;
    let is_very_narrow = app.is_very_narrow();
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_selection = app.theme.selection;
    let metric_input_style = app.theme.metric_input_style();
    let metric_output_style = app.theme.metric_output_style();
    let metric_cache_read_style = app.theme.metric_cache_read_style();
    let metric_cache_write_style = app.theme.metric_cache_write_style();
    let speed_style = app.theme.hint_key_style();
    let current_row_style = app.theme.current_row_style();
    let striped_row_style = app.theme.striped_row_style();
    let today = Local::now().date_naive();
    let date_fmt = if is_very_narrow { "%m/%d" } else { "%Y-%m-%d" };

    let header = Row::new(
        header_labels(lang, is_narrow, is_very_narrow)
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let indicator = match sort_column(i, is_narrow, is_very_narrow) {
                    Some(field) if field == sort_field => match sort_direction {
                        SortDirection::Ascending => app.glyphs.sort_up,
                        SortDirection::Descending => app.glyphs.sort_down,
                    },
                    _ => "",
                };
                Cell::from(format!("{label}{indicator}"))
            })
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(app.theme.accent)
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
            let is_today = day.date == today;
            let date_cell = Cell::from(day.date.format(date_fmt).to_string()).style(if is_today {
                app.theme.hint_key_style().add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            });
            let speed_cell = Cell::from(format_ms_per_1k(
                day.performance.ms_per_1k_tokens,
                app.glyphs,
            ))
            .style(speed_style);
            let cost_cell =
                Cell::from(format_cost(day.cost)).style(Style::default().fg(Color::Green));
            let cost_per_m_cell = Cell::from(format_cost_per_million(
                day.cost,
                day.tokens.total(),
                app.glyphs,
            ))
            .style(Style::default().fg(Color::Rgb(150, 200, 150)));

            let cells: Vec<Cell> = if is_very_narrow {
                vec![date_cell, cost_cell]
            } else if is_narrow {
                vec![date_cell, speed_cell, cost_cell, cost_per_m_cell]
            } else {
                vec![
                    date_cell,
                    Cell::from(day.messages.to_string()),
                    Cell::from(format_tokens(day.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(day.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(day.tokens.cache_read)).style(metric_cache_read_style),
                    Cell::from(format_tokens(day.tokens.cache_write))
                        .style(metric_cache_write_style),
                    total_tokens_cell(day.tokens.total(), &app.theme),
                    speed_cell,
                    cost_cell,
                    cost_per_m_cell,
                ]
            };

            let row_style = if idx == selected_index {
                Style::default().bg(theme_selection)
            } else if is_today {
                current_row_style
            } else if idx % 2 == 1 {
                striped_row_style
            } else {
                Style::default()
            };

            Row::new(cells).style(row_style).height(1)
        })
        .collect();

    let table = Table::new(rows, header_widths(is_narrow, is_very_narrow))
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));
    frame.render_widget(table, inner);

    if days_len > visible_height {
        let mut scrollbar_state = viewport_scrollbar_state(days_len, scroll_offset, visible_height);
        frame.render_stateful_widget(
            app.glyphs.scrollbar(),
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
    use crate::tui::data::{ModelDayUsage, ModelUsage, TokenBreakdown};
    use crate::tui::ui::header_budget::{
        assert_header_layout_fits, assert_headers_render_in_full, fixed_layout_width,
    };
    use chrono::NaiveDate;
    use ratatui::{backend::TestBackend, Terminal};
    use tokscale_core::ModelPerformance;

    fn day(date: &str, cost: f64, duration_ms: i64) -> ModelDayUsage {
        ModelDayUsage {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            tokens: TokenBreakdown {
                input: 1_000_000,
                ..TokenBreakdown::default()
            },
            cost,
            messages: 4,
            performance: ModelPerformance::from_totals(duration_ms, 1_000, 1),
        }
    }

    fn make_app(width: u16) -> App {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            initial_tab: None,
            ..Default::default()
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();
        app.settings.tui_language = TuiLanguage::En;
        app.terminal_width = width;
        app.current_tab = Tab::Models;
        app.data.models = vec![ModelUsage {
            model: "claude-sonnet-4-5".to_string(),
            color_key: "claude-sonnet-4-5".to_string(),
            provider: "anthropic".to_string(),
            client: "claude".to_string(),
            workspace_key: None,
            workspace_label: None,
            machine: None,
            tokens: TokenBreakdown::default(),
            cost: 5.0,
            performance: ModelPerformance::default(),
            session_count: 1,
            group_key: "claude-sonnet-4-5".to_string(),
            daily: vec![day("2026-05-28", 2.0, 30), day("2026-05-29", 3.0, 45)],
        }];
        app.selected_model_trend = Some("claude-sonnet-4-5".to_string());
        app.sort_field = SortField::Date;
        app.sort_direction = SortDirection::Descending;
        app
    }

    fn render_body(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, app, Rect::new(0, 0, width, height)))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn trend_lists_each_day_newest_first_with_speed_and_cost_per_million() {
        let mut app = make_app(140);
        let body = render_body(&mut app, 140, 8);
        let lines: Vec<&str> = body.lines().collect();

        assert!(
            lines[0].contains("Daily Trend: claude-sonnet-4-5"),
            "{body}"
        );
        assert!(lines[2].contains("2026-05-29"), "{body}");
        assert!(lines[2].contains("45ms"), "{body}");
        assert!(lines[2].contains("$3.00"), "{body}");
        assert!(lines[3].contains("2026-05-28"), "{body}");
        assert!(lines[3].contains("30ms"), "{body}");
        assert!(lines[3].contains("$2.00"), "{body}");
    }

    #[test]
    fn narrow_trend_keeps_speed_and_cost_per_million() {
        let mut app = make_app(80);
        let body = render_body(&mut app, 80, 8);
        let row = body.lines().nth(2).unwrap_or_default();
        assert!(row.contains("45ms") && row.contains("$3.00"), "{body}");
    }

    #[test]
    fn missing_model_shows_the_empty_state() {
        let mut app = make_app(140);
        app.selected_model_trend = Some("gone".to_string());
        let body = render_body(&mut app, 140, 8);
        assert!(body.contains("No daily data for this model yet"), "{body}");
    }

    #[test]
    fn no_header_overflows_its_budget_in_any_language() {
        for lang in TuiLanguage::ALL {
            assert_header_layout_fits(
                "model-trend/wide",
                lang,
                &header_labels(lang, false, false),
                &header_widths(false, false),
                &(0..10)
                    .map(|i| sort_column(i, false, false).is_some())
                    .collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn every_language_renders_its_full_header_once_the_layout_fits() {
        let widths = header_widths(false, false);
        let needed = fixed_layout_width(&widths, 1).expect("wide layout is all Length");
        for width in [needed + 2, 200] {
            for lang in TuiLanguage::ALL {
                let mut app = make_app(width);
                app.settings.tui_language = lang;
                let header = render_body(&mut app, width, 8)
                    .lines()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                assert_headers_render_in_full(
                    &format!("model-trend(width={width})"),
                    lang,
                    &header,
                    &header_labels(lang, false, false),
                );
            }
        }
    }
}
