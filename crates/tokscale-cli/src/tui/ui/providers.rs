use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table};

use super::widgets::{
    format_cost, format_tokens, get_provider_display_name, viewport_scrollbar_state,
};
use crate::tui::app::{App, ProviderRow, SortDirection, SortField};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Providers ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let rows_data = app.get_provider_rows();
    if rows_data.is_empty() {
        let empty_msg = Paragraph::new("No provider usage data found. Press 'r' to refresh.")
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
    let striped_row_style = app.theme.striped_row_style();

    let header_cells = if is_very_narrow {
        vec!["Provider", "Cost"]
    } else if is_narrow {
        vec!["Provider / Model", "Tokens", "Cost"]
    } else {
        vec!["#", "Provider / Model", "Tokens", "Cost", "Sessions"]
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
                    2 if !is_narrow => sort_indicator(SortField::Tokens),
                    3 if !is_narrow => sort_indicator(SortField::Cost),
                    1 if is_very_narrow => sort_indicator(SortField::Cost),
                    1 if is_narrow && !is_very_narrow => sort_indicator(SortField::Tokens),
                    2 if is_narrow && !is_very_narrow => sort_indicator(SortField::Cost),
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

    let total_len = rows_data.len();
    let start = scroll_offset.min(total_len.saturating_sub(1));
    let end = (start + visible_height).min(total_len);

    if start >= total_len {
        return;
    }

    let rows: Vec<Row> = rows_data[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let (label, tokens, cost, sessions, is_provider) = match row {
                ProviderRow::Provider(provider) => {
                    let marker = if app.expanded_providers.contains(&provider.provider) {
                        "▾"
                    } else {
                        "▸"
                    };
                    (
                        format!("{} {}", marker, get_provider_display_name(&provider.provider)),
                        provider.tokens.total(),
                        provider.cost,
                        provider.session_count,
                        true,
                    )
                }
                ProviderRow::Model { model, .. } => (
                    format!("  └ {}", model.model),
                    model.tokens.total(),
                    model.cost,
                    model.session_count,
                    false,
                ),
            };

            let name_style = if is_provider {
                Style::default()
                    .fg(app.theme.foreground)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme_muted)
            };

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate(&label, 22)).style(name_style),
                    Cell::from(format_cost(cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(truncate(&label, 28)).style(name_style),
                    Cell::from(format_tokens(tokens)),
                    Cell::from(format_cost(cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate(&label, 40)).style(name_style),
                    Cell::from(format_tokens(tokens)),
                    Cell::from(format_cost(cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(sessions.to_string()).style(Style::default().fg(theme_muted)),
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
        vec![Constraint::Percentage(70), Constraint::Percentage(30)]
    } else if is_narrow {
        vec![
            Constraint::Percentage(48),
            Constraint::Percentage(25),
            Constraint::Percentage(27),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(28),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(8),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if total_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state = viewport_scrollbar_state(total_len, scroll_offset, visible_height);
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
