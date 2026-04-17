use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::widgets::{format_cost, format_tokens, get_client_color, get_client_display_name};
use crate::tui::app::{App, ClickAction};

const CELL_WIDTH: u16 = 2;
const MONTH_LABELS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAY_LABELS: &[&str] = &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let has_selected_cell = app.selected_graph_cell.is_some();
    let stats_compact_h: u16 = 8;
    let min_breakdown_h: u16 = 6;
    let min_graph_h: u16 = 12;
    let sufficient_for_both = area.height >= min_graph_h + stats_compact_h + min_breakdown_h;

    if has_selected_cell && sufficient_for_both {
        // Three-zone layout: graph + compact stats + breakdown
        let non_stats = area.height.saturating_sub(stats_compact_h);
        let surplus = non_stats.saturating_sub(min_graph_h + min_breakdown_h);
        let graph_h = min_graph_h + (surplus * 3 / 5); // 60% of surplus to graph
        let breakdown_h = non_stats.saturating_sub(graph_h); // 40% to breakdown

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(graph_h),
                Constraint::Length(stats_compact_h),
                Constraint::Length(breakdown_h),
            ])
            .split(area);

        render_graph(frame, app, chunks[0]);
        render_stats_panel(frame, app, chunks[1]);
        render_breakdown_panel(frame, app, chunks[2]);
    } else if has_selected_cell {
        // Not enough room for both: graph + breakdown only
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(12)])
            .split(area);

        render_graph(frame, app, chunks[0]);
        render_breakdown_panel(frame, app, chunks[1]);
    } else {
        // No cell selected: graph + full stats
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(12)])
            .split(area);

        render_graph(frame, app, chunks[0]);
        render_stats_panel(frame, app, chunks[1]);
    }
}

