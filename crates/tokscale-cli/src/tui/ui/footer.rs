use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::spinner::{get_phase_message, get_scanner_spans};
use super::widgets::{format_cost, format_tokens, AMBIENT_STABLE_BORDER_SET};
use crate::tui::app::{App, ClickAction, SortField, Tab};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into 3 rows: sources+sort, help text, status
    let row_constraints = if inner.height >= 3 {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else if inner.height >= 2 {
        vec![Constraint::Length(1), Constraint::Length(1)]
    } else {
        vec![Constraint::Length(1)]
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(inner);

    render_main_row(frame, app, rows[0]);

    if rows.len() >= 2 {
        render_help_row(frame, app, rows[1]);
    }

    if rows.len() >= 3 {
        render_status_row(frame, app, rows[2]);
    }
}

fn render_main_row(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let is_very_narrow = app.is_very_narrow();

    // Split into left (sort buttons) and right (totals)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Left side: sort buttons
    if !is_very_narrow {
        let mut spans: Vec<Span> = Vec::new();
        let mut x_offset = chunks[0].x;

        let sort_label = tr(lang, MessageKey::SortLabel);
        spans.push(Span::styled(
            sort_label,
            Style::default().fg(app.theme.muted),
        ));
        x_offset += unicode_width::UnicodeWidthStr::width(sort_label) as u16;

        let sort_buttons = [
            (SortField::Date, tr(lang, MessageKey::SortDate)),
            (SortField::Cost, tr(lang, MessageKey::SortCost)),
            (SortField::Tokens, tr(lang, MessageKey::SortTokens)),
        ];

        for (field, label) in sort_buttons {
            let is_active = app.sort_field == field;
            let style = if is_active {
                Style::default()
                    .fg(app.theme.foreground)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.muted)
            };

            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));

            let btn_width = unicode_width::UnicodeWidthStr::width(label) as u16;
            app.add_click_area(
                Rect::new(x_offset, chunks[0].y, btn_width, 1),
                ClickAction::Sort(field),
            );
            x_offset += btn_width + 1;
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, chunks[0]);
    }

    // Right side: scroll info | tokens | cost
    let mut right_spans: Vec<Span> = Vec::new();

    // Scroll position indicator for Overview tab
    if app.current_tab == Tab::Overview {
        let total_models = app.data.models.len();
        if total_models > app.max_visible_items && app.max_visible_items > 0 {
            let start = app.scroll_offset + 1;
            let end = (app.scroll_offset + app.max_visible_items).min(total_models);
            if !is_very_narrow {
                let scroll_text = match lang {
                    TuiLanguage::Ko => format!("↓ {}-{} (총 {}개) ", start, end, total_models),
                    TuiLanguage::Ja => format!("↓ {}-{} (全{}件) ", start, end, total_models),
                    TuiLanguage::ZhCn => format!("↓ {}-{} (共{}项) ", start, end, total_models),
                    TuiLanguage::Fr => format!("↓ {}-{} sur {} ", start, end, total_models),
                    TuiLanguage::En => format!("↓ {}-{} of {} ", start, end, total_models),
                };
                right_spans.push(Span::styled(
                    scroll_text,
                    Style::default().fg(app.theme.muted),
                ));
                right_spans.push(Span::styled("| ", Style::default().fg(app.theme.muted)));
            }
        }
    }

    // Total tokens
    let total_tokens = app.data.total_tokens;
    right_spans.push(Span::styled(
        format_tokens(total_tokens),
        app.theme.count_style(),
    ));
    if !is_very_narrow {
        right_spans.push(Span::styled(
            tr(lang, MessageKey::FooterTokens),
            Style::default().fg(app.theme.muted),
        ));
    }

    right_spans.push(Span::styled(" | ", Style::default().fg(app.theme.muted)));

    // Total cost
    right_spans.push(Span::styled(
        format_cost(app.data.total_cost),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ));

    // Current list count
    if !is_very_narrow {
        let count_label = current_count_label(app);
        right_spans.push(Span::styled(
            count_label,
            Style::default().fg(app.theme.muted),
        ));
    }

    let right_line = Line::from(right_spans);
    let right_para = Paragraph::new(right_line).alignment(Alignment::Right);
    frame.render_widget(right_para, chunks[1]);
}

