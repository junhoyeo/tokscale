use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table,
};

use super::widgets::{
    fit_workspace_label_to_width, format_cache_hit_rate, format_cost, format_cost_per_million,
    format_ms_per_1k, format_tokens, get_client_display_name, get_provider_display_name,
    total_tokens_cell, truncate_text, truncate_to_width, viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};
use tokscale_core::GroupBy;

/// Width the Workspace column gets when the row has no spare cells: what every
/// other grouping's second column gets, so nothing is taken from its neighbors.
const WORKSPACE_COLUMN_BASE_WIDTH: u16 = 18;

/// Width the Workspace column gets once the row has surplus to spend.
///
/// Enough for `repo ⑃ worktree` on a maximized terminal, which is the case the
/// truncated-label bug was actually reported from.
const WORKSPACE_COLUMN_WIDE_WIDTH: u16 = 44;

/// Inner width at which the row can actually satisfy the wide Workspace column
/// without pushing Model below its `Min(20)`.
///
/// Below this the layout is zero-sum: widening Workspace can only come out of Cost
/// (clipping a dollar figure) or Model, which just relocates the
/// unreadable-truncation bug instead of fixing it. Measured against the solver —
/// 193 is the first width where Workspace is granted the full 44 cells and Model
/// still holds 20. Pinned by `workspace_column_request_matches_what_it_is_granted`.
const WORKSPACE_SURPLUS_MIN_WIDTH: u16 = 193;

/// Cells the Workspace column asks for in a table rendered into `total`.
///
/// Deliberately a fixed `Length` at both sizes rather than a `Min`: `Min` outranks
/// `Length` in ratatui's solver, so a flexible workspace column steals from its
/// neighbors on any row that cannot satisfy everyone.
fn workspace_column_width(total: u16) -> u16 {
    if total >= WORKSPACE_SURPLUS_MIN_WIDTH {
        WORKSPACE_COLUMN_WIDE_WIDTH
    } else {
        WORKSPACE_COLUMN_BASE_WIDTH
    }
}

/// Column indices in the workspace layout, for callers that need a cell's budget.
const WORKSPACE_COL_WORKSPACE: usize = 1;
const WORKSPACE_COL_MODEL: usize = 2;

/// Cells column `index` is granted out of already-solved `widths`, which is what
/// text in that cell has to be truncated to.
///
/// A `Length` is a request, not a guarantee, and `Min` floats: the solver resizes
/// both to make the row fit. Truncating to the requested width leaves the overflow
/// to be clipped by the renderer with no ellipsis — the same silent truncation this
/// change exists to remove. Reading the solved width closes that gap at every width
/// for every column, instead of each cell carrying a constant that is right only
/// for the sizes someone happened to check.
///
/// Takes the solved widths rather than an inner width so `render` can solve once
/// per frame instead of once per cell.
fn workspace_granted_width(widths: &[u16], index: usize) -> usize {
    widths
        .get(index)
        .copied()
        .unwrap_or(WORKSPACE_COLUMN_BASE_WIDTH) as usize
}

/// The workspace layout's column constraints for a row of `total` cells.
fn workspace_column_constraints(total: u16) -> Vec<Constraint> {
    vec![
        Constraint::Length(3),
        Constraint::Length(workspace_column_width(total)),
        Constraint::Min(20),
        Constraint::Length(16),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
    ]
}

/// Widths the solver grants each column of the workspace layout at `total` cells.
fn workspace_table_widths(total: u16) -> Vec<u16> {
    Layout::horizontal(workspace_column_constraints(total))
        .spacing(1)
        .split(Rect::new(0, 0, total, 1))
        .iter()
        .map(|area| area.width)
        .collect()
}

fn workspace_label(model: &crate::tui::data::ModelUsage) -> &str {
    model
        .workspace_label
        .as_deref()
        .unwrap_or("Unknown workspace")
}

