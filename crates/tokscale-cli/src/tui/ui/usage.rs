use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::commands::usage::{helpers, UsageMetric, UsageOutput};
use crate::tui::app::{App, ClickAction, CodexLoginOutcome};

const BAR_WIDTH: usize = 20;

struct ButtonSpec {
    label: &'static str,
    style: Style,
    action: ClickAction,
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(" Subscription Usage ")
        .title_style(Style::default().fg(app.theme.foreground))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content = render_action_bar(frame, app, inner);
    let content = render_codex_login_panel(frame, app, content);

    if app.subscription_usage.is_empty() {
        if app.is_fetching_usage() {
            render_fetching(frame, app, content);
        } else if app.usage_fetch_attempted {
            render_empty(frame, app, content);
        } else {
            render_loading(frame, app, content);
        }
    } else if app.subscription_usage.iter().all(|o| o.metrics.is_empty()) {
        render_empty(frame, app, content);
    } else {
        let outputs = app.subscription_usage.clone();
        render_loaded(frame, app, content, &outputs);
    }
}

fn render_action_bar(frame: &mut Frame, app: &mut App, area: Rect) -> Rect {
    if area.height == 0 {
        return area;
    }

    let refresh_label = if app.is_fetching_usage() {
        "Refreshing"
    } else {
        "Refresh"
    };
    let refresh_style = if app.is_fetching_usage() {
        Style::default().fg(app.theme.muted)
    } else {
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD)
    };

    let add_label = if app.is_codex_login_running() {
        "Adding Codex"
    } else {
        "Add Codex"
    };
    let add_style = if app.is_codex_login_running() {
        Style::default().fg(app.theme.muted)
    } else {
        Style::default().fg(app.theme.accent)
    };

    let buttons = vec![
        ButtonSpec {
            label: refresh_label,
            style: refresh_style,
            action: ClickAction::UsageRefresh,
        },
        ButtonSpec {
            label: add_label,
            style: add_style,
            action: ClickAction::CodexStartLogin,
        },
    ];

    let mut spans = vec![Span::raw(" ")];
    let right_edge = area.x.saturating_add(area.width);
    push_click_buttons(
        &mut spans,
        app,
        buttons,
        area.x.saturating_add(1),
        area.y,
        right_edge,
    );

    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, area.width, 1),
    );

    if area.height > 1 {
        Rect::new(area.x, area.y + 1, area.width, area.height - 1)
    } else {
        Rect::new(area.x, area.y, area.width, 0)
    }
}

fn button_render_width(label: &str) -> usize {
    label.chars().count() + 2
}

fn click_buttons_width(buttons: &[ButtonSpec]) -> usize {
    buttons
        .iter()
        .enumerate()
        .map(|(index, button)| button_render_width(button.label) + usize::from(index > 0))
        .sum()
}

fn push_click_buttons(
    spans: &mut Vec<Span<'static>>,
    app: &mut App,
    buttons: Vec<ButtonSpec>,
    start_x: u16,
    y: u16,
    right_edge: u16,
) {
    let mut x = start_x;
    for (index, button) in buttons.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
            x = x.saturating_add(1);
        }

        let rendered = format!("[{}]", button.label);
        let width = rendered.chars().count() as u16;
        spans.push(Span::styled(rendered, button.style));

        if x < right_edge {
            app.add_click_area(Rect::new(x, y, width.min(right_edge - x), 1), button.action);
        }
        x = x.saturating_add(width);
    }
}

