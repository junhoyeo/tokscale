use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::widgets::{
    ambient_stable_scrollbar, format_cost, format_tokens, get_client_color,
    get_client_display_name, viewport_scrollbar_state, AMBIENT_STABLE_BORDER_SET,
};
use crate::tui::app::{App, ClickAction};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

const CELL_WIDTH: u16 = 2;
fn day_labels(lang: TuiLanguage) -> &'static [&'static str] {
    match lang {
        TuiLanguage::En => &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
        TuiLanguage::Ko => &["일", "월", "화", "수", "목", "금", "토"],
        TuiLanguage::Ja => &["日", "月", "火", "水", "木", "金", "土"],
        TuiLanguage::ZhCn => &["日", "一", "二", "三", "四", "五", "六"],
        TuiLanguage::Fr => &["dim", "lun", "mar", "mer", "jeu", "ven", "sam"],
    }
}

fn month_labels(lang: TuiLanguage) -> &'static [&'static str] {
    match lang {
        TuiLanguage::En => &[
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ],
        TuiLanguage::Ko => &[
            "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
        ],
        TuiLanguage::Ja => &[
            "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
        ],
        TuiLanguage::ZhCn => &[
            "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
        ],
        TuiLanguage::Fr => &[
            "jan", "fév", "mar", "avr", "mai", "jui", "jul", "aoû", "sep", "oct", "nov", "déc",
        ],
    }
}

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
    let subtle_text_style = app.theme.subtle_text_style();
    let selected_cell = app.selected_graph_cell;
    let is_narrow = app.is_narrow();

    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(theme_border))
        .title(Span::styled(
            tr(lang, MessageKey::TitleContributionGraph),
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

    let day_labels = day_labels(lang);
    for (day_idx, label) in day_labels.iter().enumerate() {
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
                        ("▓▓", app.theme.graph_cell_selected_style(color))
                    } else {
                        ("██", Style::default().fg(color))
                    }
                }
                None => {
                    if is_selected {
                        ("▓▓", app.theme.graph_cell_selected_style(theme_colors[0]))
                    } else {
                        ("· ", subtle_text_style)
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
    let month_labels = month_labels(lang);

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
                if month < month_labels.len() {
                    let label_text = month_labels[month];
                    let label_w = unicode_width::UnicodeWidthStr::width(label_text) as u16;
                    if x + label_w <= inner.x + inner.width {
                        let label =
                            Paragraph::new(label_text).style(Style::default().fg(theme_muted));
                        frame.render_widget(label, Rect::new(x, month_y, label_w, 1));
                    }
                }
            }
        }
    }
}

