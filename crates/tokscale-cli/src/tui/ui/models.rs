use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

use super::widgets::{
    format_cache_hit_rate, format_cost, format_tokens, get_client_display_name,
    get_provider_display_name,
};
use crate::tui::app::{App, SortDirection, SortField};
use tokscale_core::GroupBy;

fn workspace_label(model: &crate::tui::data::ModelUsage) -> &str {
    model
        .workspace_label
        .as_deref()
        .unwrap_or("Unknown workspace")
}

fn model_display_name(model: &crate::tui::data::ModelUsage, group_by: &GroupBy) -> String {
    if *group_by == GroupBy::WorkspaceModel {
        format!("{} / {}", workspace_label(model), model.model)
    } else {
        model.model.clone()
    }
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Models ",
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
    let group_by = app.group_by.borrow().clone();
    let theme_accent = app.theme.accent;
    let theme_muted = app.theme.muted;
    let theme_selection = app.theme.selection;

    let models = app.get_sorted_models();
    if models.is_empty() {
        let empty_msg = Paragraph::new(
            "No usage data found. Press 'r' to refresh, 's' for sources, 'g' for grouping.",
        )
        .style(Style::default().fg(theme_muted))
        .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let header_cells = if is_very_narrow {
        vec!["Model", "Cost"]
    } else if is_narrow {
        vec!["Model", "Tokens", "Cost"]
    } else if group_by == GroupBy::WorkspaceModel {
        vec![
            "#",
            "Workspace",
            "Model",
            "Provider",
            "Source",
            "Input",
            "Output",
            "Cache Read",
            "Cache Write",
            "Total",
            "Cost",
        ]
    } else {
        vec![
            "#", "Model", "Provider", "Source", "Input", "Output", "Cache R", "Cache W", "Cache×",
            "Total", "Cost",
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
                let indicator = match i {
                    9 if !is_narrow => sort_indicator(SortField::Tokens),
                    10 if !is_narrow => sort_indicator(SortField::Cost),
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

    let models_len = models.len();
    let start = scroll_offset.min(models_len.saturating_sub(1));
    let end = (start + visible_height).min(models_len);

    if start >= models_len {
        return;
    }

    let rows: Vec<Row> = models[start..end]
        .iter()
        .enumerate()
        .map(|(i, model)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let model_color = app.model_color_for(&model.provider, &model.model);
            let display_name = model_display_name(model, &group_by);

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate(&display_name, 15)).style(Style::default().fg(model_color)),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(truncate(&display_name, 25)).style(Style::default().fg(model_color)),
                    Cell::from(format_tokens(model.tokens.total())),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if group_by == GroupBy::WorkspaceModel {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate(workspace_label(model), 18)).style(
                        Style::default()
                            .fg(theme_accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(truncate(&model.model, 24)).style(
                        Style::default()
                            .fg(model_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(get_provider_display_name(&model.provider)),
                    Cell::from(get_client_display_name(&model.client))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(format_tokens(model.tokens.input))
                        .style(Style::default().fg(Color::Rgb(100, 200, 100))),
                    Cell::from(format_tokens(model.tokens.output))
                        .style(Style::default().fg(Color::Rgb(200, 100, 100))),
                    Cell::from(format_tokens(model.tokens.cache_read))
                        .style(Style::default().fg(Color::Rgb(100, 150, 200))),
                    Cell::from(format_tokens(model.tokens.cache_write))
                        .style(Style::default().fg(Color::Rgb(200, 150, 100))),
                    Cell::from(format_tokens(model.tokens.total())),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate(&model.model, 30)).style(
                        Style::default()
                            .fg(model_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(get_provider_display_name(&model.provider)),
                    Cell::from(get_client_display_name(&model.client))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(format_tokens(model.tokens.input))
                        .style(Style::default().fg(Color::Rgb(100, 200, 100))),
                    Cell::from(format_tokens(model.tokens.output))
                        .style(Style::default().fg(Color::Rgb(200, 100, 100))),
                    Cell::from(format_tokens(model.tokens.cache_read))
                        .style(Style::default().fg(Color::Rgb(100, 150, 200))),
                    Cell::from(format_tokens(model.tokens.cache_write))
                        .style(Style::default().fg(Color::Rgb(200, 150, 100))),
                    Cell::from(format_cache_hit_rate(
                        model.tokens.cache_read,
                        model.tokens.input,
                        model.tokens.cache_write,
                    ))
                    .style(Style::default().fg(Color::Cyan)),
                    Cell::from(format_tokens(model.tokens.total())),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
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
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ]
    } else if group_by == GroupBy::WorkspaceModel {
        vec![
            Constraint::Length(3),
            Constraint::Length(18),
            Constraint::Min(20),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(20),
            Constraint::Length(18),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if models_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state = ScrollbarState::new(models_len).position(scroll_offset);

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
