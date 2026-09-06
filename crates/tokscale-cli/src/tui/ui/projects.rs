use chrono::{Local, NaiveDateTime, TimeZone};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table,
};

use super::widgets::{
    display_width, fit_workspace_label_to_width, format_cost, format_tokens,
    get_compact_client_display_name, prefix_to_width, total_tokens_cell, truncate_text,
    truncate_to_width, viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::data::{ProjectUsage, SessionModel};

/// One column of the wide Projects layout, in left-to-right display order.
///
/// Every per-column fact hangs off this enum so the header, cells, constraints
/// and truncation budgets cannot drift apart: each method is an exhaustive
/// `match` with no `_` arm, so adding a variant fails to compile until every
/// one of them has an answer for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectColumn {
    Rank,
    Project,
    Sessions,
    Sources,
    Models,
    Input,
    Output,
    CacheRead,
    CacheWrite,
    Total,
    Cost,
    LastActive,
}

/// Every variant in display order; the single source of the column set.
const WIDE_ORDER: [ProjectColumn; 12] = [
    ProjectColumn::Rank,
    ProjectColumn::Project,
    ProjectColumn::Sessions,
    ProjectColumn::Sources,
    ProjectColumn::Models,
    ProjectColumn::Input,
    ProjectColumn::Output,
    ProjectColumn::CacheRead,
    ProjectColumn::CacheWrite,
    ProjectColumn::Total,
    ProjectColumn::Cost,
    ProjectColumn::LastActive,
];
const COLUMN_SPACING: u16 = 1;

impl ProjectColumn {
    fn header(self) -> &'static str {
        match self {
            Self::Rank => "#",
            Self::Project => "Project",
            Self::Sessions => "Sessions",
            Self::Sources => "Sources",
            Self::Models => "Models",
            Self::Input => "Input",
            Self::Output => "Output",
            Self::CacheRead => "Cache Read",
            Self::CacheWrite => "Cache Write",
            Self::Total => "Total",
            Self::Cost => "Cost",
            Self::LastActive => "Last Active",
        }
    }

    /// Cells this column asks the solver for. Project is the sole flexible
    /// (`Min`) column and absorbs whatever slack or shrinkage the row has left;
    /// every other column is a fixed `Length` at its natural width.
    fn constraint(self) -> Constraint {
        if self == Self::Project {
            Constraint::Min(self.min_width())
        } else {
            Constraint::Length(self.min_width())
        }
    }

    fn min_width(self) -> u16 {
        match self {
            Self::Project | Self::Models => 18,
            Self::Rank => 4,
            Self::Sessions => 8,
            Self::Sources => 14,
            Self::Input | Self::Output | Self::CacheRead | Self::Total | Self::Cost => 10,
            Self::CacheWrite => 11,
            Self::LastActive => 16,
        }
    }

    /// The sort this column is the target of, so the indicator is placed by
    /// identity rather than by a computed index.
    fn sort_field(self) -> Option<SortField> {
        match self {
            Self::Total => Some(SortField::Tokens),
            Self::Cost => Some(SortField::Cost),
            Self::LastActive => Some(SortField::Date),
            Self::Rank
            | Self::Project
            | Self::Sessions
            | Self::Sources
            | Self::Models
            | Self::Input
            | Self::Output
            | Self::CacheRead
            | Self::CacheWrite => None,
        }
    }

    fn cell(self, rank: usize, p: &ProjectUsage, app: &App, granted: &[u16]) -> Cell<'static> {
        match self {
            Self::Rank => Cell::from(self.fit(rank.to_string(), granted))
                .style(Style::default().fg(app.theme.muted)),
            // Not a plain head cut: a workspace label is identified by the ends
            // of each of its segments, and cutting the tail leaves the prefix
            // every row shares. Truncated to the width the solver granted this
            // very column, never to the requested width.
            Self::Project => Cell::from(fit_workspace_label_to_width(
                &p.label,
                self.granted_width(granted),
            ))
            .style(
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Self::Sessions => Cell::from(self.fit(p.session_count.to_string(), granted)),
            Self::Sources => Cell::from(self.fit(sources_label(p), granted))
                .style(Style::default().fg(app.theme.muted)),
            Self::Models => build_models_cell(&p.models, self.granted_width(granted), app),
            Self::Input => Cell::from(self.fit(format_tokens(p.tokens.input), granted))
                .style(app.theme.metric_input_style()),
            Self::Output => Cell::from(self.fit(format_tokens(p.tokens.output), granted))
                .style(app.theme.metric_output_style()),
            Self::CacheRead => Cell::from(self.fit(format_tokens(p.tokens.cache_read), granted))
                .style(app.theme.metric_cache_read_style()),
            Self::CacheWrite => Cell::from(self.fit(format_tokens(p.tokens.cache_write), granted))
                .style(app.theme.metric_cache_write_style()),
            Self::Total => Cell::from(self.fit(format_tokens(p.tokens.total()), granted))
                .style(app.theme.metric_total_style()),
            Self::Cost => Cell::from(self.fit(format_cost(p.cost), granted))
                .style(Style::default().fg(Color::Green)),
            Self::LastActive => Cell::from(
                self.fit(
                    ms_to_local_naive(p.last_active_ms)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "\u{2014}".to_string()),
                    granted,
                ),
            )
            .style(Style::default().fg(app.theme.muted)),
        }
    }

    /// Position in the display order, for indexing into the solved widths.
    fn index(self) -> usize {
        WIDE_ORDER.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// Cells the solver granted this column out of the already-solved `widths`.
    /// A `Length` is a request, not a guarantee, and `Min` floats, so text is
    /// cut to what the column actually got rather than to what it asked for.
    fn granted_width(self, widths: &[u16]) -> usize {
        widths.get(self.index()).copied().unwrap_or(0) as usize
    }

    /// Clamp a fixed-width column's text to its granted width. A no-op at
    /// widths where the request was satisfied; past that a clipped number would
    /// read as a plausible wrong value, so the cut keeps an ellipsis.
    fn fit(self, text: String, granted: &[u16]) -> String {
        truncate_to_width(&text, self.granted_width(granted))
    }
}

