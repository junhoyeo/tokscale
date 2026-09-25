use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::widgets::{
    ambient_stable_scrollbar, fit_workspace_label_to_width, format_cache_hit_rate, format_cost,
    format_cost_per_million, format_ms_per_1k, format_tokens, get_client_display_name,
    get_provider_display_name, total_tokens_cell, truncate_text, truncate_to_width,
    viewport_scrollbar_state, AMBIENT_STABLE_BORDER_SET,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};
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

/// The Models table's header labels for a layout, in display order.
///
/// A function rather than an inline `vec!` so
/// `every_language_renders_its_full_header_once_the_layout_fits` asserts against
/// the labels the renderer actually writes. A test that restates the label set
/// passes while the renderer uses a different one, which is how the Korean
/// `메시지` clip in `sessions.rs` survived a header test that already existed.
///
/// `GroupBy::WorkspaceModel` has its own wide layout: it gains a Workspace
/// column and drops Cache✕, so the two wide branches are not interchangeable.
fn header_labels(
    lang: TuiLanguage,
    is_narrow: bool,
    is_very_narrow: bool,
    group_by: &GroupBy,
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
            tr(lang, MessageKey::ColTokens),
            tr(lang, MessageKey::ColCost),
        ];
    }
    if *group_by == GroupBy::WorkspaceModel {
        return vec![
            tr(lang, MessageKey::ColRank),
            tr(lang, MessageKey::ColWorkspace),
            tr(lang, MessageKey::ColModel),
            tr(lang, MessageKey::ColProvider),
            tr(lang, MessageKey::ColSource),
            tr(lang, MessageKey::ColInput),
            tr(lang, MessageKey::ColOutput),
            tr(lang, MessageKey::ColCacheRead),
            tr(lang, MessageKey::ColCacheWrite),
            tr(lang, MessageKey::ColTotal),
            tr(lang, MessageKey::ColMsPer1k),
            tr(lang, MessageKey::ColCost),
            tr(lang, MessageKey::ColCostPer1M),
        ];
    }
    vec![
        tr(lang, MessageKey::ColRank),
        tr(lang, MessageKey::ColModel),
        tr(lang, MessageKey::ColProvider),
        tr(lang, MessageKey::ColSource),
        tr(lang, MessageKey::ColInput),
        tr(lang, MessageKey::ColOutput),
        tr(lang, MessageKey::ColCacheRead),
        tr(lang, MessageKey::ColCacheWrite),
        tr(lang, MessageKey::ColCacheHit),
        tr(lang, MessageKey::ColTotal),
        tr(lang, MessageKey::ColMsPer1k),
        tr(lang, MessageKey::ColCost),
        tr(lang, MessageKey::ColCostPer1M),
    ]
}