fn render_stats_panel(frame: &mut Frame, app: &App, area: Rect) {
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            format!(" {} ", tr(lang, MessageKey::TabStats)),
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
                // Plain `.sum()` panics (debug) / wraps (release) if a single
                // corrupt/huge day's token count overflows u64 across the
                // graph; saturate instead so one bad day doesn't poison the
                // whole panel's total.
                .fold(0u64, u64::saturating_add)
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
        .map(|m| app.model_color_for(&m.provider, &m.color_key))
        .unwrap_or_else(|| app.model_color("N/A"));
    let sessions: u32 = app.data.models.iter().map(|m| m.session_count).sum();

    let col1_width = if is_narrow { 36u16 } else { 60u16 };
    let col2_x = inner.x + col1_width;
    let y_max = inner.y + inner.height;

    let mut y = inner.y;

    let row1_label = if is_narrow {
        tr(lang, MessageKey::StatsFavoriteModelShort)
    } else {
        tr(lang, MessageKey::StatsFavoriteModel)
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
        tr(lang, MessageKey::StatsTokensShort)
    } else {
        tr(lang, MessageKey::StatsTotalTokens)
    };
    let row1_col2 = Line::from(vec![
        Span::styled(tokens_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(format_tokens(total_tokens), app.theme.count_style()),
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
        Span::styled(
            tr(lang, MessageKey::StatsSessions),
            Style::default().fg(app.theme.muted),
        ),
        Span::raw(" "),
        Span::styled(sessions.to_string(), app.theme.count_style()),
    ]);
    frame.render_widget(Paragraph::new(row2), Rect::new(inner.x, y, col1_width, 1));

    let cost_label = if is_narrow {
        tr(lang, MessageKey::StatsCostShort)
    } else {
        tr(lang, MessageKey::StatsTotalCost)
    };
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
        tr(lang, MessageKey::StatsStreakShort)
    } else {
        tr(lang, MessageKey::StatsCurrentStreak)
    };
    let format_streak = |streak: u32| -> String {
        let day_key = if streak == 1 {
            MessageKey::CountDay
        } else {
            MessageKey::CountDays
        };
        match lang {
            TuiLanguage::Ko | TuiLanguage::Ja | TuiLanguage::ZhCn => {
                format!("{}{}", streak, tr(lang, day_key))
            }
            TuiLanguage::En | TuiLanguage::Fr => {
                format!("{} {}", streak, tr(lang, day_key))
            }
        }
    };
    let row3 = Line::from(vec![
        Span::styled(streak_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            format_streak(app.data.current_streak),
            app.theme.count_style(),
        ),
    ]);
    frame.render_widget(Paragraph::new(row3), Rect::new(inner.x, y, col1_width, 1));

    let longest_label = if is_narrow {
        tr(lang, MessageKey::StatsLongestStreakShort)
    } else {
        tr(lang, MessageKey::StatsLongestStreak)
    };
    let row3_col2 = Line::from(vec![
        Span::styled(longest_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            format_streak(app.data.longest_streak),
            app.theme.count_style(),
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

    let active_label = if is_narrow {
        tr(lang, MessageKey::StatsActiveShort)
    } else {
        tr(lang, MessageKey::StatsActiveDays)
    };
    let active_days_line = Line::from(vec![
        Span::styled(active_label, Style::default().fg(app.theme.muted)),
        Span::raw(" "),
        Span::styled(
            format!("{}/{}", active_days, total_days),
            app.theme.count_style(),
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
        Span::styled(
            format!("{} ", tr(lang, MessageKey::StatsLess)),
            Style::default().fg(app.theme.muted),
        ),
        Span::styled("· ", app.theme.subtle_text_style()),
        Span::styled("██", Style::default().fg(app.theme.colors[1])),
        Span::raw(" "),
        Span::styled("██", Style::default().fg(app.theme.colors[2])),
        Span::raw(" "),
        Span::styled("██", Style::default().fg(app.theme.colors[3])),
        Span::raw(" "),
        Span::styled("██", Style::default().fg(app.theme.colors[4])),
        Span::styled(
            format!(" {}", tr(lang, MessageKey::StatsMore)),
            Style::default().fg(app.theme.muted),
        ),
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
        let footer_text = match lang {
            TuiLanguage::Ko => {
                format!("AI 코딩 어시스턴트에 총 ${:.2}를 지출했습니다!", total_cost)
            }
            TuiLanguage::Ja => format!(
                "AIコーディングアシスタントに合計${:.2}を使用しました！",
                total_cost
            ),
            TuiLanguage::ZhCn => format!("您在AI编程助手上共花费了 ${:.2}！", total_cost),
            TuiLanguage::Fr => format!(
                "Votre dépense totale pour les assistants IA est de ${:.2} !",
                total_cost
            ),
            TuiLanguage::En => format!(
                "Your total spending is ${:.2} on AI coding assistants!",
                total_cost
            ),
        };
        let footer = Line::from(Span::styled(
            footer_text,
            Style::default()
                .fg(app.theme.hint_key_color())
                .add_modifier(Modifier::ITALIC),
        ));
        frame.render_widget(
            Paragraph::new(footer),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

fn format_localized_date(date: chrono::NaiveDate, lang: TuiLanguage) -> String {
    use chrono::Datelike;
    let year = date.year();
    let month = date.month() as usize;
    let day = date.day();
    let weekday = date.weekday();

    match lang {
        TuiLanguage::En => {
            let months = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
            let w_idx = weekday.num_days_from_monday() as usize;
            let m_str = months.get(month.saturating_sub(1)).unwrap_or(&"Jan");
            let w_str = weekdays.get(w_idx).unwrap_or(&"Mon");
            format!("{}, {} {:02}, {}", w_str, m_str, day, year)
        }
        TuiLanguage::Ko => {
            let weekdays = ["월", "화", "수", "목", "금", "토", "일"];
            let w_idx = weekday.num_days_from_monday() as usize;
            let w_str = weekdays.get(w_idx).unwrap_or(&"월");
            format!("{}년 {}월 {}일 ({})", year, month, day, w_str)
        }
        TuiLanguage::Ja => {
            let weekdays = ["月", "火", "水", "木", "金", "土", "日"];
            let w_idx = weekday.num_days_from_monday() as usize;
            let w_str = weekdays.get(w_idx).unwrap_or(&"月");
            format!("{}年{}月{}日 ({})", year, month, day, w_str)
        }
        TuiLanguage::ZhCn => {
            let weekdays = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
            let w_idx = weekday.num_days_from_monday() as usize;
            let w_str = weekdays.get(w_idx).unwrap_or(&"周一");
            format!("{}年{}月{}日 {}", year, month, day, w_str)
        }
        TuiLanguage::Fr => {
            let months = [
                "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.",
                "nov.", "déc.",
            ];
            let weekdays = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
            let w_idx = weekday.num_days_from_monday() as usize;
            let m_str = months.get(month.saturating_sub(1)).unwrap_or(&"janv.");
            let w_str = weekdays.get(w_idx).unwrap_or(&"lun.");
            format!("{} {} {} {}", w_str, day, m_str, year)
        }
    }
}

fn render_breakdown_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            tr(lang, MessageKey::TitleDayBreakdown),
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
            let no_data = Paragraph::new(tr(lang, MessageKey::EmptyNoDataForDay))
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
                format_localized_date(day.date, lang),
                Style::default()
                    .fg(app.theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(format_tokens(day.tokens), app.theme.count_style()),
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
                tr(lang, MessageKey::EmptyNoBreakdownAvailable),
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

                let client_color = app.theme.color(get_client_color(client));
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
                            Style::default().fg(app.theme.foreground),
                        ),
                    ]));

                    let is_narrow = app.is_narrow();
                    if is_narrow {
                        let secondary_text_style = app.theme.secondary_text_style();
                        let subtle_text_style = app.theme.subtle_text_style();
                        lines.push(Line::from(vec![
                            Span::styled("    ", Style::default()),
                            Span::styled(
                                format_tokens(model_info.tokens.input),
                                secondary_text_style,
                            ),
                            Span::styled("/", subtle_text_style),
                            Span::styled(
                                format_tokens(model_info.tokens.output),
                                secondary_text_style,
                            ),
                            Span::styled("/", subtle_text_style),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_read),
                                secondary_text_style,
                            ),
                            Span::styled("/", subtle_text_style),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_write),
                                secondary_text_style,
                            ),
                        ]));
                    } else {
                        let secondary_text_style = app.theme.secondary_text_style();
                        let subtle_text_style = app.theme.subtle_text_style();
                        lines.push(Line::from(vec![
                            Span::styled("    In: ", subtle_text_style),
                            Span::styled(
                                format_tokens(model_info.tokens.input),
                                secondary_text_style,
                            ),
                            Span::styled(" · Out: ", subtle_text_style),
                            Span::styled(
                                format_tokens(model_info.tokens.output),
                                secondary_text_style,
                            ),
                            Span::styled(" · CR: ", subtle_text_style),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_read),
                                secondary_text_style,
                            ),
                            Span::styled(" · CW: ", subtle_text_style),
                            Span::styled(
                                format_tokens(model_info.tokens.cache_write),
                                secondary_text_style,
                            ),
                        ]));
                    }
                }
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            tr(lang, MessageKey::EmptyNoBreakdownAvailable),
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
        let scrollbar = ambient_stable_scrollbar();

        let mut scrollbar_state = viewport_scrollbar_state(
            app.stats_breakdown_total_lines,
            app.scroll_offset,
            visible_height,
        );

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::TuiConfig;
    use crate::tui::data::{ContributionDay, GraphData};
    use chrono::NaiveDate;
    use ratatui::{backend::TestBackend, Terminal};

    fn make_app() -> App {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: None,
            ..Default::default()
        };
        let mut app = App::new_with_cached_data(config, None).unwrap();
        app.settings.tui_language = TuiLanguage::En;
        app
    }

    fn corrupt_day(date: NaiveDate) -> ContributionDay {
        ContributionDay {
            date,
            tokens: u64::MAX,
            cost: 0.0,
            intensity: 1.0,
        }
    }

    #[test]
    fn saturated_graph_days_render_without_overflowing_total() {
        let mut app = make_app();
        // Three days each at u64::MAX: no single day overflows, but a plain
        // `.sum()` across them does. render_stats_panel must saturate
        // instead of panicking (debug) or wrapping (release).
        app.data.graph = Some(GraphData {
            weeks: vec![vec![
                Some(corrupt_day(NaiveDate::from_ymd_opt(2026, 5, 27).unwrap())),
                Some(corrupt_day(NaiveDate::from_ymd_opt(2026, 5, 28).unwrap())),
                Some(corrupt_day(NaiveDate::from_ymd_opt(2026, 5, 29).unwrap())),
            ]],
        });

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_stats_panel(frame, &app, Rect::new(0, 0, 80, 20)))
            .unwrap();

        let body = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect::<String>();
        assert!(!body.trim().is_empty());
    }

    #[test]
    fn format_localized_date_supports_all_languages() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(); // Thursday
        assert_eq!(
            format_localized_date(date, TuiLanguage::En),
            "Thu, Sep 24, 2026"
        );
        assert_eq!(
            format_localized_date(date, TuiLanguage::Ko),
            "2026년 9월 24일 (목)"
        );
        assert_eq!(
            format_localized_date(date, TuiLanguage::Ja),
            "2026年9月24日 (木)"
        );
        assert_eq!(
            format_localized_date(date, TuiLanguage::ZhCn),
            "2026年9月24日 周四"
        );
        assert_eq!(
            format_localized_date(date, TuiLanguage::Fr),
            "jeu. 24 sept. 2026"
        );
    }

    #[test]
    fn day_and_month_labels_have_twelve_and_seven_items() {
        for lang in [
            TuiLanguage::En,
            TuiLanguage::Ko,
            TuiLanguage::Ja,
            TuiLanguage::ZhCn,
            TuiLanguage::Fr,
        ] {
            assert_eq!(day_labels(lang).len(), 7);
            assert_eq!(month_labels(lang).len(), 12);
        }
    }
}