/// The wide layout's column constraints.
fn wide_constraints() -> Vec<Constraint> {
    WIDE_ORDER.iter().map(|c| c.constraint()).collect()
}

fn wide_min_width() -> u16 {
    WIDE_ORDER.iter().map(|c| c.min_width()).sum::<u16>()
        + COLUMN_SPACING * WIDE_ORDER.len().saturating_sub(1) as u16
}

/// Widths the solver grants each wide column at `total` cells. Ratatui's table
/// solves the same constraint set with one cell of `column_spacing` between
/// columns, so this mirrors that layout exactly.
fn wide_table_widths(total: u16) -> Vec<u16> {
    Layout::horizontal(wide_constraints())
        .spacing(COLUMN_SPACING)
        .split(Rect::new(0, 0, total, 1))
        .iter()
        .map(|area| area.width)
        .collect()
}

/// Distinct clients in first-seen order, compact-named and comma-joined.
fn sources_label(p: &ProjectUsage) -> String {
    p.clients
        .iter()
        .map(|c| get_compact_client_display_name(c))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Projects ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let projects = app.get_sorted_projects();
    if projects.is_empty() {
        let empty_msg = Paragraph::new("No project usage data found. Press 'r' to refresh.")
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_very_narrow = area.width < 60;
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_selection = app.theme.selection;
    let striped_row_style = app.theme.striped_row_style();

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

    let wide = inner.width >= wide_min_width();

    let header_cells: Vec<String> = if wide {
        WIDE_ORDER
            .iter()
            .map(|c| {
                let indicator = c.sort_field().map(sort_indicator).unwrap_or("");
                format!("{}{}", c.header(), indicator)
            })
            .collect()
    } else {
        let labels: &[&str] = if is_very_narrow {
            &["Project", "Cost"]
        } else {
            &["Project", "Sessions", "Total", "Cost"]
        };
        // The narrow layouts keep hand-picked indices, and `usize::MAX` stands
        // for "this sort has no column here".
        let (total_idx, cost_idx) = if is_very_narrow {
            (usize::MAX, 1)
        } else {
            (2, 3)
        };
        labels
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let indicator = if i == total_idx {
                    sort_indicator(SortField::Tokens)
                } else if i == cost_idx {
                    sort_indicator(SortField::Cost)
                } else {
                    ""
                };
                format!("{}{}", h, indicator)
            })
            .collect()
    };

    let header = Row::new(header_cells.into_iter().map(Cell::from).collect::<Vec<_>>())
        .style(
            Style::default()
                .fg(theme_accent)
                .add_modifier(Modifier::BOLD),
        )
        .height(1);

    let projects_len = projects.len();
    let start = scroll_offset.min(projects_len);
    let end = (start + visible_height).min(projects_len);

    if start >= projects_len {
        return;
    }

    // Solve the wide layout once per render, not once per cell: the widths
    // depend only on `inner.width`.
    let wide_widths = if wide {
        wide_table_widths(inner.width)
    } else {
        Vec::new()
    };

    let rows: Vec<Row> = projects[start..end]
        .iter()
        .enumerate()
        .map(|(i, usage)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if wide {
                WIDE_ORDER
                    .iter()
                    .map(|c| c.cell(idx + 1, usage, app, &wide_widths))
                    .collect()
            } else if is_very_narrow {
                vec![
                    Cell::from(truncate_text(&usage.label, 20)).style(
                        Style::default()
                            .fg(theme_accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(format_cost(usage.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                vec![
                    Cell::from(truncate_text(&usage.label, 24)).style(
                        Style::default()
                            .fg(theme_accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(usage.session_count.to_string()),
                    total_tokens_cell(usage.tokens.total(), &app.theme),
                    Cell::from(format_cost(usage.cost)).style(Style::default().fg(Color::Green)),
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

    let widths: Vec<Constraint> = if wide {
        wide_constraints()
    } else if is_very_narrow {
        vec![Constraint::Percentage(60), Constraint::Percentage(40)]
    } else {
        vec![
            Constraint::Percentage(40),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ]
    };

    let table = Table::new(rows, widths)
        .column_spacing(COLUMN_SPACING)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if projects_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state =
            viewport_scrollbar_state(projects_len, scroll_offset, visible_height);

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

/// Build a table cell for the Models column with each model name colored by the
/// same family-shade system used in the Overview and Models tabs, joined with
/// `", "` and truncated to `max_cells` in terminal cells. Mirrors the Sessions
/// tab's model cell: a multi-model project always ends in an ellipsis when some
/// of its models did not fit, so it never renders as a single-model one.
fn build_models_cell(models: &[SessionModel], max_cells: usize, app: &App) -> Cell<'static> {
    if max_cells == 0 {
        return Cell::from("");
    }
    if models.is_empty() {
        return Cell::from("\u{2014}".to_string()).style(Style::default().fg(app.theme.muted));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut budget = max_cells;

    for (i, model) in models.iter().enumerate() {
        if i > 0 {
            if budget < 3 {
                spans.push(Span::styled("…", Style::default().fg(app.theme.muted)));
                break;
            }
            spans.push(Span::styled(", ", Style::default().fg(app.theme.muted)));
            budget -= 2;
        }

        let color = app.model_color_for(&model.provider, &model.color_key);
        let name = &model.display_name;
        let model_len = display_width(name);

        // Leave room to mark omitted models only while more names remain.
        // The last name can use the full remaining width.
        let has_more = i + 1 < models.len();
        if model_len <= budget.saturating_sub(usize::from(has_more)) {
            spans.push(Span::styled(name.clone(), Style::default().fg(color)));
            budget -= model_len;
        } else {
            let head = prefix_to_width(name, budget - 1);
            spans.push(Span::styled(
                format!("{}…", head),
                Style::default().fg(color),
            ));
            break;
        }
    }

    Cell::from(Line::from(spans))
}

/// Convert Unix-ms to a local NaiveDateTime for display.
fn ms_to_local_naive(ms: i64) -> Option<NaiveDateTime> {
    if ms <= 0 {
        return None;
    }
    let secs = ms / 1000;
    match Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => Some(dt.naive_local()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{Tab, TuiConfig};
    use crate::tui::data::TokenBreakdown;
    use ratatui::{backend::TestBackend, Terminal};

    fn project(label: &str, cost: f64, last_ms: i64) -> ProjectUsage {
        ProjectUsage {
            group_key: label.to_string(),
            workspace_key: Some(label.to_string()),
            label: label.to_string(),
            path: None,
            clients: vec!["opencode".to_string(), "claude".to_string()],
            models: vec![SessionModel {
                display_name: "claude-sonnet-4".to_string(),
                provider: "anthropic".to_string(),
                color_key: "claude-sonnet-4".to_string(),
            }],
            tokens: TokenBreakdown {
                input: 1_234_567,
                output: 234_567,
                cache_read: 45_678_901,
                cache_write: 2_345_678,
                reasoning: 0,
            },
            cost,
            message_count: 428,
            session_count: 3,
            first_active_ms: last_ms.saturating_sub(3_600_000),
            last_active_ms: last_ms,
        }
    }

    fn make_app(width: u16) -> App {
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
        app.terminal_width = width;
        app.current_tab = Tab::Projects;
        app.sort_field = SortField::Cost;
        app.sort_direction = SortDirection::Descending;
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

    #[test]
    fn wide_header_lists_every_column() {
        let mut app = make_app(200);
        app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
        let body = render_body(&mut app, 200, 6);
        let header = body.lines().nth(1).unwrap_or_default();
        for c in WIDE_ORDER {
            assert!(
                header.contains(c.header()),
                "wide header is missing {:?} ({:?})",
                c,
                c.header()
            );
        }
    }

    #[test]
    fn wide_row_renders_full_values_when_the_row_fits() {
        let mut app = make_app(200);
        app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
        let body = render_body(&mut app, 200, 6);
        let row = body.lines().nth(2).unwrap_or_default();
        for expected in [
            "tokscale", "3", "1.2M", "234K", "45.7M", "2.3M", "49.5M", "$12.35",
        ] {
            assert!(row.contains(expected), "row is missing {expected:?}: {row}");
        }
    }

    #[test]
    fn project_column_truncates_to_granted_width_not_requested() {
        // At any width, the label cell must be cut to what the solver actually
        // granted the Project column — cutting to the request clips with no
        // ellipsis when the solver shrinks the column.
        for total in 60u16..=240 {
            let granted = wide_table_widths(total);
            let project_width = ProjectColumn::Project.granted_width(&granted);
            let label = "a/very/long/workspace/label/that/keeps/going";
            let fitted = fit_workspace_label_to_width(label, project_width);
            assert!(
                display_width(&fitted) <= project_width,
                "at {total} cols the label exceeds its granted {project_width} cells"
            );
        }
    }

    #[test]
    fn narrow_and_very_narrow_layouts_render() {
        let mut app = make_app(70);
        app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
        let body = render_body(&mut app, 70, 6);
        assert!(body.contains("Project"));
        assert!(body.contains("Sessions"));

        let mut app = make_app(30);
        app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
        let body = render_body(&mut app, 30, 6);
        assert!(body.contains("Project"));
        assert!(body.contains("Cost"));
    }

    #[test]
    fn compact_layout_preserves_totals_until_all_wide_columns_fit() {
        let mut app = make_app(200);
        app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
        for width in [80, 100, 120, 140, 151] {
            let body = render_body(&mut app, width, 6);
            let header = body.lines().nth(1).unwrap();
            let row = body.lines().nth(2).unwrap();
            for label in ["Project", "Sessions", "Total", "Cost ▼"] {
                assert!(header.contains(label), "at {width} columns: {header}");
            }
            assert!(!header.contains("Models"), "at {width} columns: {header}");
            for value in ["tokscale", "49.5M", "$12.35"] {
                assert!(row.contains(value), "at {width} columns: {row}");
            }
        }
    }

    #[test]
    fn wide_layout_fits_at_its_minimum_width() {
        let mut app = make_app(152);
        let last_ms = 1_736_000_000_000;
        app.data.projects = vec![project("tokscale", 12.3456, last_ms)];
        let body = render_body(&mut app, 152, 6);
        let header = body.lines().nth(1).unwrap();
        let row = body.lines().nth(2).unwrap();
        for column in WIDE_ORDER {
            assert!(header.contains(column.header()), "{header}");
        }
        let last_active = ms_to_local_naive(last_ms)
            .unwrap()
            .format("%Y-%m-%d %H:%M")
            .to_string();
        for value in ["234K", "45.7M", "2.3M", "49.5M", "$12.35", &last_active] {
            assert!(row.contains(value), "{row}");
        }
    }

    fn render_models(names: &[&str], width: u16) -> String {
        let app = make_app(200);
        let models = names
            .iter()
            .map(|name| SessionModel {
                display_name: name.to_string(),
                provider: "openai".to_string(),
                color_key: name.to_string(),
            })
            .collect::<Vec<_>>();
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        let table = Table::new(
            [Row::new([build_models_cell(&models, width as usize, &app)])],
            [Constraint::Length(width)],
        );
        Widget::render(table, area, &mut buffer);
        let mut text = String::new();
        let mut x = 0;
        while x < width {
            let symbol = buffer[(x, 0)].symbol();
            text.push_str(symbol);
            x += display_width(symbol).max(1) as u16;
        }
        text.trim_end().to_string()
    }

    #[test]
    fn models_cell_shows_all_names_when_they_exactly_fit() {
        assert_eq!(render_models(&["gpt-5", "k3"], 9), "gpt-5, k3");
        assert_eq!(render_models(&["gpt-5", "k3", "o3"], 13), "gpt-5, k3, o3");
        assert_eq!(render_models(&["gpt-5", "k3"], 18), "gpt-5, k3");
    }

    #[test]
    fn models_cell_marks_truncation_within_the_available_width() {
        for (width, expected) in [
            (0, ""),
            (1, "…"),
            (5, "gpt-…"),
            (6, "gpt-5…"),
            (7, "gpt-5…"),
            (8, "gpt-5, …"),
        ] {
            let rendered = render_models(&["gpt-5", "k3"], width);
            assert_eq!(rendered, expected, "at {width} columns");
            assert!(display_width(&rendered) <= width as usize);
        }
        assert_eq!(render_models(&["a", "🇺🇸x"], 5), "a, …");
        assert_eq!(render_models(&["a", "🇺🇸x"], 6), "a, 🇺🇸x");
    }

    #[test]
    fn empty_state_renders_hint() {
        let mut app = make_app(120);
        let body = render_body(&mut app, 120, 6);
        assert!(body.contains("No project usage data found"));
    }
}