fn current_count_label(app: &App) -> String {
    let lang = app.settings.tui_language;
    let format_count =
        |n: usize, key: MessageKey| -> String { format!(" ({} {})", n, tr(lang, key)) };
    match app.current_tab {
        Tab::Overview | Tab::Models => format_count(app.data.models.len(), MessageKey::CountModels),
        Tab::Agents => format_count(app.data.agents.len(), MessageKey::CountAgents),
        Tab::Daily if app.is_daily_detail_active() => format_count(
            app.get_sorted_daily_detail_rows().len(),
            MessageKey::CountModels,
        ),
        Tab::Daily => format_count(app.data.daily.len(), MessageKey::CountDays),
        Tab::Hourly => format_count(app.data.hourly.len(), MessageKey::CountHours),
        Tab::Minutely => format_count(app.data.minutely.len(), MessageKey::CountMinutes),
        Tab::Monthly if app.is_monthly_detail_active() => format_count(
            app.get_sorted_monthly_detail_days().len(),
            MessageKey::CountDays,
        ),
        Tab::Monthly => format_count(app.data.monthly.len(), MessageKey::CountMonths),
        Tab::Sessions => format_count(app.data.sessions.len(), MessageKey::CountSessions),
        Tab::Projects => format_count(app.data.projects.len(), MessageKey::CountProjects),
        Tab::Stats | Tab::Usage => String::new(),
    }
}

fn render_help_row(frame: &mut Frame, app: &App, area: Rect) {
    let is_very_narrow = app.is_very_narrow();
    let hint_style = app.theme.hint_key_style();
    let count_style = app.theme.count_style();

    let spans = if is_very_narrow {
        let mut spans = vec![
            Span::styled("↑↓", Style::default().fg(app.theme.muted)),
            Span::styled("·", Style::default().fg(app.theme.muted)),
            Span::styled("←→", Style::default().fg(app.theme.muted)),
            Span::styled("·", Style::default().fg(app.theme.muted)),
            Span::styled("d/t/c", Style::default().fg(Color::Blue)),
            Span::styled("·", Style::default().fg(app.theme.muted)),
            Span::styled("[s]", count_style),
            Span::styled("·", Style::default().fg(app.theme.muted)),
            Span::styled("[g]", count_style),
            Span::styled("·", Style::default().fg(app.theme.muted)),
            Span::styled("[p]", Style::default().fg(Color::Magenta)),
            Span::styled("·", Style::default().fg(app.theme.muted)),
            Span::styled("[r]", hint_style),
            Span::styled("·", Style::default().fg(app.theme.muted)),
            Span::styled("q", Style::default().fg(app.theme.muted)),
        ];
        if app.current_tab == Tab::Daily {
            spans.push(Span::styled("·", Style::default().fg(app.theme.muted)));
            if app.is_daily_detail_active() {
                spans.push(Span::styled("esc", hint_style));
            } else {
                spans.push(Span::styled("↵", hint_style));
                spans.push(Span::styled("·", Style::default().fg(app.theme.muted)));
                spans.push(Span::styled("j", hint_style));
            }
        }
        if app.current_tab == Tab::Monthly {
            spans.push(Span::styled("·", Style::default().fg(app.theme.muted)));
            if app.is_monthly_detail_active() {
                spans.push(Span::styled("esc", hint_style));
            } else {
                spans.push(Span::styled("↵", hint_style));
            }
        }
        if app.current_tab == Tab::Hourly {
            spans.push(Span::styled("·", Style::default().fg(app.theme.muted)));
            spans.push(Span::styled("v", hint_style));
        }
        spans
    } else {
        let lang = app.settings.tui_language;
        let mut spans = vec![
            Span::styled(
                format!("{} • ", tr(lang, MessageKey::HelpScroll)),
                Style::default().fg(app.theme.muted),
            ),
            Span::styled(
                tr(lang, MessageKey::HelpSort),
                Style::default().fg(Color::Blue),
            ),
            Span::styled(" • ", Style::default().fg(app.theme.muted)),
        ];
        if app.current_tab == Tab::Daily {
            if app.is_daily_detail_active() {
                spans.push(Span::styled(tr(lang, MessageKey::HelpBack), hint_style));
            } else {
                spans.push(Span::styled(tr(lang, MessageKey::HelpDetails), hint_style));
                spans.push(Span::styled(" ", Style::default()));
                spans.push(Span::styled(tr(lang, MessageKey::HelpToday), hint_style));
            }
            spans.push(Span::styled(" • ", Style::default().fg(app.theme.muted)));
        }
        if app.current_tab == Tab::Monthly {
            if app.is_monthly_detail_active() {
                spans.push(Span::styled(tr(lang, MessageKey::HelpBack), hint_style));
            } else {
                spans.push(Span::styled(tr(lang, MessageKey::HelpDetails), hint_style));
            }
            spans.push(Span::styled(" • ", Style::default().fg(app.theme.muted)));
        }
        if app.current_tab == Tab::Hourly {
            spans.push(Span::styled(tr(lang, MessageKey::HelpProfile), hint_style));
            spans.push(Span::styled(" • ", Style::default().fg(app.theme.muted)));
        }
        spans.push(Span::styled(tr(lang, MessageKey::HelpSources), count_style));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(
            tr(lang, MessageKey::HelpLanguage),
            count_style,
        ));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(
            format!("[g:{}]", app.group_by.borrow()),
            count_style,
        ));
        // `w` only does anything under workspace grouping, so only advertise it there.
        if *app.group_by.borrow() == tokscale_core::GroupBy::WorkspaceModel {
            spans.push(Span::styled(" ", Style::default()));
            spans.push(Span::styled(
                match app.worktree_rollup {
                    tokscale_core::WorktreeRollup::MergeIntoRepo => "[w:repos]",
                    tokscale_core::WorktreeRollup::Separate => "[w:worktrees]",
                },
                count_style,
            ));
        }
        spans.push(Span::styled(" • ", Style::default().fg(app.theme.muted)));
        spans.push(Span::styled(
            format!("[p:{}]", app.theme.name.as_str()),
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(
            if app.auto_refresh {
                format!("[R:auto {}s]", app.auto_refresh_interval.as_secs())
            } else {
                "[R:auto off]".to_string()
            },
            Style::default().fg(if app.auto_refresh {
                Color::Green
            } else {
                app.theme.muted
            }),
        ));
        spans.push(Span::styled(" • ", Style::default().fg(app.theme.muted)));
        spans.push(Span::styled(tr(lang, MessageKey::HelpRefresh), hint_style));
        spans.push(Span::styled(
            " • e • ",
            Style::default().fg(app.theme.muted),
        ));
        spans.push(Span::styled(
            tr(lang, MessageKey::HelpQuit),
            Style::default().fg(app.theme.muted),
        ));
        spans
    };

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

