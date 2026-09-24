use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::widgets::{format_cost, format_tokens, AMBIENT_STABLE_BORDER_SET};
use crate::tui::app::App;
use crate::tui::data::{aggregate_by_period, aggregate_by_weekday, find_peak_hour};
use crate::tui::i18n::{tr, MessageKey};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            tr(lang, MessageKey::TitleHourlyProfile),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.data.hourly.is_empty() {
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoHourlyData))
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let hourly = &app.data.hourly;
    let total_tokens = app.data.total_tokens;
    let total_cost = app.data.total_cost;

    // Calculate dynamic bar width to use most of the available width
    // Period line: label (10) + hour_range (12) + spaces (4) + percentage (8) = 34 chars overhead
    // Weekday line: day name (10) + spaces (3) + percentage (8) = 21 chars overhead
    // Use period overhead since it's larger, then subtract a margin for safety
    let overhead = 36; // 34 + small margin
    let bar_width = (inner.width as usize)
        .saturating_sub(overhead)
        .clamp(20, 80);

    // Get date range
    let min_date = hourly.iter().map(|h| h.datetime.date()).min();
    let max_date = hourly.iter().map(|h| h.datetime.date()).max();
    let date_range = match (min_date, max_date) {
        (Some(mn), Some(mx)) if mn == mx => mn.format("%Y-%m-%d").to_string(),
        (Some(mn), Some(mx)) => format!("{} to {}", mn.format("%Y-%m-%d"), mx.format("%Y-%m-%d")),
        _ => tr(lang, MessageKey::EmptyNoDataAvailable).to_string(),
    };

    // Aggregate data
    let periods = aggregate_by_period(hourly);
    let weekdays = aggregate_by_weekday(hourly);
    let peak_hour = find_peak_hour(hourly);

    // Build content
    let mut lines: Vec<Line> = Vec::new();

    // Title line
    lines.push(Line::from(vec![
        Span::styled(
            "Hourly Profile",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(&date_range, Style::default().fg(app.theme.muted)),
    ]));
    lines.push(Line::from(""));

    // Summary line
    let summary_spans = vec![
        Span::styled(
            format!("{} {}", hourly.len(), tr(lang, MessageKey::CountHours)),
            app.theme.count_style(),
        ),
        Span::styled("  |  ", Style::default().fg(app.theme.muted)),
        Span::styled(
            format!(
                "{} {}",
                format_tokens(total_tokens),
                tr(lang, MessageKey::ProfileTotalTokens)
            ),
            app.theme.count_style(),
        ),
        Span::styled("  |  ", Style::default().fg(app.theme.muted)),
        Span::styled(
            format!(
                "{} {}",
                format_cost(total_cost),
                tr(lang, MessageKey::ProfileTotalCost)
            ),
            Style::default().fg(Color::Green),
        ),
    ];
    lines.push(Line::from(summary_spans));
    lines.push(Line::from(""));

    // Time-of-day breakdown
    lines.push(Line::from(vec![Span::styled(
        tr(lang, MessageKey::ProfileWhenYouWorkMost),
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    let max_period_tokens = periods.iter().map(|p| p.total_tokens).max().unwrap_or(1);

    for period in &periods {
        let percentage = if total_tokens > 0 {
            period.total_tokens as f64 / total_tokens as f64 * 100.0
        } else {
            0.0
        };
        let bar_filled = if max_period_tokens > 0 {
            (period.total_tokens as f64 / max_period_tokens as f64 * bar_width as f64).round()
                as usize
        } else {
            0
        };
        let bar_filled = bar_filled.min(bar_width);
        let bar_empty = bar_width - bar_filled;

        let bar = format!("{}{}", "█".repeat(bar_filled), "░".repeat(bar_empty));

        let p_label = match period.label {
            "Morning" => tr(lang, MessageKey::PeriodMorning),
            "Daytime" => tr(lang, MessageKey::PeriodDaytime),
            "Evening" => tr(lang, MessageKey::PeriodEvening),
            "Night" => tr(lang, MessageKey::PeriodNight),
            _ => period.label,
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<10}", p_label),
                Style::default().fg(app.theme.foreground),
            ),
            Span::styled(
                format!("{:>12}", period.hour_range),
                Style::default().fg(app.theme.muted),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(bar, Style::default().fg(Color::Green)),
            Span::styled("  ", Style::default()),
            Span::styled(format!("{:>5.1}%", percentage), app.theme.hint_key_style()),
        ]));
    }
    lines.push(Line::from(""));

    // Weekday breakdown
    lines.push(Line::from(vec![Span::styled(
        tr(lang, MessageKey::ProfileMostProductiveDay),
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    let max_weekday_tokens = weekdays.iter().map(|w| w.total_tokens).max().unwrap_or(1);

    // Find best weekday
    let best_weekday = weekdays
        .iter()
        .max_by_key(|w| w.total_tokens)
        .map(|w| w.day)
        .unwrap_or("Monday");

    for weekday in &weekdays {
        let percentage = if total_tokens > 0 {
            weekday.total_tokens as f64 / total_tokens as f64 * 100.0
        } else {
            0.0
        };
        let bar_filled = if max_weekday_tokens > 0 {
            (weekday.total_tokens as f64 / max_weekday_tokens as f64 * bar_width as f64).round()
                as usize
        } else {
            0
        };
        let bar_filled = bar_filled.min(bar_width);
        let bar_empty = bar_width - bar_filled;

        let bar = format!("{}{}", "█".repeat(bar_filled), "░".repeat(bar_empty));

        let is_best = weekday.day == best_weekday;

        let w_label = match weekday.day {
            "Monday" => tr(lang, MessageKey::WeekdayMonday),
            "Tuesday" => tr(lang, MessageKey::WeekdayTuesday),
            "Wednesday" => tr(lang, MessageKey::WeekdayWednesday),
            "Thursday" => tr(lang, MessageKey::WeekdayThursday),
            "Friday" => tr(lang, MessageKey::WeekdayFriday),
            "Saturday" => tr(lang, MessageKey::WeekdaySaturday),
            "Sunday" => tr(lang, MessageKey::WeekdaySunday),
            _ => weekday.day,
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<10}", w_label),
                Style::default().fg(if is_best {
                    app.theme.hint_key_color()
                } else {
                    app.theme.foreground
                }),
            ),
            Span::styled(bar, Style::default().fg(Color::Green)),
            Span::styled("  ", Style::default()),
            Span::styled(format!("{:>5.1}%", percentage), app.theme.hint_key_style()),
        ]));
    }
    lines.push(Line::from(""));

    // Peak hour insight
    if let Some((hour, tokens, cost)) = peak_hour {
        lines.push(Line::from(vec![
            Span::styled(
                tr(lang, MessageKey::ProfilePeakHour),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:02}:00-{:02}:59", hour, hour),
                app.theme.hint_key_style(),
            ),
            Span::styled("  (", Style::default().fg(app.theme.muted)),
            Span::styled(format_tokens(tokens), app.theme.count_style()),
            Span::styled(
                format!(" {}, ", tr(lang, MessageKey::ColTokens)),
                Style::default().fg(app.theme.muted),
            ),
            Span::styled(format_cost(cost), Style::default().fg(Color::Green)),
            Span::styled(")", Style::default().fg(app.theme.muted)),
        ]));
    }

    // Legend
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            tr(lang, MessageKey::ProfileLegend),
            Style::default().fg(app.theme.muted),
        ),
        Span::styled("░", Style::default().fg(app.theme.muted)),
        Span::styled(
            format!(" {}  ", tr(lang, MessageKey::ProfileLow)),
            Style::default().fg(app.theme.muted),
        ),
        Span::styled("█", Style::default().fg(Color::Green)),
        Span::styled(
            format!(" {}", tr(lang, MessageKey::ProfileHigh)),
            Style::default().fg(app.theme.muted),
        ),
    ]));

    // Hint
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        tr(lang, MessageKey::ProfileSwitchHint),
        Style::default().fg(app.theme.muted),
    )]));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Left);
    frame.render_widget(paragraph, inner);
}