fn model_display_name(model: &crate::tui::data::ModelUsage, group_by: &GroupBy) -> String {
    if matches!(
        group_by,
        GroupBy::WorkspaceModel | GroupBy::WorkspaceProviderModel
    ) {
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
    app.set_max_visible_items(visible_height);

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
    let metric_input_style = app.theme.metric_input_style();
    let metric_output_style = app.theme.metric_output_style();
    let metric_cache_read_style = app.theme.metric_cache_read_style();
    let metric_cache_write_style = app.theme.metric_cache_write_style();
    let striped_row_style = app.theme.striped_row_style();

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
    } else if matches!(
        group_by,
        GroupBy::WorkspaceModel | GroupBy::WorkspaceProviderModel
    ) {
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
            "ms/1K",
            "Cost",
            "Cost/1M",
        ]
    } else {
        vec![
            "#", "Model", "Provider", "Source", "Input", "Output", "Cache R", "Cache W", "Cache×",
            "Total", "ms/1K", "Cost", "Cost/1M",
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
                    11 if !is_narrow => sort_indicator(SortField::Cost),
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

    // Solve the workspace layout once per render, not once per cell: the widths
    // depend only on `inner.width`, so running the solver inside the row loop
    // repeated the same work (and its allocation) twice for every visible row.
    let workspace_widths = if group_by == GroupBy::WorkspaceModel {
        workspace_table_widths(inner.width)
    } else {
        Vec::new()
    };
    let granted = |index: usize| workspace_granted_width(&workspace_widths, index);

    let rows: Vec<Row> = models[start..end]
        .iter()
        .enumerate()
        .map(|(i, model)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let model_color = app.model_color_for(&model.provider, &model.color_key);
            let display_name = model_display_name(model, &group_by);

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate_text(&display_name, 15))
                        .style(Style::default().fg(model_color)),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(truncate_text(&display_name, 25))
                        .style(Style::default().fg(model_color)),
                    total_tokens_cell(model.tokens.total(), &app.theme),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if matches!(
                group_by,
                GroupBy::WorkspaceModel | GroupBy::WorkspaceProviderModel
            ) {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    // Not a plain head cut, unlike every other cell: a workspace
                    // label is identified by the ends of each of its segments,
                    // and cutting the tail leaves the prefix every row shares —
                    // which rendered distinct worktrees as identical rows at the
                    // 18 cells this column gets on an ordinary terminal.
                    Cell::from(fit_workspace_label_to_width(
                        workspace_label(model),
                        granted(WORKSPACE_COL_WORKSPACE),
                    ))
                    .style(
                        Style::default()
                            .fg(theme_accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    // Cells, not code points, and the granted width rather than a
                    // constant: Model holds the flexible slot here, so the solver
                    // hands it 20 cells on a narrow row and far more on a wide one.
                    // A fixed 24 over-truncated at 97 widths and under-truncated at
                    // the rest.
                    Cell::from(truncate_to_width(
                        &model.model,
                        granted(WORKSPACE_COL_MODEL),
                    ))
                    .style(
                        Style::default()
                            .fg(model_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(get_provider_display_name(&model.provider)),
                    Cell::from(get_client_display_name(&model.client))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(format_tokens(model.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(model.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(model.tokens.cache_read))
                        .style(metric_cache_read_style),
                    Cell::from(format_tokens(model.tokens.cache_write))
                        .style(metric_cache_write_style),
                    total_tokens_cell(model.tokens.total(), &app.theme),
                    Cell::from(format_ms_per_1k(model.performance.ms_per_1k_tokens))
                        .style(Style::default().fg(Color::Yellow)),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(format_cost_per_million(model.cost, model.tokens.total()))
                        .style(Style::default().fg(Color::Rgb(150, 200, 150))),
                ]
            } else {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate_text(&model.model, 30)).style(
                        Style::default()
                            .fg(model_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(get_provider_display_name(&model.provider)),
                    Cell::from(get_client_display_name(&model.client))
                        .style(Style::default().fg(theme_muted)),
                    Cell::from(format_tokens(model.tokens.input)).style(metric_input_style),
                    Cell::from(format_tokens(model.tokens.output)).style(metric_output_style),
                    Cell::from(format_tokens(model.tokens.cache_read))
                        .style(metric_cache_read_style),
                    Cell::from(format_tokens(model.tokens.cache_write))
                        .style(metric_cache_write_style),
                    Cell::from(format_cache_hit_rate(
                        model.tokens.cache_read,
                        model.tokens.input,
                        model.tokens.cache_write,
                    ))
                    .style(Style::default().fg(Color::Cyan)),
                    total_tokens_cell(model.tokens.total(), &app.theme),
                    Cell::from(format_ms_per_1k(model.performance.ms_per_1k_tokens))
                        .style(Style::default().fg(Color::Yellow)),
                    Cell::from(format_cost(model.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(format_cost_per_million(model.cost, model.tokens.total()))
                        .style(Style::default().fg(Color::Rgb(150, 200, 150))),
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
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ]
    } else if matches!(
        group_by,
        GroupBy::WorkspaceModel | GroupBy::WorkspaceProviderModel
    ) {
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

        let mut scrollbar_state =
            viewport_scrollbar_state(models_len, scroll_offset, visible_height);

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
    use crate::tui::data::ModelUsage;

    fn usage(model: &str, workspace: Option<&str>) -> ModelUsage {
        ModelUsage {
            model: model.to_string(),
            color_key: model.to_string(),
            provider: "anthropic".to_string(),
            client: "claude".to_string(),
            workspace_key: workspace.map(String::from),
            workspace_label: workspace.map(String::from),
            tokens: Default::default(),
            cost: 0.0,
            performance: Default::default(),
            session_count: 0,
        }
    }

    #[test]
    fn workspace_groupings_name_the_project_in_the_row_label() {
        // Both workspace groupings split rows per project, so both have to say
        // which project a row belongs to. Gating this on one of them left the
        // other rendering bare model names, which reads as the grouping having
        // done nothing at all.
        for group_by in [GroupBy::WorkspaceModel, GroupBy::WorkspaceProviderModel] {
            assert_eq!(
                model_display_name(&usage("sonnet", Some("app")), &group_by),
                "app / sonnet",
                "{group_by} must name the project it grouped by"
            );
        }
    }

    #[test]
    fn other_groupings_leave_the_row_label_as_the_model() {
        for group_by in [
            GroupBy::Model,
            GroupBy::ClientModel,
            GroupBy::ClientProviderModel,
            GroupBy::Session,
            GroupBy::ClientSession,
        ] {
            assert_eq!(
                model_display_name(&usage("sonnet", Some("app")), &group_by),
                "sonnet"
            );
        }
    }

    #[test]
    fn a_row_without_workspace_metadata_is_labelled_rather_than_dropped() {
        assert_eq!(
            workspace_label(&usage("sonnet", None)),
            "Unknown workspace",
            "an unattributed row still has to render"
        );
    }
}
