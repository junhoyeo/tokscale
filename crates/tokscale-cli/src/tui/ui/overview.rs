use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::bar_chart::{render_stacked_bar_chart, ModelSegment, StackedBarData};
use super::widgets::{
    display_width, fit_workspace_label_to_width_with_marker, format_tokens,
    truncate_ellipsis_to_width_with_marker, truncate_to_width, viewport_scrollbar_state,
};
use crate::tui::app::{App, ChartGranularity};
use crate::tui::i18n::{tr, MessageKey};
use tokscale_core::GroupBy;

struct ModelRowData {
    model: String,
    color_key: String,
    provider: String,
    workspace_label: Option<String>,
    machine: Option<String>,
    tokens_input: u64,
    tokens_output: u64,
    tokens_cache_read: u64,
    tokens_cache_write: u64,
    cost: f64,
}

fn overview_model_label(
    group_by: &GroupBy,
    model: &str,
    workspace_label: Option<&str>,
    machine: Option<&str>,
) -> String {
    match group_by {
        GroupBy::WorkspaceModel => format!(
            "{} / {}",
            workspace_label.unwrap_or("Unknown workspace"),
            model
        ),
        GroupBy::MachineModel => format!(
            "{} / {}",
            machine.unwrap_or(tokscale_core::machine_label::DEFAULT_LOCAL_MACHINE_LABEL),
            model
        ),
        _ => model.to_string(),
    }
}

/// Fit a `machine / model` label into `max_cells`, keeping the model whole
/// and cutting the machine. When not even one machine cell and the marker fit
/// beside the model, the model leads and is head-cut, so the row still names
/// what it counts.
fn fit_machine_model_label(
    label: &str,
    machine: &str,
    model: &str,
    max_cells: usize,
    marker: &str,
) -> String {
    if display_width(label) <= max_cells {
        return label.to_string();
    }
    let suffix = format!(" / {model}");
    let machine_cells = max_cells.saturating_sub(display_width(&suffix));
    if machine_cells > display_width(marker) {
        return format!(
            "{}{suffix}",
            truncate_ellipsis_to_width_with_marker(machine, machine_cells, marker)
        );
    }
    truncate_ellipsis_to_width_with_marker(model, max_cells, marker)
}

fn overview_color_key<'a>(group_by: &GroupBy, model: &'a str) -> &'a str {
    if *group_by == GroupBy::WorkspaceModel {
        model
            .rsplit_once(" / ")
            .map(|(_, base_model)| base_model)
            .unwrap_or(model)
    } else {
        model
    }
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    // Pre-fill entire overview area with theme background so that chart and
    // legend cells (which only set fg via direct buffer writes) don't fall
    // through to the terminal's default background color.
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.background)),
        area,
    );

    let safe_height = area.height.max(12) as usize;
    let chart_height = (safe_height as f64 * 0.35).floor().max(5.0) as u16;
    let legend_height = 1u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(chart_height),
            Constraint::Length(legend_height),
            Constraint::Min(0),
        ])
        .split(area);

    let list_area_height = chunks[2].height.saturating_sub(2);
    let items_per_page = ((list_area_height / 2) as usize).max(1);
    app.set_max_visible_items(items_per_page);

    render_chart(frame, app, chunks[0]);
    render_legend(frame, app, chunks[1]);
    render_top_models(frame, app, chunks[2], items_per_page);
}

fn render_chart(frame: &mut Frame, app: &App, area: Rect) {
    let group_by = app.group_by.borrow().clone();

    let data: Vec<StackedBarData> = match app.chart_granularity {
        ChartGranularity::Daily => app
            .data
            .daily
            .iter()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|d| {
                let mut models_by_key = std::collections::BTreeMap::<String, ModelSegment>::new();
                for source_info in d.source_breakdown.values() {
                    for (key, info) in &source_info.models {
                        let entry =
                            models_by_key
                                .entry(key.clone())
                                .or_insert_with(|| ModelSegment {
                                    model_id: info.display_name.clone(),
                                    tokens: 0,
                                    color: app.model_color_for(
                                        &info.provider,
                                        overview_color_key(&group_by, &info.color_key),
                                    ),
                                });
                        entry.tokens = entry.tokens.saturating_add(info.tokens.total());
                    }
                }
                let models: Vec<ModelSegment> = models_by_key.into_values().collect();

                StackedBarData {
                    date: d.date.format("%m/%d").to_string(),
                    models,
                    total: d.tokens.total(),
                }
            })
            .collect(),
        ChartGranularity::Hourly => app
            .data
            .hourly
            .iter()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|h| {
                let models: Vec<ModelSegment> = h
                    .models
                    .values()
                    .map(|info| ModelSegment {
                        model_id: info.display_name.clone(),
                        tokens: info.tokens.total(),
                        color: app.model_color_for(&info.provider, &info.color_key),
                    })
                    .collect();

                StackedBarData {
                    date: h.datetime.format("%d %H:%M").to_string(),
                    models,
                    total: h.tokens.total(),
                }
            })
            .collect(),
    };

    render_stacked_bar_chart(frame, app, area, &data);
}

