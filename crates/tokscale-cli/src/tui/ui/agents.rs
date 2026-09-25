use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::widgets::{
    ambient_stable_scrollbar, format_cost, get_client_display_name, total_tokens_cell,
    truncate_text, viewport_scrollbar_state, AMBIENT_STABLE_BORDER_SET,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};
use crate::ClientFilter;

/// The Agents table's header labels for a layout, in display order.
///
/// A function rather than an inline `vec!` so
/// `every_language_renders_its_full_header_once_the_layout_fits` asserts against
/// the labels the renderer actually writes. A test that restates the label set
/// passes while the renderer uses a different one, which is how the Korean
/// `메시지` clip in `sessions.rs` survived a header test that already existed.
fn header_labels(lang: TuiLanguage, is_narrow: bool, is_very_narrow: bool) -> Vec<&'static str> {
    if is_very_narrow {
        return vec![
            tr(lang, MessageKey::ColAgent),
            tr(lang, MessageKey::ColCost),
        ];
    }
    if is_narrow {
        return vec![
            tr(lang, MessageKey::ColAgent),
            tr(lang, MessageKey::ColTokens),
            tr(lang, MessageKey::ColCost),
        ];
    }
    vec![
        tr(lang, MessageKey::ColRank),
        tr(lang, MessageKey::ColAgent),
        tr(lang, MessageKey::ColSource),
        tr(lang, MessageKey::ColTokens),
        tr(lang, MessageKey::ColCost),
        tr(lang, MessageKey::ColMessagesShort),
    ]
}