/// The Models table's column widths, index-aligned with [`header_labels`].
///
/// A function rather than an inline `vec!` so
/// `no_header_overflows_its_budget_in_any_language` checks the widths the
/// renderer actually lays out with, not a copy of them. The narrow layouts are
/// percentage-based and have no declared budget; the wide ones are `Length`
/// (plus one flexible Model or Workspace column), so a header longer than its
/// `Length` is a clip the arithmetic can name.
///
/// `inner_width` only matters for the workspace layout, whose Workspace column
/// widens once the row has surplus cells — see [`workspace_column_width`].
fn header_widths(
    is_narrow: bool,
    is_very_narrow: bool,
    group_by: &GroupBy,
    inner_width: u16,
) -> Vec<Constraint> {
    if is_very_narrow {
        return vec![Constraint::Percentage(70), Constraint::Percentage(30)];
    }
    if is_narrow {
        return vec![
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ];
    }
    if *group_by == GroupBy::WorkspaceModel {
        // Same shape as the default layout, with a wider Workspace column once the
        // row has surplus cells to give it. Model keeps the flexible slot so the
        // workspace column can never widen at Cost's expense. Shared with
        // `workspace_column_granted_width` so the label is truncated to the width
        // this very layout hands out.
        return workspace_column_constraints(inner_width);
    }
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
    if *group_by == GroupBy::WorkspaceModel {
        format!("{} / {}", workspace_label(model), model.model)
    } else {
        model.model.clone()
    }
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            format!(" {} ", tr(lang, MessageKey::TabModels)),
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

    let lang = app.settings.tui_language;
    let models = app.get_sorted_models();
    if models.is_empty() {
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoUsageData))
            .style(Style::default().fg(theme_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let header_cells = header_labels(lang, is_narrow, is_very_narrow, &group_by);

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
            } else if group_by == GroupBy::WorkspaceModel {
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
                        .style(app.theme.hint_key_style()),
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
                    .style(app.theme.count_style()),
                    total_tokens_cell(model.tokens.total(), &app.theme),
                    Cell::from(format_ms_per_1k(model.performance.ms_per_1k_tokens))
                        .style(app.theme.hint_key_style()),
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

    let widths = header_widths(is_narrow, is_very_narrow, &group_by, inner.width);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if models_len > visible_height {
        let scrollbar = ambient_stable_scrollbar();

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
    use crate::tui::ui::header_budget::{assert_header_layout_fits, assert_headers_render_in_full};

    /// Column widths the solver grants the workspace layout. Calls the production
    /// helper rather than restating the constraints, so a change to the layout
    /// cannot leave the test asserting against a stale copy of it.
    fn workspace_layout_at(total: u16) -> Vec<u16> {
        workspace_table_widths(total)
    }

    /// Every text cell truncates to the width the table hands out, never to a width
    /// it merely asked for.
    ///
    /// Both columns hit this. Between 174 and 192 cells the row could not satisfy a
    /// 44-cell Workspace while Model held its `Min(20)`, so the solver granted 25..43
    /// while the formatter cut to 44. Model had the mirror-image bug from the other
    /// direction: a constant 24-cell budget against a floating `Min` that the solver
    /// sets to 20 on a narrow row and 77 on a wide one — over-truncating at 97 widths
    /// and silently clipping at the rest. Asserting per column across every width is
    /// what makes a third cell unable to reintroduce it.
    #[test]
    fn workspace_column_request_matches_what_it_is_granted() {
        for total in 78u16..=400 {
            for (index, name) in [
                (WORKSPACE_COL_WORKSPACE, "Workspace"),
                (WORKSPACE_COL_MODEL, "Model"),
            ] {
                let solved = workspace_layout_at(total);
                assert_eq!(
                    workspace_granted_width(&solved, index),
                    solved[index] as usize,
                    "at {total} cols the {name} truncation width disagrees with its allocation"
                );
            }
        }

        // And the wide request is genuinely satisfiable at its threshold, so the
        // extra width is real rather than immediately clawed back.
        let widths = workspace_layout_at(WORKSPACE_SURPLUS_MIN_WIDTH);
        assert_eq!(widths[WORKSPACE_COL_WORKSPACE], WORKSPACE_COLUMN_WIDE_WIDTH);
        assert!(
            widths[WORKSPACE_COL_MODEL] >= 20,
            "Model fell to {} cells at the switch width",
            widths[WORKSPACE_COL_MODEL]
        );
    }

    /// The workspace layout as it stood before this change: Workspace pinned to 18
    /// at every width. It is the correct bar to clear — the default (non-workspace)
    /// layout has one fewer wide column, so it is not comparable, and holding this
    /// layout to it would flag a 1-cell difference at width 84 that predates this
    /// change and is inherent to rendering a Workspace column at all.
    fn previous_workspace_layout_at(total: u16) -> Vec<u16> {
        use ratatui::layout::Layout;
        let widths = vec![
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
        ];
        Layout::horizontal(widths)
            .spacing(1)
            .split(Rect::new(0, 0, total, 1))
            .iter()
            .map(|r| r.width)
            .collect()
    }

    /// Widening Workspace must never come out of Cost, Cost/1M, or Model.
    ///
    /// ratatui's solver shrinks whatever it must to fit the row, so a workspace
    /// column that grows on a row with no surplus silently pays for itself by
    /// clipping a dollar figure or collapsing the model name — which relocates the
    /// unreadable-truncation bug this change exists to fix rather than fixing it.
    ///
    /// Sweeps EVERY width in 78..=400 on purpose. The first version of this test
    /// sampled six widths and passed while 18 widths in the same range were
    /// clipping Cost by a cell; a solver-driven layout cannot be spot-checked.
    #[test]
    fn workspace_layout_never_starves_its_neighbors() {
        for total in 78u16..=400 {
            let ws = workspace_layout_at(total);
            let base = previous_workspace_layout_at(total);

            // Cost (11) and Cost/1M (12) must be no narrower than before.
            for (idx, name) in [(11usize, "Cost"), (12usize, "Cost/1M")] {
                assert!(
                    ws[idx] >= base[idx],
                    "at {total} cols the Workspace column cost {name} {} cell(s)",
                    base[idx] - ws[idx]
                );
            }

            // Model keeps the flexible slot, so the wide Workspace column is paid
            // for out of Model's SURPLUS -- never out of its Min(20) floor, and so
            // never out of a column carrying a number.
            assert!(
                ws[2] >= 20 || ws[2] == base[2],
                "at {total} cols Model fell below its floor: {} cells (was {})",
                ws[2],
                base[2]
            );
        }
    }

    /// The column has to actually get wider somewhere, or the fix does nothing.
    #[test]
    fn workspace_column_widens_once_the_row_has_surplus() {
        assert_eq!(
            workspace_column_width(WORKSPACE_SURPLUS_MIN_WIDTH - 1),
            WORKSPACE_COLUMN_BASE_WIDTH
        );
        assert_eq!(
            workspace_column_width(WORKSPACE_SURPLUS_MIN_WIDTH),
            WORKSPACE_COLUMN_WIDE_WIDTH
        );
        // `repo ⑃ worktree` for the reported case fits at the wide size.
        assert!(
            WORKSPACE_COLUMN_WIDE_WIDTH as usize
                >= "ea-world-service ⑃ nicole-25-20".chars().count()
        );
    }

    /// The Workspace column's granted width at the three terminal sizes the
    /// column actually takes: 18 on anything below the surplus threshold, and 44
    /// once above it.
    fn workspace_cell_width(total: u16) -> usize {
        workspace_granted_width(&workspace_table_widths(total), WORKSPACE_COL_WORKSPACE)
    }

    /// Labels drawn from the shapes the aggregator actually produces: sibling
    /// worktrees whose names differ only in their last characters, six worktrees
    /// of one repo, and parent-qualified repo names.
    fn colliding_label_fixture() -> Vec<String> {
        let mut labels = vec![
            "tokscale-2 ⑃ wf_2429b20d-2d5-1".to_string(),
            "tokscale-2 ⑃ wf_2429b20d-2d5-10".to_string(),
            "tokscale-2 ⑃ wf_aacbce6c-c09-1".to_string(),
            "tokscale-2 ⑃ wf_aacbce6c-c09-2".to_string(),
            "junhoyeo/tokscale ⑃ pr1105-dedupe-p1".to_string(),
            "junhoyeo/tokscale ⑃ pr1105-dedupe-p1-target".to_string(),
            "swebench-matplotlib__matplotlib-25775-dven5vd8/matplotlib".to_string(),
            "swebench-matplotlib__matplotlib-25775-8hddztzg/matplotlib".to_string(),
            "swebench-matplotlib__matplotlib-24870-_z_ymmel/matplotlib".to_string(),
        ];
        labels.extend((1..=6).map(|n| format!("tokscale ⑃ worker-{n}")));
        labels
    }

    /// The regression this fixes: the labels are unique STRINGS, but the row the
    /// user sees is the truncated label, and a head-first cut rendered distinct
    /// worktrees identically at the width the column is granted on an ordinary
    /// terminal. Asserted at 18, 44 and 60 cells — the base column width, the
    /// wide width, and a width in between — against the same head cut the column
    /// used before, so the test fails if the elision is reverted.
    #[test]
    fn workspace_cells_render_distinctly_at_the_widths_the_column_gets() {
        let labels = colliding_label_fixture();
        for cells in [18usize, 44, 60] {
            let rendered: std::collections::HashSet<String> = labels
                .iter()
                .map(|label| fit_workspace_label_to_width(label, cells))
                .collect();
            assert_eq!(
                rendered.len(),
                labels.len(),
                "{cells}-cell rows collapsed onto each other: {rendered:?}"
            );
        }

        // 18 and 44 are widths the layout really hands this column out, so the
        // sizes above are ones a terminal produces rather than ones picked to
        // make the numbers look good. 60 is measured too, as headroom for a
        // future wider column.
        for cells in [18u16, 44] {
            assert!(
                (78u16..=400).any(|total| workspace_cell_width(total) == cells as usize),
                "no terminal width grants the Workspace column {cells} cells"
            );
        }

        // The old head cut is what produced the collisions.
        let head: std::collections::HashSet<String> = labels
            .iter()
            .map(|label| truncate_to_width(label, 18))
            .collect();
        assert!(
            head.len() < labels.len(),
            "fixture no longer reproduces the head-cut collision: {head:?}"
        );
    }

    /// Pins the painted screen, not the helpers behind it. The reported symptom
    /// was purely visual -- readable labels that still rendered as a truncated
    /// shared prefix -- so only the rendered buffer can prove it is fixed.
    #[test]
    fn workspace_grouping_paints_full_repo_and_worktree_labels() {
        use crate::tui::app::{Tab, TuiConfig};
        use crate::tui::data::ModelUsage;
        use ratatui::{backend::TestBackend, Terminal};

        let mut app = App::new_with_cached_data(
            TuiConfig {
                theme: "blue".to_string(),
                refresh: 0,
                sessions_path: None,
                clients: None,
                since: None,
                until: None,
                year: None,
                initial_tab: None,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        app.settings.tui_language = crate::tui::i18n::TuiLanguage::En;

        let width = 200u16;
        app.terminal_width = width;
        app.current_tab = Tab::Models;
        *app.group_by.borrow_mut() = GroupBy::WorkspaceModel;
        app.data.models = vec![ModelUsage {
            model: "claude-opus-5".to_string(),
            color_key: "claude-opus-5".to_string(),
            provider: "anthropic".to_string(),
            client: "claude".to_string(),
            workspace_key: Some("/Users/z/devpro/ea/ea-world-service".to_string()),
            // The label the aggregator now produces for a worktree row.
            workspace_label: Some("ea-world-service ⑃ nicole-25-20".to_string()),
            tokens: Default::default(),
            cost: 5367.48,
            performance: Default::default(),
            session_count: 1,
        }];

        let backend = TestBackend::new(width, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, Rect::new(0, 0, width, 8)))
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            screen.contains("ea-world-service ⑃ nicole-25-20"),
            "workspace label must be painted in full, got:\n{screen}"
        );
        // The old behavior: a right-truncated label ending in an ellipsis.
        assert!(
            !screen.contains("ea-world-s…") && !screen.contains("ea-world-s..."),
            "label must not be truncated at this width, got:\n{screen}"
        );
    }

    /// Width at and above which an all-`Length` layout (plus one flexible column
    /// at its floor) gets every cell it asked for, block borders included.
    ///
    /// `fixed_layout_width` returns `None` for a layout containing a `Min`, and
    /// both wide Models layouts have one, so the `Min`'s floor is summed the way
    /// `daily.rs`'s detail test does.
    fn layout_fitting_width(widths: &[Constraint]) -> u16 {
        let fixed: u16 = widths
            .iter()
            .map(|constraint| match constraint {
                Constraint::Length(cells) => *cells,
                // The sole `Min`: Model's floor in both wide layouts.
                Constraint::Min(cells) => *cells,
                other => unreachable!("unexpected constraint {other:?}"),
            })
            .sum();
        // + one separator between each pair, + 2 for the block borders.
        fixed + widths.len().saturating_sub(1) as u16 + 2
    }

    fn make_table_app(width: u16, group_by: GroupBy) -> App {
        use crate::tui::app::{Tab, TuiConfig};
        use crate::tui::data::ModelUsage;

        let mut app = App::new_with_cached_data(
            TuiConfig {
                theme: "blue".to_string(),
                refresh: 0,
                sessions_path: None,
                clients: None,
                since: None,
                until: None,
                year: None,
                initial_tab: None,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        app.settings.tui_language = TuiLanguage::En;
        app.terminal_width = width;
        app.current_tab = Tab::Models;
        app.sort_field = SortField::Tokens;
        app.sort_direction = SortDirection::Descending;
        *app.group_by.borrow_mut() = group_by;
        app.data.models = vec![ModelUsage {
            model: "claude-opus-5".to_string(),
            color_key: "claude-opus-5".to_string(),
            provider: "anthropic".to_string(),
            client: "claude".to_string(),
            workspace_key: Some("/Users/z/tokscale".to_string()),
            workspace_label: Some("tokscale ⑃ main".to_string()),
            tokens: Default::default(),
            cost: 12.5,
            performance: Default::default(),
            session_count: 1,
        }];
        app
    }

    /// The rendered header row of the Models table.
    fn header_for(lang: TuiLanguage, width: u16, group_by: GroupBy) -> String {
        use ratatui::{backend::TestBackend, Terminal};

        let mut app = make_table_app(width, group_by);
        app.settings.tui_language = lang;
        let backend = TestBackend::new(width, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, Rect::new(0, 0, width, 10)))
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
            .get(1)
            .cloned()
            .unwrap_or_default()
    }

    /// Which wide-layout columns carry a sort arrow, index-aligned with
    /// `header_labels`. Read straight off the `sort_indicator` match in `render`:
    /// in the wide branch (`!is_narrow`) only `9 => Tokens` and `11 => Cost`
    /// match. Both wide layouts put Total at 9 and Cost at 11 — the workspace
    /// layout gains Workspace at 1 and drops Cache✕, which cancel out — so one
    /// flag vector serves both.
    const WIDE_SORTABLE: [bool; 13] = [
        false, false, false, false, false, false, false, false, false, true, false, true, false,
    ];

    /// No header label exceeds the `Constraint::Length` its own layout declares,
    /// in any language, in either wide grouping.
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
    /// renderer uses something else. `GroupBy::WorkspaceModel` is checked
    /// separately because it has its own layout, and at both Workspace column
    /// widths because that column widens once the row has surplus cells.
    #[test]
    fn no_header_overflows_its_budget_in_any_language() {
        for lang in TuiLanguage::ALL {
            assert_header_layout_fits(
                "models/wide",
                lang,
                &header_labels(lang, false, false, &GroupBy::Model),
                &header_widths(false, false, &GroupBy::Model, 200),
                &WIDE_SORTABLE,
            );
            // Below and above the surplus threshold, i.e. Workspace at 18 cells
            // and at 44.
            for inner in [WORKSPACE_SURPLUS_MIN_WIDTH - 1, WORKSPACE_SURPLUS_MIN_WIDTH] {
                assert_header_layout_fits(
                    &format!("models/wide-workspace(inner={inner})"),
                    lang,
                    &header_labels(lang, false, false, &GroupBy::WorkspaceModel),
                    &header_widths(false, false, &GroupBy::WorkspaceModel, inner),
                    &WIDE_SORTABLE,
                );
            }
        }
    }

    /// At and above the width its own constraints add up to, each wide layout gets
    /// every cell it asked for, so every header must render in full — in every
    /// language, in both groupings. Below that ratatui shrinks every column and
    /// clips English headers too, which is the pre-existing #964-class over-ask
    /// this tab never solved, not a localization defect.
    #[test]
    fn every_language_renders_its_full_header_once_the_layout_fits() {
        for group_by in [GroupBy::Model, GroupBy::WorkspaceModel] {
            // Measured at a width above the Workspace surplus threshold, which is
            // the wider of the two workspace layouts and therefore the binding
            // one; the default layout does not vary with width at all.
            let fitting = layout_fitting_width(&header_widths(false, false, &group_by, 240));
            // 240 rather than 200: the wide Workspace column alone needs 195, so
            // 200 leaves no room for the +20 step to mean anything.
            for width in [fitting, fitting + 20, 240] {
                // The layout solved at this very width really does fit it, so the
                // claim below is about translation and not about shrinking.
                assert!(
                    width
                        >= layout_fitting_width(&header_widths(false, false, &group_by, width - 2)),
                    "{width} cols does not fit the {group_by:?} layout"
                );
                for lang in TuiLanguage::ALL {
                    assert_headers_render_in_full(
                        &format!("models(width={width},group={group_by:?})"),
                        lang,
                        &header_for(lang, width, group_by.clone()),
                        &header_labels(lang, false, false, &group_by),
                    );
                }
            }
        }
    }
}
