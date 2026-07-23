use chrono::{Local, NaiveDateTime, TimeZone};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table,
};

use super::widgets::{
    format_cache_hit_rate, format_cost, format_cost_per_million, format_tokens,
    get_client_display_name, total_tokens_cell, truncate_text, viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Sessions ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let sessions = app.get_sorted_sessions();
    if sessions.is_empty() {
        let empty_msg = Paragraph::new("No session usage data found. Press 'r' to refresh.")
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let has_turn_data = sessions.iter().any(|s| s.turn_count > 0);
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

    // Timestamp format adapts to width: full date+time when there's room,
    // compact "mm-dd HH:MM" when the full layout would overflow.
    let last_active_fmt: &str = if is_narrow || is_very_narrow {
        "%m-%d %H:%M"
    } else {
        "%Y-%m-%d %H:%M"
    };

    let header_cells = if is_very_narrow {
        vec!["Session", "Cost"]
    } else if is_narrow {
        if has_turn_data {
            vec!["Session", "Client", "Turn", "Msgs", "Tokens", "Cost"]
        } else {
            vec!["Session", "Client", "Msgs", "Tokens", "Cost"]
        }
    } else if has_turn_data {
        vec![
            "Session",
            "Client",
            "Turn",
            "Msgs",
            "Input",
            "Output",
            "Cache R",
            "Cache W",
            "Cache×",
            "Total",
            "Cost",
            "Cost/1M",
            "Duration",
            "Last Active",
        ]
    } else {
        vec![
            "Session",
            "Client",
            "Msgs",
            "Input",
            "Output",
            "Cache R",
            "Cache W",
            "Cache×",
            "Total",
            "Cost",
            "Cost/1M",
            "Duration",
            "Last Active",
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
                // "Last Active" column index is the sort target for Date.
                let last_active_idx = if is_very_narrow {
                    usize::MAX // no last-active column in very narrow mode
                } else if is_narrow {
                    // narrow drops last-active entirely; Date sort has no header indicator
                    usize::MAX
                } else if has_turn_data {
                    13
                } else {
                    12
                };
                let total_idx = if is_very_narrow {
                    usize::MAX
                } else if is_narrow {
                    if has_turn_data {
                        4
                    } else {
                        3
                    }
                } else if has_turn_data {
                    9
                } else {
                    8
                };
                let cost_idx = if is_very_narrow {
                    1
                } else if is_narrow {
                    if has_turn_data {
                        5
                    } else {
                        4
                    }
                } else if has_turn_data {
                    10
                } else {
                    9
                };
                let indicator = if i == last_active_idx {
                    sort_indicator(SortField::Date)
                } else if i == total_idx {
                    sort_indicator(SortField::Tokens)
                } else if i == cost_idx {
                    sort_indicator(SortField::Cost)
                } else {
                    ""
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

    let sessions_len = sessions.len();
    let start = scroll_offset.min(sessions_len);
    let end = (start + visible_height).min(sessions_len);

    if start >= sessions_len {
        return;
    }

    let rows: Vec<Row> = sessions[start..end]
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let last_active_str = ms_to_local_naive(session.last_active_ms)
                .map(|dt| dt.format(last_active_fmt).to_string())
                .unwrap_or_else(|| "\u{2014}".to_string());
            let duration_str = format_duration(session.first_active_ms, session.last_active_ms);

            // Show the human-readable title when the source client stored one,
            // falling back to the session ID for clients that don't.
            let session_label = session
                .title
                .as_deref()
                .filter(|t| !t.is_empty())
                .unwrap_or(&session.session_id);

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate_text(session_label, 20)).style(
                        Style::default()
                            .fg(theme_muted)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(format_cost(session.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                let mut cells = vec![Cell::from(truncate_text(session_label, 24)).style(
                    Style::default()
                        .fg(theme_muted)
                        .add_modifier(Modifier::BOLD),
                )];
                cells.push(
                    Cell::from(get_client_display_name(&session.client))
                        .style(Style::default().fg(theme_muted)),
                );
                if has_turn_data {
                    let turn_str = if session.turn_count > 0 {
                        session.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(session.message_count.to_string()),
                    total_tokens_cell(session.tokens.total(), &app.theme),
                    Cell::from(format_cost(session.cost)).style(Style::default().fg(Color::Green)),
                ]);
                cells
            } else {
                let mut cells = vec![Cell::from(truncate_text(session_label, 60)).style(
                    Style::default()
                        .fg(theme_muted)
                        .add_modifier(Modifier::BOLD),
                )];
                cells.push(
                    Cell::from(get_client_display_name(&session.client))
                        .style(Style::default().fg(theme_muted)),
                );
                if has_turn_data {
                    let turn_str = if session.turn_count > 0 {
                        session.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(session.message_count.to_string()),
                    Cell::from(format_tokens(session.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(session.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(session.tokens.cache_read))
                        .style(metric_cache_read_style),
                    Cell::from(format_tokens(session.tokens.cache_write))
                        .style(metric_cache_write_style),
                    Cell::from(format_cache_hit_rate(
                        session.tokens.cache_read,
                        session.tokens.input,
                        session.tokens.cache_write,
                    ))
                    .style(Style::default().fg(Color::Cyan)),
                    total_tokens_cell(session.tokens.total(), &app.theme),
                    Cell::from(format_cost(session.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(format_cost_per_million(
                        session.cost,
                        session.tokens.total(),
                    ))
                    .style(Style::default().fg(Color::Rgb(150, 200, 150))),
                    Cell::from(duration_str).style(Style::default().fg(Color::Yellow)),
                    Cell::from(last_active_str).style(Style::default().fg(theme_muted)),
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
            Constraint::Percentage(28),
            Constraint::Percentage(16),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
        ]
    } else if is_narrow {
        vec![
            Constraint::Percentage(32),
            Constraint::Percentage(18),
            Constraint::Percentage(12),
            Constraint::Percentage(18),
            Constraint::Percentage(20),
        ]
    } else if has_turn_data {
        vec![
            Constraint::Min(20),
            Constraint::Length(12),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(17),
        ]
    } else {
        vec![
            Constraint::Min(20),
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(17),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if sessions_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state =
            viewport_scrollbar_state(sessions_len, scroll_offset, visible_height);

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

/// Convert Unix-ms to a local NaiveDateTime for display.
fn ms_to_local_naive(ms: i64) -> Option<NaiveDateTime> {
    if ms <= 0 {
        return None;
    }
    let secs = ms / 1000;
    match Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => Some(dt.naive_local()),
        _ => None,
    }
}

/// Human-readable elapsed time between first and last message in a session.
/// Returns "—" when the bounds are missing or inverted.
fn format_duration(first_ms: i64, last_ms: i64) -> String {
    if first_ms <= 0 || last_ms <= 0 || last_ms < first_ms {
        return "\u{2014}".to_string();
    }
    let secs = (last_ms - first_ms) / 1000;
    if secs <= 0 {
        return "0s".to_string();
    }
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        format!("{}s", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{Tab, TuiConfig};
    use crate::tui::data::{SessionUsage, TokenBreakdown};
    use ratatui::{backend::TestBackend, Terminal};

    fn session(id: &str, client: &str, cost: f64, last_ms: i64) -> SessionUsage {
        SessionUsage {
            session_id: id.to_string(),
            client: client.to_string(),
            title: None,
            tokens: TokenBreakdown {
                input: 1000,
                output: 500,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            message_count: 4,
            turn_count: 2,
            first_active_ms: last_ms.saturating_sub(3_600_000),
            last_active_ms: last_ms,
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
        app.current_tab = Tab::Sessions;
        app.sort_field = SortField::Cost;
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
    fn wide_terminal_renders_session_and_duration_columns() {
        let mut app = make_app(200);
        app.data.sessions = vec![
            session("abc-123", "opencode", 1.5, 1_736_000_000_000),
            session("def-456", "claude", 0.25, 1_736_000_000_000),
        ];
        let body = render_body(&mut app, 200, 12);
        assert!(body.contains("abc-123"), "expected session id\n{body}");
        assert!(
            body.contains("Duration"),
            "expected duration header\n{body}"
        );
        assert!(
            body.contains("Last Active"),
            "expected last active header\n{body}"
        );
    }

    #[test]
    fn session_title_displayed_when_available() {
        let mut app = make_app(200);
        let mut s = session("ses_09d2eac0", "opencode", 1.5, 1_736_000_000_000);
        s.title = Some("This is a long title for a session that could be used".to_string());
        app.data.sessions = vec![s];
        let body = render_body(&mut app, 200, 12);
        assert!(
            body.contains("This is a long"),
            "expected session title in body\n{body}"
        );
    }

    #[test]
    fn session_column_expands_on_wide_terminal() {
        let mut app = make_app(220);
        let mut s = session("ses_09d2eac0", "opencode", 1.5, 1_736_000_000_000);
        s.title = Some("This is a long title for a session that could be used".to_string());
        app.data.sessions = vec![s];
        let body = render_body(&mut app, 220, 12);
        assert!(
            body.contains("This is a long title for a session that could be used"),
            "expected full title on wide terminal\n{body}"
        );
    }

    #[test]
    fn narrow_terminal_drops_token_breakdown_columns() {
        let mut app = make_app(70);
        app.data.sessions = vec![session("abc-123", "opencode", 1.5, 1_736_000_000_000)];
        let body = render_body(&mut app, 70, 12);
        assert!(body.contains("abc-123"), "expected session id\n{body}");
        assert!(
            !body.contains("Cache×"),
            "cache hit rate should be dropped in narrow mode\n{body}"
        );
    }

    #[test]
    fn empty_sessions_shows_refresh_message() {
        let mut app = make_app(120);
        let body = render_body(&mut app, 120, 12);
        assert!(
            body.contains("No session usage data"),
            "expected empty message\n{body}"
        );
    }

    #[test]
    fn format_duration_handles_days_hours_minutes_seconds() {
        assert_eq!(format_duration(0, 1000), "—");
        assert_eq!(format_duration(1000, 500), "—");
        assert_eq!(format_duration(1000, 1000), "0s");
        assert_eq!(format_duration(0, 0), "—");
        assert_eq!(format_duration(1000, 2000), "1s");
        assert_eq!(format_duration(1000, 61_000), "1m");
        assert_eq!(format_duration(1000, 3_661_000), "1h 1m");
        assert_eq!(format_duration(1000, 90_061_000), "1d 1h");
    }

    #[test]
    fn format_duration_boundary_units() {
        // exactly one hour without extra minutes (use a real first_ms since
        // first_ms == 0 short-circuits to "—")
        assert_eq!(format_duration(1_000, 3_601_000), "1h 0m");
        // exactly one day
        assert_eq!(format_duration(1_000, 86_401_000), "1d 0h");
    }
}
