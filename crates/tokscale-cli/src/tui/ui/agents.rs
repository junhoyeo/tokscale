use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};
use tokscale_core::ClientId;

use super::widgets::{format_cost, format_tokens, get_client_display_name};
use crate::tui::app::{App, SortDirection, SortField};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Agents ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.max_visible_items = visible_height;

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_muted = app.theme.muted;
    let theme_selection = app.theme.selection;

    let agents = app.get_sorted_agents();
    if agents.is_empty() {
        let empty_msg = Paragraph::new(get_empty_message(app))
            .style(Style::default().fg(theme_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let header_cells = if is_very_narrow {
        vec!["Agent", "Cost"]
    } else if is_narrow {
        vec!["Agent", "Tokens", "Cost"]
    } else {
        vec!["#", "Agent", "Source", "Tokens", "Cost", "Msgs"]
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
                let indicator = match i {
                    3 if !is_narrow => sort_indicator(SortField::Tokens),
                    4 if !is_narrow => sort_indicator(SortField::Cost),
                    1 if is_very_narrow => sort_indicator(SortField::Cost),
                    2 if is_narrow && !is_very_narrow => sort_indicator(SortField::Cost),
                    1 if is_narrow && !is_very_narrow => sort_indicator(SortField::Tokens),
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

    let agents_len = agents.len();
    let start = scroll_offset.min(agents_len.saturating_sub(1));
    let end = (start + visible_height).min(agents_len);

    if start >= agents_len {
        return;
    }

    let rows: Vec<Row> = agents[start..end]
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate(&agent.agent, 18))
                        .style(Style::default().fg(app.theme.foreground)),
                    Cell::from(format_cost(agent.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(truncate(&agent.agent, 18))
                        .style(Style::default().fg(app.theme.foreground)),
                    Cell::from(format_tokens(agent.tokens.total())),
                    Cell::from(format_cost(agent.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate(&agent.agent, 22)).style(
                        Style::default()
                            .fg(app.theme.foreground)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(truncate(&client_labels(&agent.clients), 24))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(format_tokens(agent.tokens.total())),
                    Cell::from(format_cost(agent.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(agent.message_count.to_string())
                        .style(Style::default().fg(theme_muted)),
                ]
            };

            let row_style = if is_selected {
                Style::default().bg(theme_selection)
            } else if is_striped {
                Style::default().bg(Color::Rgb(20, 24, 30))
            } else {
                Style::default()
            };

            Row::new(cells).style(row_style).height(1)
        })
        .collect();

    let widths = if is_very_narrow {
        vec![Constraint::Percentage(70), Constraint::Percentage(30)]
    } else if is_narrow {
        vec![
            Constraint::Percentage(45),
            Constraint::Percentage(27),
            Constraint::Percentage(28),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(16),
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(6),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if agents_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state = ScrollbarState::new(agents_len).position(scroll_offset);

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

fn get_empty_message(app: &App) -> String {
    let enabled_clients = app.enabled_clients.borrow();
    let only_codex = !enabled_clients.is_empty()
        && enabled_clients
            .iter()
            .all(|client| *client == ClientId::Codex);

    if only_codex {
        "No agent breakdown is available for the current sources.\nThe selected source usually does not record agent metadata for regular sessions.\nPress 's' to try a different source."
            .to_string()
    } else {
        "No agent breakdown is available for the current sources.\nOnly some sources record agent metadata.\nPress 's' to change sources or 'r' to refresh."
            .to_string()
    }
}

fn client_labels(clients: &str) -> String {
    clients
        .split(", ")
        .map(get_client_display_name)
        .collect::<Vec<_>>()
        .join(", ")
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

#[cfg(test)]
mod tests {
    use super::get_empty_message;
    use crate::tui::app::{App, TuiConfig};
    use crate::tui::data::UsageData;
    use tokscale_core::ClientId;

    fn make_app(clients: Vec<ClientId>) -> App {
        let app = App::new_with_cached_data(
            TuiConfig {
                theme: "tokscale".to_string(),
                refresh: 0,
                sessions_path: None,
                clients: None,
                since: None,
                until: None,
                year: None,
                initial_tab: None,
            },
            Some(UsageData::default()),
        )
        .unwrap();

        *app.enabled_clients.borrow_mut() = clients.into_iter().collect();
        app
    }

    #[test]
    fn test_get_empty_message_for_codex_only() {
        let app = make_app(vec![ClientId::Codex]);
        let message = get_empty_message(&app);

        assert!(message.contains("selected source usually does not record"));
        assert!(message.contains("try a different source"));
    }

    #[test]
    fn test_get_empty_message_for_mixed_sources() {
        let app = make_app(vec![ClientId::OpenCode, ClientId::RooCode]);
        let message = get_empty_message(&app);

        assert!(message.contains("Only some sources record agent metadata"));
        assert!(message.contains("change sources"));
    }
}