fn render_legend(frame: &mut Frame, app: &App, area: Rect) {
    let legend_limit = if app.is_narrow() { 3 } else { 5 };
    let max_name_width = if app.is_narrow() { 12 } else { 18 };
    let muted_color = app.theme.muted;
    let group_by = app.group_by.borrow().clone();

    let top_models: Vec<(String, Color)> = app
        .get_sorted_models()
        .iter()
        .take(legend_limit)
        .map(|m| {
            (
                overview_model_label(
                    &group_by,
                    &m.model,
                    m.workspace_label.as_deref(),
                    m.machine.as_deref(),
                ),
                app.model_color_for(&m.provider, &m.color_key),
            )
        })
        .collect();

    if top_models.is_empty() {
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    for (i, (model_name, color)) in top_models.iter().enumerate() {
        let name =
            truncate_ellipsis_to_width_with_marker(model_name, max_name_width, app.glyphs.ellipsis);

        spans.push(Span::styled(
            app.glyphs.legend_dot,
            Style::default().fg(*color),
        ));
        spans.push(Span::raw(format!(" {}", name)));

        if i < top_models.len() - 1 {
            spans.push(Span::styled(
                format!("  {}", app.glyphs.compact_separator),
                Style::default().fg(muted_color),
            ));
        }
    }

    let legend_line = Line::from(spans);
    let paragraph = Paragraph::new(legend_line);
    frame.render_widget(paragraph, area);
}

fn render_top_models(frame: &mut Frame, app: &mut App, area: Rect, items_per_page: usize) {
    use super::widgets::format_cost;
    use crate::tui::app::SortField;

    let theme_border = app.theme.border;
    let theme_accent = app.theme.accent;
    let theme_background = app.theme.background;
    let theme_muted = app.theme.muted;
    let theme_foreground = app.theme.foreground;
    let theme_selection = app.theme.selection;
    let secondary_text_style = app.theme.secondary_text_style();
    let subtle_text_style = app.theme.subtle_text_style();
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let sort_field = app.sort_field;
    let total_cost = app.data.total_cost;
    let group_by = app.group_by.borrow().clone();

    let models_data: Vec<ModelRowData> = app
        .get_sorted_models()
        .iter()
        .map(|m| ModelRowData {
            model: m.model.clone(),
            color_key: m.color_key.clone(),
            provider: m.provider.clone(),
            workspace_label: m.workspace_label.clone(),
            machine: m.machine.clone(),
            tokens_input: m.tokens.input,
            tokens_output: m.tokens.output,
            tokens_cache_read: m.tokens.cache_read,
            tokens_cache_write: m.tokens.cache_write,
            cost: m.cost,
        })
        .collect();

    let lang = app.settings.tui_language;
    let title = if is_very_narrow {
        tr(lang, MessageKey::OverviewTopModels).to_string()
    } else {
        match sort_field {
            SortField::Tokens => tr(lang, MessageKey::OverviewModelsByTokens).to_string(),
            _ => tr(lang, MessageKey::OverviewModelsByCost).to_string(),
        }
    };

    let title_right = if is_very_narrow {
        format_cost(total_cost)
    } else {
        format!(
            "{}{}",
            tr(lang, MessageKey::OverviewTotal),
            format_cost(total_cost)
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(app.glyphs.border)
        .border_style(Style::default().fg(theme_border))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(theme_accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_top(
            Line::from(Span::styled(
                format!(" {} ", title_right),
                Style::default().fg(Color::Green),
            ))
            .right_aligned(),
        )
        .style(Style::default().bg(theme_background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if models_data.is_empty() {
        let empty = Paragraph::new(tr(lang, MessageKey::EmptyNoDataAvailable))
            .style(Style::default().fg(theme_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let total = models_data
        .iter()
        .map(|m| if m.cost.is_finite() { m.cost } else { 0.0 })
        .sum::<f64>()
        .max(0.01);
    let models_len = models_data.len();
    let start = scroll_offset.min(models_len);
    let end = (start + items_per_page).min(models_len);
    // `workspace / model` is roughly twice the text of a bare model name, and
    // the label's distinguishing part (repo, worktree) sits at the far end, so a
    // 35-cell cap truncated every row down to the same shared path prefix. The
    // panel is full-width here, so spend it rather than clipping to a constant.
    let max_name_width = if is_narrow {
        20
    } else if matches!(group_by, GroupBy::WorkspaceModel | GroupBy::MachineModel) {
        // Leave room for the "● " marker and the trailing " (12.3%)".
        inner.width.saturating_sub(12).max(35) as usize
    } else {
        35
    };

    if start >= models_len {
        return;
    }

    let mut y = inner.y;
    for (i, model) in models_data[start..end].iter().enumerate() {
        if y + 1 >= inner.y + inner.height {
            break;
        }

        let idx = i + start;
        let is_selected = idx == selected_index;
        let row_style = if is_selected {
            Style::default().bg(theme_selection).fg(theme_foreground)
        } else {
            Style::default()
        };

        let model_color = app.model_color_for(&model.provider, &model.color_key);
        let display_name = overview_model_label(
            &group_by,
            &model.model,
            model.workspace_label.as_deref(),
            model.machine.as_deref(),
        );
        // Measured in cells, not code points: `max_name_width` is derived from
        // `inner.width`, and a workspace path can hold full-width graphemes (a CJK
        // directory name), which a char count calls short at twice the cells it
        // occupies -- overflowing the row and pushing the cost percentage off it.
        let name = if group_by == GroupBy::WorkspaceModel {
            // `workspace / model` is identified by both ends, so a head cut on a
            // narrow panel leaves the shared path prefix and drops the model
            // entirely. Same reasoning as the Workspace column in the table.
            fit_workspace_label_to_width_with_marker(
                &display_name,
                max_name_width,
                app.glyphs.ellipsis,
            )
        } else if group_by == GroupBy::MachineModel {
            // `machine / model` is read by its model: a head cut keeps the
            // machine and drops the model, so every model on one machine
            // rendered as the same row. The machine part gives way instead.
            fit_machine_model_label(
                &display_name,
                model
                    .machine
                    .as_deref()
                    .unwrap_or(tokscale_core::machine_label::DEFAULT_LOCAL_MACHINE_LABEL),
                &model.model,
                max_name_width,
                app.glyphs.ellipsis,
            )
        } else {
            truncate_to_width(&display_name, max_name_width)
        };
        let percentage = if model.cost.is_finite() && total.is_finite() && total > 0.0 {
            (model.cost / total) * 100.0
        } else {
            0.0
        };

        let line1_area = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(Paragraph::new("").style(row_style), line1_area);

        let line1_spans = vec![
            Span::styled(app.glyphs.legend_dot, Style::default().fg(model_color)),
            Span::styled(
                format!(" {}", name),
                Style::default()
                    .fg(if is_selected {
                        theme_foreground
                    } else {
                        model_color
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({:.1}%)", percentage),
                Style::default().fg(theme_muted),
            ),
        ];
        let line1 = Line::from(line1_spans);
        let line1_para = Paragraph::new(line1).style(row_style);
        frame.render_widget(line1_para, line1_area);

        y += 1;
        if y >= inner.y + inner.height {
            break;
        }

        let line2_area = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(Paragraph::new("").style(row_style), line2_area);

        let line2_spans = if is_narrow {
            vec![
                Span::raw("  "),
                Span::styled(format_tokens(model.tokens_input), secondary_text_style),
                Span::styled("/", subtle_text_style),
                Span::styled(format_tokens(model.tokens_output), secondary_text_style),
                Span::styled("/", subtle_text_style),
                Span::styled(format_tokens(model.tokens_cache_read), secondary_text_style),
                Span::styled("/", subtle_text_style),
                Span::styled(
                    format_tokens(model.tokens_cache_write),
                    secondary_text_style,
                ),
            ]
        } else {
            vec![
                Span::styled("  In: ", subtle_text_style),
                Span::styled(format_tokens(model.tokens_input), secondary_text_style),
                Span::styled(
                    format!(" {} Out: ", app.glyphs.compact_separator),
                    subtle_text_style,
                ),
                Span::styled(format_tokens(model.tokens_output), secondary_text_style),
                Span::styled(
                    format!(" {} CR: ", app.glyphs.compact_separator),
                    subtle_text_style,
                ),
                Span::styled(format_tokens(model.tokens_cache_read), secondary_text_style),
                Span::styled(
                    format!(" {} CW: ", app.glyphs.compact_separator),
                    subtle_text_style,
                ),
                Span::styled(
                    format_tokens(model.tokens_cache_write),
                    secondary_text_style,
                ),
            ]
        };

        let line2 = Line::from(line2_spans);
        let line2_para = Paragraph::new(line2).style(row_style);
        frame.render_widget(line2_para, line2_area);

        y += 1;
    }

    if models_len > items_per_page {
        let scrollbar = app.glyphs.scrollbar();

        let mut scrollbar_state =
            viewport_scrollbar_state(models_len, scroll_offset, items_per_page);

        frame.render_stateful_widget(
            scrollbar,
            inner.inner(Margin {
                horizontal: 0,
                vertical: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;
    use crate::tui::data::ModelUsage;
    use crate::tui::{Tab, TuiConfig};

    const MODEL: &str = "claude-sonnet-4-5-20250929";

    fn machine_row(machine: &str) -> ModelUsage {
        ModelUsage {
            model: MODEL.to_string(),
            color_key: MODEL.to_string(),
            provider: "anthropic".to_string(),
            client: "claude".to_string(),
            workspace_key: None,
            workspace_label: None,
            machine: Some(machine.to_string()),
            tokens: Default::default(),
            cost: 1.0,
            performance: Default::default(),
            session_count: 1,
            group_key: format!("{machine}:{MODEL}"),
            daily: Vec::new(),
        }
    }

    /// Render the Overview tab under `machine,model` and return its rows.
    fn render_overview_rows(machine: &str, width: u16) -> Vec<String> {
        let mut app = App::new_with_cached_data(TuiConfig::default(), None).unwrap();
        app.current_tab = Tab::Overview;
        *app.group_by.borrow_mut() = GroupBy::MachineModel;
        app.data.models = vec![machine_row(machine)];
        app.terminal_width = width;
        app.terminal_height = 40;

        let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
        terminal
            .draw(|frame| super::render(frame, &mut app, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    /// A long machine label must not push the model id off the row. Head
    /// truncation at a fixed 35 cells kept only the machine, so every model
    /// on that machine rendered as the same `machine...` row.
    #[test]
    fn top_models_keeps_the_model_id_whole_after_a_long_machine_label() {
        let machine = "steven-macbook-pro-2024-studio-mirror";
        let rows = render_overview_rows(machine, 120);
        let label = format!("{machine} / {MODEL}");
        assert!(
            rows.iter().any(|row| row.contains(&label)),
            "the full `machine / model` label must fit a 120-cell Overview:\n{}",
            rows.join("\n")
        );
    }

    /// When the label is wider than the row, the machine part gives way and
    /// the model id stays whole.
    #[test]
    fn top_models_truncates_the_machine_not_the_model() {
        let machine = "steven-macbook-pro-2024-studio-mirror-with-a-very-long-hostname-suffix";
        let rows = render_overview_rows(machine, 90);
        let row = rows
            .iter()
            .find(|row| row.contains(&format!(" / {MODEL}")))
            .unwrap_or_else(|| {
                panic!(
                    "the model id must stay whole after the machine:\n{}",
                    rows.join("\n")
                )
            });
        assert!(row.contains("steven-macbook"), "{row}");
        assert!(!row.contains(machine), "{row}");
        assert!(row.contains("(100.0%)"), "{row}");
    }

    #[test]
    fn fit_machine_model_label_prefers_the_model() {
        let label = |machine: &str, cells: usize| {
            fit_machine_model_label(&format!("{machine} / {MODEL}"), machine, MODEL, cells, "~")
        };
        // Fits: unchanged.
        assert_eq!(label("blade", 60), format!("blade / {MODEL}"));
        // Too wide: only the machine is cut.
        let fitted = label("steven-macbook-pro", 40);
        assert_eq!(fitted, format!("steven-mac~ / {MODEL}"));
        assert_eq!(display_width(&fitted), 40);
        // No room for any machine text: the model still leads the cut.
        let fitted = label("steven-macbook-pro", 20);
        assert_eq!(display_width(&fitted), 20);
        assert!(fitted.starts_with("claude-sonnet"), "{fitted}");
    }
}