fn render_codex_login_panel(frame: &mut Frame, app: &mut App, area: Rect) -> Rect {
    if area.height == 0 || !app.should_show_codex_login_panel() {
        return area;
    }

    let max_output_lines = 5usize;
    let output_start = app.codex_login_lines.len().saturating_sub(max_output_lines);
    let output_lines: Vec<String> = app.codex_login_lines[output_start..].to_vec();
    let height = (2 + output_lines.len() as u16 + u16::from(app.codex_login_outcome.is_some()))
        .min(area.height);
    if height == 0 {
        return area;
    }

    let mut lines: Vec<Line> = Vec::new();
    let status = match &app.codex_login_outcome {
        Some(CodexLoginOutcome::Imported(_)) => "Imported",
        Some(CodexLoginOutcome::Failed(_)) => "Failed",
        None if app.is_codex_login_running() => "Running",
        None => "Idle",
    };

    let mut header_spans = vec![
        Span::styled(
            " Codex login ",
            Style::default()
                .fg(app.theme.foreground)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(status, Style::default().fg(app.theme.muted)),
    ];
    if app.codex_login_outcome.is_some() {
        let dismiss = "[Dismiss]";
        let used_width = 14usize + status.chars().count();
        let dismiss_width = dismiss.chars().count();
        let dismiss_click_width = (dismiss_width as u16).min(area.width);
        let padding = (area.width as usize).saturating_sub(used_width + dismiss_width);
        header_spans.push(Span::raw(" ".repeat(padding)));
        header_spans.push(Span::styled(dismiss, Style::default().fg(app.theme.accent)));
        let x = area
            .x
            .saturating_add(area.width.saturating_sub(dismiss_click_width));
        if dismiss_click_width > 0 {
            app.add_click_area(
                Rect::new(x, area.y, dismiss_click_width, 1),
                ClickAction::CodexDismissLogin,
            );
        }
    }
    lines.push(Line::from(header_spans));

    if output_lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Waiting for codex output...",
            Style::default().fg(app.theme.muted),
        )));
    } else {
        for line in output_lines {
            lines.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(app.theme.muted),
            )));
        }
    }

    if let Some(outcome) = &app.codex_login_outcome {
        let (label, style) = match outcome {
            CodexLoginOutcome::Imported(info) => (
                format!(
                    "  Imported {}",
                    info.label.as_deref().unwrap_or(info.id.as_str())
                ),
                Style::default().fg(app.theme.accent),
            ),
            CodexLoginOutcome::Failed(error) => {
                (format!("  {error}"), Style::default().fg(Color::Red))
            }
        };
        lines.push(Line::from(Span::styled(label, style)));
    }

    frame.render_widget(
        Paragraph::new(lines),
        Rect::new(area.x, area.y, area.width, height),
    );

    if area.height > height {
        Rect::new(area.x, area.y + height, area.width, area.height - height)
    } else {
        Rect::new(area.x, area.y, area.width, 0)
    }
}

fn render_fetching(frame: &mut Frame, app: &App, area: Rect) {
    let center = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(3),
            Constraint::Percentage(40),
        ])
        .split(area)[1];

    let spin = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][app.spinner_frame % 10];
    let paragraph = Paragraph::new(format!("{spin} Fetching subscription data..."))
        .style(Style::default().fg(app.theme.muted))
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, center);
}

fn render_loading(frame: &mut Frame, app: &App, area: Rect) {
    let center = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(3),
            Constraint::Percentage(40),
        ])
        .split(area)[1];

    let msg = if app.data.loading {
        "Loading subscription data..."
    } else {
        "Use Refresh to fetch subscription usage"
    };
    let paragraph = Paragraph::new(msg)
        .style(Style::default().fg(app.theme.muted))
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, center);
}

fn render_empty(frame: &mut Frame, app: &App, area: Rect) {
    let center = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(3),
            Constraint::Percentage(40),
        ])
        .split(area)[1];

    let paragraph = Paragraph::new("No subscription data available")
        .style(Style::default().fg(app.theme.muted))
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, center);
}