/// Data-source indicator label (#699): "local" when only this machine's
/// data is on screen, or "local+remote (N devices)" when server-side
/// aggregated stats are available for cross-checking.
fn data_source_label(app: &App) -> String {
    let lang = app.settings.tui_language;
    match app.remote_stats {
        Some(ref remote) => {
            let devices = if remote.device_count == 1 {
                tr(lang, MessageKey::StatusDevice).to_string()
            } else {
                match lang {
                    TuiLanguage::Ko => format!("{}대 기기", remote.device_count),
                    TuiLanguage::Ja => format!("{}台のデバイス", remote.device_count),
                    TuiLanguage::ZhCn => format!("{}台设备", remote.device_count),
                    TuiLanguage::Fr => format!("{} appareils", remote.device_count),
                    TuiLanguage::En => format!("{} devices", remote.device_count),
                }
            };
            match lang {
                TuiLanguage::Ko => format!("로컬+원격 ({})", devices),
                TuiLanguage::Ja => format!("ローカル+リモート ({})", devices),
                TuiLanguage::ZhCn => format!("本地+远程 ({})", devices),
                TuiLanguage::Fr => format!("local+distant ({})", devices),
                TuiLanguage::En => format!("local+remote ({})", devices),
            }
        }
        None => tr(lang, MessageKey::StatusLocal).to_string(),
    }
}

