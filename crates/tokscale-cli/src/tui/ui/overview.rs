use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::bar_chart::{render_stacked_bar_chart, ModelSegment, StackedBarData};
use super::widgets::{format_tokens, get_model_color};
use crate::tui::app::App;
use tokscale_core::GroupBy;

struct ModelRowData {
    model: String,
    workspace_label: Option<String>,
    tokens_input: u64,
    tokens_output: u64,
    tokens_cache_read: u64,
    tokens_cache_write: u64,
    cost: f64,
}

fn overview_model_label(group_by: &GroupBy, model: &str, workspace_label: Option<&str>) -> String {
    if *group_by == GroupBy::WorkspaceModel {
        format!(
            "{} / {}",
            workspace_label.unwrap_or("Unknown workspace"),
            model
        )
    } else {
        model.to_string()
    }
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
    app.max_visible_items = items_per_page;

    render_chart(frame, app, chunks[0]);
    render_legend(frame, app, chunks[1]);
    render_top_models(frame, app, chunks[2], items_per_page);
}

fn render_chart(frame: &mut Frame, app: &App, area: Rect) {
    let daily = &app.data.daily;
    let group_by = app.group_by.borrow().clone();
    let mut sorted_daily: Vec<_> = daily.iter().collect();
    sorted_daily.sort_by(|a, b| a.date.cmp(&b.date));

    let data: Vec<StackedBarData> = sorted_daily
        .iter()
        .rev()
        .take(60)
        .rev()
        .map(|d| {
            let date = d.date.format("%m/%d").to_string();
            let total = d.tokens.total();

            let models: Vec<ModelSegment> = d
                .models
                .values()
                .map(|info| ModelSegment {
                    model_id: info.display_name.clone(),
                    tokens: info.tokens.total(),
                    color: get_model_color(overview_color_key(&group_by, &info.color_key)),
                })
                .collect();

            StackedBarData {
                date,
                models,
                total,
            }
        })
        .collect();

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
                overview_model_label(&group_by, &m.model, m.workspace_label.as_deref()),
                get_model_color(&m.model),
            )
        })
        .collect();

    if top_models.is_empty() {
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    for (i, (model_name, color)) in top_models.iter().enumerate() {
        let name = truncate_string(model_name, max_name_width);

        spans.push(Span::styled("●", Style::default().fg(*color)));
        spans.push(Span::raw(format!(" {}", name)));

        if i < top_models.len() - 1 {
            spans.push(Span::styled("  ·", Style::default().fg(muted_color)));
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
            workspace_label: m.workspace_label.clone(),
            tokens_input: m.tokens.input,
            tokens_output: m.tokens.output,
            tokens_cache_read: m.tokens.cache_read,
            tokens_cache_write: m.tokens.cache_write,
            cost: m.cost,
        })
        .collect();

    let title = if is_very_narrow {
        "Top Models".to_string()
    } else {
        match sort_field {
            SortField::Tokens => "Models by Tokens".to_string(),
            _ => "Models by Cost".to_string(),
        }
    };

    let title_right = if is_very_narrow {
        format_cost(total_cost)
    } else {
        format!("Total: {}", format_cost(total_cost))
    };

    let block = Block::default()
        .borders(Borders::ALL)
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
        let empty = Paragraph::new("No data available")
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
    let max_name_width = if is_narrow { 20 } else { 35 };

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

        let model_color = get_model_color(&model.model);
        let display_name =
            overview_model_label(&group_by, &model.model, model.workspace_label.as_deref());
        let name = truncate_string(&display_name, max_name_width);
        let percentage = if model.cost.is_finite() && total.is_finite() && total > 0.0 {
            (model.cost / total) * 100.0
        } else {
            0.0
        };

        let line1_area = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(Paragraph::new("").style(row_style), line1_area);

        let line1_spans = vec![
            Span::styled("●", Style::default().fg(model_color)),
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
                Span::styled(
                    format_tokens(model.tokens_input),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
                Span::styled("/", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled(
                    format_tokens(model.tokens_output),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
                Span::styled("/", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled(
                    format_tokens(model.tokens_cache_read),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
                Span::styled("/", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled(
                    format_tokens(model.tokens_cache_write),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
            ]
        } else {
            vec![
                Span::styled("  In: ", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled(
                    format_tokens(model.tokens_input),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
                Span::styled(" · Out: ", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled(
                    format_tokens(model.tokens_output),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
                Span::styled(" · CR: ", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled(
                    format_tokens(model.tokens_cache_read),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
                Span::styled(" · CW: ", Style::default().fg(Color::Rgb(102, 102, 102))),
                Span::styled(
                    format_tokens(model.tokens_cache_write),
                    Style::default().fg(Color::Rgb(170, 170, 170)),
                ),
            ]
        };

        let line2 = Line::from(line2_spans);
        let line2_para = Paragraph::new(line2).style(row_style);
        frame.render_widget(line2_para, line2_area);

        y += 1;
    }

    if models_len > items_per_page {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state = ScrollbarState::new(models_len).position(scroll_offset);

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

fn truncate_string(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else if max_chars == 1 {
        "…".to_string()
    } else {
        let head: String = s.chars().take(max_chars - 1).collect();
        format!("{}…", head)
    }
}