fn render_loaded(frame: &mut Frame, app: &mut App, area: Rect, outputs: &[UsageOutput]) {
    let mut lines: Vec<Line> = Vec::new();
    let groups = group_outputs_by_provider(outputs);

    for (group_index, group) in groups.iter().enumerate() {
        if group_index > 0 {
            lines.push(Line::from(""));
        }

        let account_group = group.outputs.iter().any(|output| output.account.is_some());

        if account_group {
            push_provider_header(&mut lines, app, group.provider);

            for (output_index, output) in group.outputs.iter().enumerate() {
                if output_index > 0 {
                    lines.push(Line::from(""));
                }
                push_account_header(&mut lines, app, output, area);
                push_output_details(
                    &mut lines,
                    app,
                    output,
                    "   ",
                    "Email",
                    account_header_uses_email(output),
                );
            }
        } else {
            for (output_index, output) in group.outputs.iter().enumerate() {
                if output_index > 0 {
                    lines.push(Line::from(""));
                }
                push_provider_header(&mut lines, app, &output.display_name());
                push_output_details(&mut lines, app, output, " ", "Account", false);
            }
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

struct UsageProviderGroup<'a> {
    provider: &'a str,
    outputs: Vec<&'a UsageOutput>,
}

fn group_outputs_by_provider(outputs: &[UsageOutput]) -> Vec<UsageProviderGroup<'_>> {
    let mut groups: Vec<UsageProviderGroup<'_>> = Vec::new();

    for output in outputs {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.provider == output.provider)
        {
            group.outputs.push(output);
        } else {
            groups.push(UsageProviderGroup {
                provider: &output.provider,
                outputs: vec![output],
            });
        }
    }

    groups
}

fn push_provider_header(lines: &mut Vec<Line>, app: &App, label: &str) {
    lines.push(Line::from(Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(app.theme.foreground)
            .add_modifier(Modifier::BOLD),
    )));
}

fn push_account_header(lines: &mut Vec<Line>, app: &mut App, output: &UsageOutput, area: Rect) {
    let (name, is_active) = match &output.account {
        Some(account) => (
            output.account_display_name().unwrap_or_default(),
            account.is_active,
        ),
        None => (output.display_name(), false),
    };
    let name_width = name.chars().count();
    let marker = if is_active { "*" } else { "-" };
    let marker_style = if is_active {
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(app.theme.muted)
    };
    let name_style = if is_active {
        Style::default()
            .fg(app.theme.foreground)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(app.theme.foreground)
    };

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(marker, marker_style),
        Span::raw(" "),
        Span::styled(name, name_style),
    ];

    if let Some(account) = &output.account {
        let y = area.y.saturating_add(lines.len() as u16);
        let left_width = 3usize.saturating_add(name_width);
        let pending_remove =
            app.pending_codex_remove_account_id.as_deref() == Some(account.id.as_str());

        let mut buttons: Vec<ButtonSpec> = Vec::new();
        if !account.is_active {
            buttons.push(ButtonSpec {
                label: "Use",
                style: Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
                action: ClickAction::CodexUseAccount {
                    account_id: account.id.clone(),
                },
            });
        }

        buttons.push(ButtonSpec {
            label: if pending_remove { "Confirm" } else { "Remove" },
            style: Style::default().fg(if pending_remove {
                Color::Red
            } else {
                app.theme.muted
            }),
            action: ClickAction::CodexRemoveAccount {
                account_id: account.id.clone(),
            },
        });

        let button_width = click_buttons_width(&buttons);
        let padding = (area.width as usize).saturating_sub(left_width + button_width);
        spans.push(Span::raw(" ".repeat(padding)));

        let x = area
            .x
            .saturating_add(left_width as u16)
            .saturating_add(padding as u16);
        let right_edge = area.x.saturating_add(area.width);
        push_click_buttons(&mut spans, app, buttons, x, y, right_edge);
    }

    lines.push(Line::from(spans));
}

fn push_output_details(
    lines: &mut Vec<Line>,
    app: &App,
    output: &UsageOutput,
    indent: &str,
    email_label: &str,
    skip_email: bool,
) {
    for metric in &output.metrics {
        lines.push(metric_line(app, metric, indent));
    }

    if !skip_email {
        if let Some(ref email) = output.email {
            push_metadata_line(lines, app, indent, email_label, email);
        }
    }
    if let Some(ref plan) = output.plan {
        push_metadata_line(lines, app, indent, "Plan", plan);
    }
}

fn account_header_uses_email(output: &UsageOutput) -> bool {
    let Some(account) = &output.account else {
        return false;
    };
    if account.label_name().is_some() {
        return false;
    }

    output
        .email
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .is_some()
}