/// The Agents table's column widths, index-aligned with [`header_labels`].
///
/// A function rather than an inline `vec!` so
/// `no_header_overflows_its_budget_in_any_language` checks the widths the
/// renderer actually lays out with, not a copy of them. The narrow layouts are
/// percentage-based and have no declared budget; the wide one is `Length` (plus
/// a flexible Agent column), so a header longer than its `Length` is a clip the
/// arithmetic can name.
fn header_widths(is_narrow: bool, is_very_narrow: bool) -> Vec<Constraint> {
    if is_very_narrow {
        return vec![Constraint::Percentage(70), Constraint::Percentage(30)];
    }
    if is_narrow {
        return vec![
            Constraint::Percentage(45),
            Constraint::Percentage(27),
            Constraint::Percentage(28),
        ];
    }
    vec![
        Constraint::Length(3),
        Constraint::Min(24),
        Constraint::Length(24),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(6),
    ]
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            format!(" {} ", tr(lang, MessageKey::TabAgents)),
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
    let theme_accent = app.theme.accent;
    let theme_muted = app.theme.muted;
    let theme_selection = app.theme.selection;
    let striped_row_style = app.theme.striped_row_style();

    let agents = app.get_sorted_agents();
    if agents.is_empty() {
        let empty_msg = Paragraph::new(get_empty_message(app))
            .style(Style::default().fg(theme_muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let lang = app.settings.tui_language;
    let header_cells = header_labels(lang, is_narrow, is_very_narrow);

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
                    3 if !is_narrow => sort_indicator(SortField::Tokens),
                    4 if !is_narrow => sort_indicator(SortField::Cost),
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

    let agents_len = agents.len();
    let start = scroll_offset.min(agents_len.saturating_sub(1));
    let end = (start + visible_height).min(agents_len);

    if start >= agents_len {
        return;
    }

    let rows: Vec<Row> = agents[start..end]
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if is_very_narrow {
                vec![
                    Cell::from(truncate_text(&agent.agent, 18))
                        .style(Style::default().fg(app.theme.foreground)),
                    Cell::from(format_cost(agent.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else if is_narrow {
                vec![
                    Cell::from(truncate_text(&agent.agent, 18))
                        .style(Style::default().fg(app.theme.foreground)),
                    total_tokens_cell(agent.tokens.total(), &app.theme),
                    Cell::from(format_cost(agent.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                vec![
                    Cell::from(format!("{}", idx + 1)).style(Style::default().fg(theme_muted)),
                    Cell::from(truncate_text(&agent.agent, 32)).style(
                        Style::default()
                            .fg(app.theme.foreground)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(truncate_text(&client_labels(&agent.clients), 24))
                        .style(Style::default().fg(theme_muted)),
                    total_tokens_cell(agent.tokens.total(), &app.theme),
                    Cell::from(format_cost(agent.cost)).style(Style::default().fg(Color::Green)),
                    Cell::from(agent.message_count.to_string())
                        .style(Style::default().fg(theme_muted)),
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

    let widths = header_widths(is_narrow, is_very_narrow);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if agents_len > visible_height {
        let scrollbar = ambient_stable_scrollbar();

        let mut scrollbar_state =
            viewport_scrollbar_state(agents_len, scroll_offset, visible_height);

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

fn get_empty_message(app: &App) -> String {
    let lang = app.settings.tui_language;
    let enabled_clients = app.enabled_clients.borrow();
    let only_codex = !enabled_clients.is_empty()
        && enabled_clients
            .iter()
            .all(|client| *client == ClientFilter::Codex);

    if only_codex {
        tr(lang, MessageKey::EmptyNoAgentCodex).to_string()
    } else {
        tr(lang, MessageKey::EmptyNoAgentMixed).to_string()
    }
}

fn client_labels(clients: &str) -> String {
    clients
        .split(", ")
        .map(get_client_display_name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::{get_empty_message, header_labels, header_widths, render};
    use crate::tui::app::{App, SortDirection, SortField, Tab, TuiConfig};
    use crate::tui::data::{AgentUsage, TokenBreakdown, UsageData};
    use crate::tui::i18n::TuiLanguage;
    use crate::tui::ui::header_budget::{assert_header_layout_fits, assert_headers_render_in_full};
    use crate::ClientFilter;
    use ratatui::layout::{Constraint, Rect};
    use ratatui::{backend::TestBackend, Terminal};

    fn make_app(clients: Vec<ClientFilter>) -> App {
        let mut app = App::new_with_cached_data(
            TuiConfig {
                theme: "tokscale".to_string(),
                refresh: 0,
                sessions_path: None,
                clients: None,
                since: None,
                until: None,
                year: None,
                initial_tab: None,
                ..Default::default()
            },
            Some(UsageData::default()),
        )
        .unwrap();

        app.settings.tui_language = crate::tui::i18n::TuiLanguage::En;
        *app.enabled_clients.borrow_mut() = clients.into_iter().collect();
        app
    }

    #[test]
    fn test_get_empty_message_for_codex_only() {
        let app = make_app(vec![ClientFilter::Codex]);
        let message = get_empty_message(&app);

        assert!(message.contains("selected source usually does not record"));
        assert!(message.contains("try a different source"));
    }

    #[test]
    fn test_get_empty_message_for_mixed_sources() {
        let app = make_app(vec![ClientFilter::Opencode, ClientFilter::Roocode]);
        let message = get_empty_message(&app);

        assert!(message.contains("Only some sources record agent metadata"));
        assert!(message.contains("change sources"));
    }

    /// One agent row, so the table has something to render a header above.
    fn agent(name: &str) -> AgentUsage {
        AgentUsage {
            agent: name.to_string(),
            clients: "claude-code".to_string(),
            tokens: TokenBreakdown::default(),
            cost: 1.5,
            message_count: 7,
        }
    }

    fn make_table_app(width: u16) -> App {
        let mut app = make_app(vec![ClientFilter::Claude]);
        app.terminal_width = width;
        app.current_tab = Tab::Agents;
        app.sort_field = SortField::Tokens;
        app.sort_direction = SortDirection::Descending;
        app.data.agents = vec![agent("plan-writer"), agent("code-reviewer")];
        app
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
                    .map(|c| c.symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The rendered header row of the Agents table.
    fn header_for(lang: TuiLanguage, width: u16) -> String {
        let mut app = make_table_app(width);
        app.settings.tui_language = lang;
        render_body(&mut app, width, 10)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    /// Which wide-layout columns carry a sort arrow, index-aligned with
    /// `header_labels`. Read straight off the `sort_indicator` match in
    /// `render`: in the wide branch (`!is_narrow`) only `3 => Tokens` and
    /// `4 => Cost` match, so Tokens and Cost carry the arrows and the other four
    /// columns carry none.
    const WIDE_SORTABLE: [bool; 6] = [false, false, false, true, true, false];

    /// No header label exceeds the `Constraint::Length` its own layout declares,
    /// in any language.
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
    /// renderer uses something else.
    #[test]
    fn no_header_overflows_its_budget_in_any_language() {
        for lang in TuiLanguage::ALL {
            assert_header_layout_fits(
                "agents/wide",
                lang,
                &header_labels(lang, false, false),
                &header_widths(false, false),
                &WIDE_SORTABLE,
            );
        }
    }

    /// At and above the width its own constraints add up to, the wide layout gets
    /// every cell it asked for, so every header must render in full — in every
    /// language. Below that ratatui shrinks every column and clips English
    /// headers too, which is the pre-existing #964-class over-ask this tab never
    /// solved, not a localization defect.
    ///
    /// The Agent column is a `Min`, unbounded above, so the fitting width is
    /// measured from the `Length`s plus Agent's floor.
    #[test]
    fn every_language_renders_its_full_header_once_the_layout_fits() {
        let widths = header_widths(false, false);
        let fixed: u16 = widths
            .iter()
            .map(|constraint| match constraint {
                Constraint::Length(cells) => *cells,
                // The sole `Min`, which is Agent's floor.
                Constraint::Min(cells) => *cells,
                other => unreachable!("unexpected constraint {other:?}"),
            })
            .sum();
        // + one separator between each pair, + 2 for the block borders.
        let needed = fixed + widths.len().saturating_sub(1) as u16 + 2;
        for width in [needed, needed + 20, 200] {
            for lang in TuiLanguage::ALL {
                assert_headers_render_in_full(
                    &format!("agents(width={width})"),
                    lang,
                    &header_for(lang, width),
                    &header_labels(lang, false, false),
                );
            }
        }
    }
}
