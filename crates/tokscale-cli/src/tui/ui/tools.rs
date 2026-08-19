use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table,
};

use super::widgets::{
    format_tokens_with_commas, get_client_display_name, truncate_text, viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};

/// Tool calls, per tool.
///
/// Deliberately carries no cost or token column. A tool call is recorded on the
/// message that made it, and that message's tokens pay for the whole turn, so
/// splitting them across the tools it happened to call would invent an
/// attribution nobody measured. Calls and the messages that made them are the
/// honest numbers here.
pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Tools ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // One line is reserved for the partial-coverage notice when there is one,
    // so the table never renders a row underneath it.
    let notice = coverage_notice(app);
    let (table_area, notice_area) = match notice {
        Some(_) if inner.height > 2 => (
            Rect {
                height: inner.height - 1,
                ..inner
            },
            Some(Rect {
                y: inner.y + inner.height - 1,
                height: 1,
                ..inner
            }),
        ),
        _ => (inner, None),
    };

    let visible_height = table_area.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_muted = app.theme.muted;
    let theme_foreground = app.theme.foreground;
    let theme_selection = app.theme.selection;
    let striped_row_style = app.theme.striped_row_style();

    let tools = app.get_sorted_tools();
    if tools.is_empty() {
        let empty = Paragraph::new(empty_message(app))
            .style(Style::default().fg(theme_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let header_cells = if is_very_narrow {
        vec!["Tool", "Calls"]
    } else if is_narrow {
        vec!["Tool", "Calls", "Msgs"]
    } else {
        vec!["#", "Tool", "Kind", "Source", "Calls", "Msgs"]
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

    let calls_column = if is_very_narrow || is_narrow { 1 } else { 4 };
    let name_column = if is_very_narrow || is_narrow { 0 } else { 1 };
    let header = Row::new(
        header_cells
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let indicator = if i == calls_column {
                    sort_indicator(SortField::Count)
                } else if i == name_column {
                    sort_indicator(SortField::Name)
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

    let tools_len = tools.len();
    let start = scroll_offset.min(tools_len.saturating_sub(1));
    let end = (start + visible_height).min(tools_len);
    if start >= tools_len {
        return;
    }

    let rows: Vec<Row> = tools[start..end]
        .iter()
        .enumerate()
        .map(|(i, tool)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate_text(&tool.name, 18))
                        .style(Style::default().fg(theme_foreground)),
                    Cell::from(format_tokens_with_commas(tool.calls))
                        .style(Style::default().fg(theme_accent)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(truncate_text(&tool.name, 18))
                        .style(Style::default().fg(theme_foreground)),
                    Cell::from(format_tokens_with_commas(tool.calls))
                        .style(Style::default().fg(theme_accent)),
                    Cell::from(tool.message_count.to_string())
                        .style(Style::default().fg(theme_muted)),
                ]
            } else {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate_text(&tool.name, 30)).style(
                        Style::default()
                            .fg(theme_foreground)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(if tool.mcp { "MCP" } else { "built-in" })
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(truncate_text(&client_labels(&tool.clients), 22))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(format_tokens_with_commas(tool.calls))
                        .style(Style::default().fg(theme_accent)),
                    Cell::from(tool.message_count.to_string())
                        .style(Style::default().fg(theme_muted)),
                ]
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
        vec![Constraint::Percentage(65), Constraint::Percentage(35)]
    } else if is_narrow {
        vec![
            Constraint::Percentage(50),
            Constraint::Percentage(28),
            Constraint::Percentage(22),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(22),
            Constraint::Length(9),
            Constraint::Length(22),
            Constraint::Length(10),
            Constraint::Length(7),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));
    frame.render_widget(table, table_area);

    if let (Some(text), Some(area)) = (notice, notice_area) {
        let notice = Paragraph::new(text)
            .style(Style::default().fg(theme_muted))
            .alignment(Alignment::Center);
        frame.render_widget(notice, area);
    }

    if tools_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state =
            viewport_scrollbar_state(tools_len, scroll_offset, visible_height);
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

/// Says how much of the loaded data this view could not see.
///
/// Most clients do not report tool calls, and a message parsed before the field
/// existed reports unknown rather than zero. Without this line the table reads
/// as a complete tally of everything that ran, which for a mixed set of sources
/// it is not.
pub(crate) fn coverage_notice(app: &App) -> Option<String> {
    let unknown = app.data.messages_without_tool_data;
    if unknown == 0 {
        return None;
    }
    Some(format!(
        "{} messages do not report tool calls and are not counted here",
        format_tokens_with_commas(u64::from(unknown))
    ))
}

fn empty_message(app: &App) -> String {
    if app.data.messages_without_tool_data > 0 {
        "No tool calls were recorded for the current sources.\nThe selected sources do not report tool calls.\nPress 's' to change sources or 'r' to refresh."
            .to_string()
    } else {
        "No tool calls were recorded for the current sources.\nPress 's' to change sources or 'r' to refresh."
            .to_string()
    }
}

fn client_labels(clients: &str) -> String {
    clients
        .split(", ")
        .filter(|client| !client.is_empty())
        .map(get_client_display_name)
        .collect::<Vec<_>>()
        .join(", ")
}