fn metric_line(app: &App, metric: &UsageMetric, indent: &str) -> Line<'static> {
    let remaining = metric
        .remaining_label
        .clone()
        .unwrap_or_else(|| format!("{:.0}% left", metric.remaining_percent));
    let bar = helpers::render_ascii_bar(metric.remaining_percent, BAR_WIDTH);
    let reset = metric
        .resets_at
        .as_ref()
        .map(|r| helpers::format_reset_time(r))
        .unwrap_or_default();

    let label = Span::styled(
        format!("{indent}{:<14}", metric.label),
        Style::default().fg(app.theme.foreground),
    );
    let value = Span::styled(
        format!("{:<11}", remaining),
        Style::default().fg(app.theme.foreground),
    );
    let bar_span = Span::styled(
        format!("{:<24}", bar),
        Style::default().fg(if metric.remaining_percent < 10.0 {
            Color::Red
        } else if metric.remaining_percent < 25.0 {
            Color::Yellow
        } else {
            app.theme.accent
        }),
    );
    let reset_span = Span::styled(reset, Style::default().fg(app.theme.muted));

    Line::from(vec![label, value, bar_span, reset_span])
}

fn push_metadata_line(lines: &mut Vec<Line>, app: &App, indent: &str, label: &str, value: &str) {
    lines.push(Line::from(Span::styled(
        format!("{indent}{:<12}{value}", label),
        Style::default().fg(app.theme.muted),
    )));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::usage::{UsageAccount, UsageMetric};
    use crate::tui::app::TuiConfig;
    use crate::tui::data::UsageData;
    use ratatui::{backend::TestBackend, Terminal};

    fn output(provider: &str, account: Option<UsageAccount>) -> UsageOutput {
        UsageOutput {
            provider: provider.to_string(),
            account,
            plan: None,
            email: None,
            metrics: vec![UsageMetric {
                label: "Session".to_string(),
                used_percent: 10.0,
                remaining_percent: 90.0,
                remaining_label: Some("90% left".to_string()),
                resets_at: None,
            }],
        }
    }

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
        };
        App::new_with_cached_data(config, Some(UsageData::default())).unwrap()
    }

    fn render_body(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, app, Rect::new(0, 0, width, height)))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn groups_usage_outputs_by_provider_preserving_first_seen_order() {
        let outputs = vec![
            output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_work".to_string(),
                    label: Some("work".to_string()),
                    is_active: true,
                }),
            ),
            output("Claude", None),
            output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_personal".to_string(),
                    label: Some("personal".to_string()),
                    is_active: false,
                }),
            ),
        ];

        let groups = group_outputs_by_provider(&outputs);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].provider, "Codex");
        assert_eq!(groups[0].outputs.len(), 2);
        assert_eq!(groups[1].provider, "Claude");
        assert_eq!(groups[1].outputs.len(), 1);
    }

    #[test]
    fn renders_codex_accounts_under_one_provider_section() {
        let mut app = make_app();
        app.subscription_usage = vec![
            output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_work".to_string(),
                    label: Some("work".to_string()),
                    is_active: true,
                }),
            ),
            output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_personal".to_string(),
                    label: Some("personal".to_string()),
                    is_active: false,
                }),
            ),
            output("Claude", None),
        ];

        let body = render_body(&mut app, 100, 18);

        assert_eq!(
            body.matches(" Codex ").count(),
            1,
            "Codex provider should render once for grouped accounts\n{body}"
        );
        assert!(body.contains("* work"), "active account missing\n{body}");
        assert!(
            body.contains("- personal"),
            "inactive account missing\n{body}"
        );
        assert!(
            body.contains(" Claude "),
            "other providers still render\n{body}"
        );
    }

    #[test]
    fn renders_codex_account_email_instead_of_raw_account_id() {
        let mut app = make_app();
        let raw_id = "123e4567-e89b-12d3-a456-426614174000";
        app.subscription_usage = vec![UsageOutput {
            provider: "Codex".to_string(),
            account: Some(UsageAccount {
                id: raw_id.to_string(),
                label: None,
                is_active: true,
            }),
            plan: None,
            email: Some("user@example.com".to_string()),
            metrics: vec![UsageMetric {
                label: "Session".to_string(),
                used_percent: 10.0,
                remaining_percent: 90.0,
                remaining_label: Some("90% left".to_string()),
                resets_at: None,
            }],
        }];

        let body = render_body(&mut app, 100, 10);

        assert!(body.contains("* user@example.com"), "email missing\n{body}");
        assert!(!body.contains(raw_id), "raw id leaked\n{body}");
    }

    #[test]
    fn registers_usage_refresh_and_codex_account_click_areas() {
        let mut app = make_app();
        app.subscription_usage = vec![
            output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_work".to_string(),
                    label: Some("work".to_string()),
                    is_active: true,
                }),
            ),
            output(
                "Codex",
                Some(UsageAccount {
                    id: "acct_personal".to_string(),
                    label: Some("personal".to_string()),
                    is_active: false,
                }),
            ),
        ];

        let body = render_body(&mut app, 100, 18);

        assert!(body.contains("[Refresh]"), "refresh button missing\n{body}");
        assert!(body.contains("[Add Codex]"), "add button missing\n{body}");
        assert!(body.contains("* work"), "active marker missing\n{body}");
        assert!(body.contains("[Use]"), "use button missing\n{body}");
        assert!(body.contains("[Remove]"), "remove button missing\n{body}");
        assert!(app
            .click_areas
            .iter()
            .any(|area| matches!(area.action, ClickAction::UsageRefresh)));
        assert!(app
            .click_areas
            .iter()
            .any(|area| matches!(area.action, ClickAction::CodexStartLogin)));
        assert!(app.click_areas.iter().any(|area| matches!(
            &area.action,
            ClickAction::CodexUseAccount { account_id } if account_id == "acct_personal"
        )));
        assert!(app.click_areas.iter().any(|area| matches!(
            &area.action,
            ClickAction::CodexRemoveAccount { account_id } if account_id == "acct_work"
        )));
    }

    #[test]
    fn renders_confirm_for_pending_codex_removal() {
        let mut app = make_app();
        app.pending_codex_remove_account_id = Some("acct_work".to_string());
        app.subscription_usage = vec![output(
            "Codex",
            Some(UsageAccount {
                id: "acct_work".to_string(),
                label: Some("work".to_string()),
                is_active: true,
            }),
        )];

        let body = render_body(&mut app, 100, 10);

        assert!(body.contains("[Confirm]"), "confirm button missing\n{body}");
        assert!(
            !body.contains("[Remove]"),
            "stale remove button shown\n{body}"
        );
    }

    #[test]
    fn renders_codex_login_panel_output_and_dismiss_action() {
        let mut app = make_app();
        app.codex_login_lines = vec![
            "Open https://example.com/device".to_string(),
            "Code ABCD-EFGH".to_string(),
        ];
        app.codex_login_outcome = Some(CodexLoginOutcome::Failed("expired".to_string()));

        let body = render_body(&mut app, 100, 12);

        assert!(body.contains("Codex login"), "login panel missing\n{body}");
        assert!(
            body.contains("Open https://example.com/device"),
            "login output missing\n{body}"
        );
        assert!(body.contains("expired"), "login error missing\n{body}");
        assert!(body.contains("[Dismiss]"), "dismiss button missing\n{body}");
        assert!(app
            .click_areas
            .iter()
            .any(|area| matches!(area.action, ClickAction::CodexDismissLogin)));
    }

    #[test]
    fn codex_login_dismiss_click_area_stays_inside_narrow_panel() {
        let mut app = make_app();
        app.codex_login_outcome = Some(CodexLoginOutcome::Failed("expired".to_string()));

        let _body = render_body(&mut app, 6, 8);

        let dismiss_area = app
            .click_areas
            .iter()
            .find(|area| matches!(area.action, ClickAction::CodexDismissLogin))
            .expect("dismiss click area");
        assert!(
            dismiss_area.rect.x + dismiss_area.rect.width <= 6,
            "dismiss click area exceeds panel: {:?}",
            dismiss_area.rect
        );
    }
}