fn render_graph(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme_border = app.theme.border;
    let theme_accent = app.theme.accent;
    let theme_background = app.theme.background;
    let theme_muted = app.theme.muted;
    let theme_colors = app.theme.colors;
    let selected_cell = app.selected_graph_cell;
    let is_narrow = app.is_narrow();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_border))
        .title(Span::styled(
            " Contribution Graph (52 weeks) ",
            Style::default()
                .fg(theme_accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(theme_background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let graph = match &app.data.graph {
        Some(g) => g.clone(),
        None => return,
    };

    let label_width = if is_narrow { 2u16 } else { 4u16 };
    let graph_start_x = inner.x + label_width;
    let graph_start_y = inner.y + 2;

    for (day_idx, label) in DAY_LABELS.iter().enumerate() {
        if day_idx % 2 == 1 {
            let y = graph_start_y + day_idx as u16;
            if y < inner.y + inner.height {
                let display_label = if is_narrow { "" } else { *label };
                let text = Paragraph::new(display_label).style(Style::default().fg(theme_muted));
                frame.render_widget(text, Rect::new(inner.x, y, label_width, 1));
            }
        }
    }

    let max_weeks = (inner.width.saturating_sub(label_width) / CELL_WIDTH) as usize;
    let weeks_to_show = graph.weeks.len().min(max_weeks);
    let start_week = graph.weeks.len().saturating_sub(weeks_to_show);

    let intensity_color = |intensity: f64| -> Color {
        let safe_intensity = if intensity.is_finite() {
            intensity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let idx = match safe_intensity {
            x if x <= 0.0 => 0,
            x if x < 0.25 => 1,
            x if x < 0.50 => 2,
            x if x < 0.75 => 3,
            _ => 4,
        };
        theme_colors[idx]
    };

    let mut click_areas_to_add: Vec<(Rect, usize, usize)> = Vec::new();

    for (week_idx, week) in graph.weeks.iter().skip(start_week).enumerate() {
        let x = graph_start_x + (week_idx as u16 * CELL_WIDTH);

        for (day_idx, day_opt) in week.iter().enumerate() {
            let y = graph_start_y + day_idx as u16;

            if x >= inner.x + inner.width || y >= inner.y + inner.height {
                continue;
            }

            let actual_week_idx = week_idx + start_week;
            let is_selected = selected_cell == Some((actual_week_idx, day_idx));

            let (cell_str, style) = match day_opt {
                Some(day) => {
                    let color = intensity_color(day.intensity);
                    if is_selected {
                        ("▓▓", Style::default().fg(Color::White).bg(color))
                    } else {
                        ("██", Style::default().fg(color))
                    }
                }
                None => {
                    if is_selected {
                        ("▓▓", Style::default().fg(Color::White).bg(theme_colors[0]))
                    } else {
                        ("· ", Style::default().fg(Color::Rgb(102, 102, 102)))
                    }
                }
            };

            let cell = Paragraph::new(cell_str).style(style);
            frame.render_widget(cell, Rect::new(x, y, CELL_WIDTH, 1));

            click_areas_to_add.push((Rect::new(x, y, CELL_WIDTH, 1), actual_week_idx, day_idx));
        }
    }

    for (rect, week, day) in click_areas_to_add {
        app.add_click_area(rect, ClickAction::GraphCell { week, day });
    }

    let month_y = inner.y;
    let mut current_month: Option<usize> = None;

    for (week_idx, week) in graph.weeks.iter().skip(start_week).enumerate() {
        if let Some(Some(day)) = week.first() {
            let month = day
                .date
                .format("%m")
                .to_string()
                .parse::<usize>()
                .unwrap_or(1)
                - 1;
            if current_month != Some(month) {
                current_month = Some(month);
                let x = graph_start_x + (week_idx as u16 * CELL_WIDTH);
                if x + 3 < inner.x + inner.width && month < MONTH_LABELS.len() {
                    let label =
                        Paragraph::new(MONTH_LABELS[month]).style(Style::default().fg(theme_muted));
                    frame.render_widget(label, Rect::new(x, month_y, 3, 1));
                }
            }
        }
    }
}

fn render_stats_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Stats ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let is_narrow = app.is_narrow();
    let graph = &app.data.graph;

    let total_tokens: u64 = graph
        .as_ref()
        .map(|g| {
            g.weeks
                .iter()
                .flat_map(|w| w.iter())
                .filter_map(|d| d.as_ref())
                .map(|d| d.tokens)
                .sum()
        })
        .unwrap_or(0);

    let total_cost: f64 = graph
        .as_ref()
        .map(|g| {
            g.weeks
                .iter()
                .flat_map(|w| w.iter())
                .filter_map(|d| d.as_ref())
                .map(|d| d.cost)
                .sum()
        })
        .unwrap_or(0.0);

    let active_days: u32 = graph
        .as_ref()
        .map(|g| {
            g.weeks
                .iter()
                .flat_map(|w| w.iter())
                .filter_map(|d| d.as_ref())
                .filter(|d| d.tokens > 0)
                .count() as u32
        })
        .unwrap_or(0);

    let total_days: u32 = graph
        .as_ref()
        .map(|g| {
            g.weeks
                .iter()
                .flat_map(|w| w.iter())
                .filter(|d| d.is_some())
                .count() as u32
        })
        .unwrap_or(365);

    let favorite_model = app.data.models.iter().max_by(|a, b| {
        a.cost
            .partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let favorite_model_name = favorite_model.map(|m| m.model.as_str()).unwrap_or("N/A");
    let model_color = favorite_model
        .map(|m| app.model_color_for(&m.provider, &m.model))
        .unwrap_or_else(|| app.model_color("N/A"));
    let sessions: u32 = app.data.models.iter().map(|m| m.session_count).sum();

    let col1_width = if is_narrow { 36u16 } else { 60u16 };
    let col2_x = inner.x + col1_width;
    let y_max = inner.y + inner.height;

    let mut y = inner.y;

    let row1_label = if is_narrow {
        "Model:"
    } else {
        "Favorite model:"
    };
    let row1 = Line::from(vec![
        Span::styled(row1_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            truncate_model_name(favorite_model_name, if is_narrow { 15 } else { 30 }),
            Style::default().fg(model_color),
        ),
    ]);
    frame.render_widget(Paragraph::new(row1), Rect::new(inner.x, y, col1_width, 1));

    let tokens_label = if is_narrow {
        "Tokens:"
    } else {
        "Total tokens:"
    };
    let row1_col2 = Line::from(vec![
        Span::styled(tokens_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            format_tokens(total_tokens),
            Style::default().fg(Color::Cyan),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(row1_col2),
        Rect::new(col2_x, y, inner.width.saturating_sub(col1_width), 1),
    );

    y += 1;
    if y >= y_max {
        return;
    }

    let row2 = Line::from(vec![
        Span::styled("Sessions:", Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(sessions.to_string(), Style::default().fg(Color::Cyan)),
    ]);
    frame.render_widget(Paragraph::new(row2), Rect::new(inner.x, y, col1_width, 1));

    let cost_label = if is_narrow { "Cost:" } else { "Total cost:" };
    let row2_col2 = Line::from(vec![
        Span::styled(cost_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(format_cost(total_cost), Style::default().fg(Color::Green)),
    ]);
    frame.render_widget(
        Paragraph::new(row2_col2),
        Rect::new(col2_x, y, inner.width.saturating_sub(col1_width), 1),
    );

    y += 1;
    if y >= y_max {
        return;
    }

    // Row 3: Current streak / Longest streak
    let streak_label = if is_narrow {
        "Streak:"
    } else {
        "Current streak:"
    };
    let row3 = Line::from(vec![
        Span::styled(streak_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            format!("{} days", app.data.current_streak),
            Style::default().fg(Color::Cyan),
        ),
    ]);
    frame.render_widget(Paragraph::new(row3), Rect::new(inner.x, y, col1_width, 1));

    let longest_label = if is_narrow {
        "Max streak:"
    } else {
        "Longest streak:"
    };
    let row3_col2 = Line::from(vec![
        Span::styled(longest_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            format!("{} days", app.data.longest_streak),
            Style::default().fg(Color::Cyan),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(row3_col2),
        Rect::new(col2_x, y, inner.width.saturating_sub(col1_width), 1),
    );

    y += 1;
    if y >= y_max {
        return;
    }

    let active_label = if is_narrow { "Active:" } else { "Active days:" };
    let active_days_line = Line::from(vec![
        Span::styled(active_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            format!("{}/{}", active_days, total_days),
            Style::default().fg(Color::Cyan),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(active_days_line),
        Rect::new(inner.x, y, col1_width, 1),
    );

    y += 2;
    if y >= y_max {
        return;
    }

    let legend_spans = vec![
        Span::styled("Less ", Style::default().fg(app.theme.muted)),
        Span::styled("· ", Style::default().fg(Color::Rgb(102, 102, 102))),
        Span::styled("██", Style::default().fg(app.theme.colors[1])),
        Span::raw(" "),
        Span::styled("██", Style::default().fg(app.theme.colors[2])),
        Span::raw(" "),
        Span::styled("██", Style::default().fg(app.theme.colors[3])),
        Span::raw(" "),
        Span::styled("██", Style::default().fg(app.theme.colors[4])),
        Span::styled(" More", Style::default().fg(app.theme.muted)),
    ];
    let legend_line = Line::from(legend_spans);
    frame.render_widget(
        Paragraph::new(legend_line),
        Rect::new(inner.x, y, inner.width, 1),
    );

    y += 2;
    if y >= y_max {
        return;
    }

    if !is_narrow {
        let footer = Line::from(Span::styled(
            format!(
                "Your total spending is ${:.2} on AI coding assistants!",
                total_cost
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ));
        frame.render_widget(
            Paragraph::new(footer),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

fn render_breakdown_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Day Breakdown (ESC to close) ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (week_idx, day_idx) = match app.selected_graph_cell {
        Some(cell) => cell,
        None => return,
    };

    let graph = match &app.data.graph {
        Some(g) => g,
        None => {
            app.stats_breakdown_total_lines = 0;
            return;
        }
    };

    let day = match graph
        .weeks
        .get(week_idx)
        .and_then(|w| w.get(day_idx))
        .and_then(|d| d.as_ref())
    {
        Some(d) => d,
        None => {
            app.stats_breakdown_total_lines = 0;
            let no_data = Paragraph::new("No data for this day")
                .style(Style::default().fg(app.theme.muted))
                .alignment(Alignment::Center);
            frame.render_widget(no_data, inner);
            return;
        }
    };

    let daily_usage = app.data.daily.iter().find(|d| d.date == day.date);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                day.date.format("%a, %b %d, %Y").to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(format_tokens(day.tokens), Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::styled(
                format_cost(day.cost),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];

    if let Some(daily) = daily_usage {
        if daily.source_breakdown.is_empty() {
            lines.push(Line::from(Span::styled(
                "No detailed breakdown available",
                Style::default().fg(app.theme.muted),
            )));
        } else {
            for (client, source_info) in &daily.source_breakdown {
                let mut models: Vec<_> = source_info.models.values().collect();
                models.sort_by(|a, b| {
                    b.tokens
                        .total()
                        .cmp(&a.tokens.total())
                        .then_with(|| a.display_name.cmp(&b.display_name))
                });

                let client_color = get_client_color(client);
                let client_name = get_client_display_name(client);
                let model_count = models.len();
                let plural = if model_count > 1 { "s" } else { "" };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("● {}", client_name),
                        Style::default()
                            .fg(client_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" ({} model{})", model_count, plural),
                        Style::default().fg(app.theme.muted),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format_cost(source_info.cost),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));

                for model_info in models {
                    let model_color =
                        app.model_color_for(&model_info.provider, &model_info.color_key);
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("●", Style::default().fg(model_color)),
                        Span::styled(
                            format!(" {}", truncate_model_name(&model_info.display_name, 25)),
                            Style::default().fg(Color::White),
                        ),
                    ]));

                    let is_narrow = app.is_narrow();
                    if is_narrow {
                        lines.push(Line::from(vec![
                            Span::styled("    ", Style::default()),
                            Span::styled(
                                format_tokens(model_info.tokens.input),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                            Span::styled("/", Style::default().fg(Color::Rgb(102, 102, 102))),
                            Span::styled(
                                format_tokens(model_info.tokens.output),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                            Span::styled("/", Style::default().fg(Color::Rgb(102, 102, 102))),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_read),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                            Span::styled("/", Style::default().fg(Color::Rgb(102, 102, 102))),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_write),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "    In: ",
                                Style::default().fg(Color::Rgb(102, 102, 102)),
                            ),
                            Span::styled(
                                format_tokens(model_info.tokens.input),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                            Span::styled(
                                " · Out: ",
                                Style::default().fg(Color::Rgb(102, 102, 102)),
                            ),
                            Span::styled(
                                format_tokens(model_info.tokens.output),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                            Span::styled(" · CR: ", Style::default().fg(Color::Rgb(102, 102, 102))),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_read),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                            Span::styled(" · CW: ", Style::default().fg(Color::Rgb(102, 102, 102))),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_write),
                                Style::default().fg(Color::Rgb(170, 170, 170)),
                            ),
                        ]));
                    }
                }
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No detailed breakdown available",
            Style::default().fg(app.theme.muted),
        )));
    }

    let visible_height = inner.height.max(1) as usize;
    app.max_visible_items = visible_height;
    app.stats_breakdown_total_lines = lines.len();

    if lines.is_empty() {
        app.selected_index = 0;
        app.scroll_offset = 0;
    } else {
        app.selected_index = app.selected_index.min(lines.len() - 1);
        let max_scroll = lines.len().saturating_sub(visible_height);
        app.scroll_offset = app.scroll_offset.min(max_scroll);
    }

    let paragraph = Paragraph::new(lines).scroll((app.scroll_offset as u16, 0));
    frame.render_widget(paragraph, inner);

    if app.stats_breakdown_total_lines > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state =
            ScrollbarState::new(app.stats_breakdown_total_lines).position(app.scroll_offset);

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

fn truncate_model_name(s: &str, max_chars: usize) -> String {
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