fn render_status_row(frame: &mut Frame, app: &App, area: Rect) {
    let lang = app.settings.tui_language;
    let mut spans: Vec<Span> = Vec::new();

    // Always-visible data-source indicator, so it is clear whether the
    // numbers on screen are local-only or backed by server-side aggregates.
    spans.push(Span::styled(
        data_source_label(app),
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    if let Some(ref remote) = app.remote_stats {
        spans.push(Span::styled(
            format!(
                "{}{} · {}",
                tr(lang, MessageKey::StatusAllDevices),
                format_tokens(remote.total_tokens),
                format_cost(remote.total_cost)
            ),
            Style::default().fg(app.theme.muted),
        ));
    }
    spans.push(Span::styled(" • ", Style::default().fg(app.theme.muted)));

    if app.data.loading {
        let scanner_spans = get_scanner_spans(app.spinner_frame, &app.theme);
        spans.extend(scanner_spans);
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            get_phase_message("parsing-sources", lang),
            Style::default().fg(app.theme.muted),
        ));
    } else if app.background_loading {
        if app.has_visible_data() {
            spans.push(Span::styled(
                tr(lang, MessageKey::StatusRefreshingBackground),
                Style::default().fg(app.theme.muted),
            ));
        } else {
            let scanner_spans = get_scanner_spans(app.spinner_frame, &app.theme);
            spans.extend(scanner_spans);
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                get_phase_message("parsing-sources", lang),
                Style::default().fg(app.theme.muted),
            ));
        }
    } else if let Some(ref msg) = app.status_message {
        spans.push(Span::styled(
            msg.clone(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        let elapsed = app.last_refresh.elapsed();
        let ago = if elapsed.as_secs() < 60 {
            match lang {
                TuiLanguage::Ko => format!("{}초 전", elapsed.as_secs()),
                TuiLanguage::Ja => format!("{}秒前", elapsed.as_secs()),
                TuiLanguage::ZhCn => format!("{}秒前", elapsed.as_secs()),
                TuiLanguage::Fr => format!("il y a {}s", elapsed.as_secs()),
                TuiLanguage::En => format!("{}s ago", elapsed.as_secs()),
            }
        } else if elapsed.as_secs() < 3600 {
            match lang {
                TuiLanguage::Ko => format!("{}분 전", elapsed.as_secs() / 60),
                TuiLanguage::Ja => format!("{}分前", elapsed.as_secs() / 60),
                TuiLanguage::ZhCn => format!("{}分钟前", elapsed.as_secs() / 60),
                TuiLanguage::Fr => format!("il y a {}m", elapsed.as_secs() / 60),
                TuiLanguage::En => format!("{}m ago", elapsed.as_secs() / 60),
            }
        } else {
            match lang {
                TuiLanguage::Ko => format!("{}시간 전", elapsed.as_secs() / 3600),
                TuiLanguage::Ja => format!("{}時間前", elapsed.as_secs() / 3600),
                TuiLanguage::ZhCn => format!("{}小时前", elapsed.as_secs() / 3600),
                TuiLanguage::Fr => format!("il y a {}h", elapsed.as_secs() / 3600),
                TuiLanguage::En => format!("{}h ago", elapsed.as_secs() / 3600),
            }
        };
        spans.push(Span::styled(
            format!("{}{}", tr(lang, MessageKey::StatusLastUpdated), ago),
            Style::default().fg(app.theme.muted),
        ));

        if app.auto_refresh {
            spans.push(Span::styled(
                format!(" • Auto: {}s", app.auto_refresh_interval.as_secs()),
                Style::default().fg(app.theme.muted),
            ));
        }
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::TuiConfig;
    use crate::tui::data::UsageData;

    fn make_app_on(tab: Tab) -> App {
        let config = TuiConfig {
            theme: "blue".to_string(),
            refresh: 0,
            sessions_path: None,
            clients: None,
            since: None,
            until: None,
            year: None,
            initial_tab: Some(tab),
            ..Default::default()
        };
        let mut app = App::new_with_cached_data(config, Some(UsageData::default())).unwrap();
        app.settings.tui_language = crate::tui::i18n::TuiLanguage::En;
        app
    }

    #[test]
    fn test_current_count_label_matches_active_tab() {
        assert_eq!(
            current_count_label(&make_app_on(Tab::Models)),
            " (0 models)"
        );
        assert_eq!(
            current_count_label(&make_app_on(Tab::Agents)),
            " (0 agents)"
        );
        assert_eq!(current_count_label(&make_app_on(Tab::Daily)), " (0 days)");
        assert_eq!(current_count_label(&make_app_on(Tab::Hourly)), " (0 hours)");
        assert_eq!(
            current_count_label(&make_app_on(Tab::Monthly)),
            " (0 months)"
        );
        assert_eq!(
            current_count_label(&make_app_on(Tab::Sessions)),
            " (0 sessions)"
        );
        assert_eq!(current_count_label(&make_app_on(Tab::Stats)), "");
    }

    #[test]
    fn test_current_count_label_minutely_when_flag_enabled() {
        let mut app = make_app_on(Tab::Models);
        app.settings.minutely_tab_enabled = true;
        app.current_tab = Tab::Minutely;
        assert_eq!(current_count_label(&app), " (0 minutes)");
    }

    #[test]
    fn test_data_source_label_local_without_remote_stats() {
        let app = make_app_on(Tab::Models);
        assert_eq!(data_source_label(&app), "local");
    }

    #[test]
    fn test_data_source_label_with_remote_stats() {
        let mut app = make_app_on(Tab::Models);
        app.remote_stats = Some(crate::tui::remote::RemoteStats {
            schema_version: 1,
            total_tokens: 1250,
            total_cost: 1.75,
            device_count: 2,
            last_submitted_at: None,
            days: Vec::new(),
            devices: Vec::new(),
            fetched_at_secs: 0,
            cached_for_user: "alice".to_string(),
            cached_for_api_url: "https://tokscale.ai".to_string(),
        });
        assert_eq!(data_source_label(&app), "local+remote (2 devices)");

        app.remote_stats.as_mut().unwrap().device_count = 1;
        assert_eq!(data_source_label(&app), "local+remote (1 device)");
    }
}
