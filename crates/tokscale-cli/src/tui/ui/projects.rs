use chrono::{Local, NaiveDateTime, TimeZone};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::widgets::{
    ambient_stable_scrollbar, display_width, fit_workspace_label_to_width, format_cache_hit_rate,
    format_cost, format_tokens, get_compact_client_display_name, prefix_to_width, truncate_text,
    truncate_to_width, viewport_scrollbar_state, AMBIENT_STABLE_BORDER_SET, MIDDLE_ELLIPSIS,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::data::{ProjectUsage, SessionModel};
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};

/// One column of the wide Projects layout, in left-to-right display order.
///
/// Every per-column fact hangs off this enum so the header, cells, constraints
/// and truncation budgets cannot drift apart: each method is an exhaustive
/// `match` with no `_` arm, so adding a variant fails to compile until every
/// one of them has an answer for it.
///
/// The layout is budget-driven like the Sessions tab: a column is admitted at
/// its natural width or not shown at all, in [`WIDE_PRIORITY`] group order.
/// The previous all-or-nothing design asked for 150 columns and otherwise fell
/// back to a four-column compact table, so a terminal anywhere between 60 and
/// 151 columns showed Project/Sessions/Total/Cost and nothing else.
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
    CacheHit,
    Total,
    Cost,
    LastActive,
}

/// Every variant, exactly once. Checked against `WIDE_ORDER` and
/// `WIDE_PRIORITY` by `every_column_is_ordered_and_prioritized`, because the
/// compiler never looks at those two arrays.
#[cfg(test)]
const ALL: [ProjectColumn; 13] = [
    ProjectColumn::Rank,
    ProjectColumn::Project,
    ProjectColumn::Sessions,
    ProjectColumn::Sources,
    ProjectColumn::Models,
    ProjectColumn::Input,
    ProjectColumn::Output,
    ProjectColumn::CacheRead,
    ProjectColumn::CacheWrite,
    ProjectColumn::CacheHit,
    ProjectColumn::Total,
    ProjectColumn::Cost,
    ProjectColumn::LastActive,
];

/// Left-to-right display order. Cosmetic: reshuffling it never changes what
/// fits, only where it sits.
const WIDE_ORDER: [ProjectColumn; 13] = [
    ProjectColumn::Rank,
    ProjectColumn::Project,
    ProjectColumn::Sessions,
    ProjectColumn::Sources,
    ProjectColumn::Models,
    ProjectColumn::Input,
    ProjectColumn::Output,
    ProjectColumn::CacheRead,
    ProjectColumn::CacheWrite,
    ProjectColumn::CacheHit,
    ProjectColumn::Total,
    ProjectColumn::Cost,
    ProjectColumn::LastActive,
];

/// Admission order: earlier groups are admitted first and dropped last. Each
/// group is all-or-nothing — `Input` without `Output` invites a reader to take
/// Input for a total, and `Cache R` without `Cache W` is the same trap.
///
/// The core is the column set the old compact layout showed, kept atomic so
/// admission can never stop *inside* it: the wide layout always shows at least
/// those four. After that, groups are ranked by how directly they line the tab
/// up with the other token tables — the Input/Output/Cache breakdown first,
/// then the columns that make a project findable and comparable.
const WIDE_PRIORITY: [&[ProjectColumn]; 8] = [
    &[
        ProjectColumn::Project,
        ProjectColumn::Sessions,
        ProjectColumn::Total,
        ProjectColumn::Cost,
    ],
    &[ProjectColumn::Rank],
    &[ProjectColumn::Input, ProjectColumn::Output],
    &[ProjectColumn::CacheRead, ProjectColumn::CacheWrite],
    &[ProjectColumn::CacheHit],
    &[ProjectColumn::LastActive],
    &[ProjectColumn::Models],
    &[ProjectColumn::Sources],
];

const COLUMN_SPACING: u16 = 1;

/// Produced by admission, consumed by rendering. Every width that depends on
/// the chosen set lives here and nowhere else.
struct WideLayout {
    chosen: Vec<ProjectColumn>,
    /// What `Project`'s cell fits its label to. Project is the sole `Min`
    /// column, so ratatui hands it every cell of slack; this has to match that
    /// or the label is clipped without an ellipsis.
    project_width: u16,
}

impl ProjectColumn {
    fn header(self, lang: TuiLanguage) -> &'static str {
        match self {
            Self::Rank => tr(lang, MessageKey::ColRank),
            Self::Project => tr(lang, MessageKey::ColProject),
            Self::Sessions => tr(lang, MessageKey::ColSessions),
            Self::Sources => tr(lang, MessageKey::ColSources),
            Self::Models => tr(lang, MessageKey::ColModels),
            Self::Input => tr(lang, MessageKey::ColInput),
            Self::Output => tr(lang, MessageKey::ColOutput),
            Self::CacheRead => tr(lang, MessageKey::ColCacheRead),
            Self::CacheWrite => tr(lang, MessageKey::ColCacheWrite),
            Self::CacheHit => tr(lang, MessageKey::ColCacheHit),
            Self::Total => tr(lang, MessageKey::ColTotal),
            Self::Cost => tr(lang, MessageKey::ColCost),
            Self::LastActive => tr(lang, MessageKey::ColLastActive),
        }
    }

    /// Cells this column needs to render its widest realistic value, and the
    /// only width input to admission. `Constraint`s are *derived* from this
    /// rather than written beside it: a second literal is exactly the
    /// budget-versus-layout divergence this design exists to kill.
    fn natural(self) -> u16 {
        match self {
            Self::Project | Self::Models => 18,
            Self::Rank => 4,
            Self::Sessions => 8,
            Self::Sources => 14,
            Self::Input | Self::Output | Self::CacheRead | Self::CacheWrite | Self::Total => 10,
            Self::CacheHit => 8,
            Self::Cost => 10,
            Self::LastActive => 16,
        }
    }

    /// The constraint a column is laid out with, derived from `natural()`.
    /// Project is the sole flexible (`Min`) column and absorbs whatever slack
    /// admission leaves behind.
    fn constraint(self) -> Constraint {
        if self == Self::Project {
            Constraint::Min(self.natural())
        } else {
            Constraint::Length(self.natural())
        }
    }

    /// The sort this column is the target of, so the indicator is placed by
    /// identity rather than by a computed index. When the sorted column is not
    /// admitted nothing matches and no arrow is drawn.
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
            | Self::CacheWrite
            | Self::CacheHit => None,
        }
    }

    fn cell(self, rank: usize, p: &ProjectUsage, app: &App, layout: &WideLayout) -> Cell<'static> {
        match self {
            Self::Rank => {
                Cell::from(self.fit(rank.to_string())).style(Style::default().fg(app.theme.muted))
            }
            // Not a plain head cut: a workspace label is identified by the ends
            // of each of its segments, and cutting the tail leaves the prefix
            // every row shares. Fitted to the width admission resolved for this
            // column, never to the requested width.
            Self::Project => Cell::from(fit_workspace_label_to_width(
                &p.label,
                layout.project_width as usize,
            ))
            .style(
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Self::Sessions => Cell::from(self.fit(p.session_count.to_string())),
            Self::Sources => {
                Cell::from(self.fit(sources_label(p))).style(Style::default().fg(app.theme.muted))
            }
            Self::Models => build_models_cell(&p.models, self.natural() as usize, app),
            Self::Input => Cell::from(self.fit(format_tokens(p.tokens.input)))
                .style(app.theme.metric_input_style()),
            Self::Output => Cell::from(self.fit(format_tokens(p.tokens.output)))
                .style(app.theme.metric_output_style()),
            Self::CacheRead => Cell::from(self.fit(format_tokens(p.tokens.cache_read)))
                .style(app.theme.metric_cache_read_style()),
            Self::CacheWrite => Cell::from(self.fit(format_tokens(p.tokens.cache_write)))
                .style(app.theme.metric_cache_write_style()),
            Self::CacheHit => Cell::from(self.fit(format_cache_hit_rate(
                p.tokens.cache_read,
                p.tokens.input,
                p.tokens.cache_write,
            )))
            .style(app.theme.count_style()),
            Self::Total => Cell::from(self.fit(format_tokens(p.tokens.total())))
                .style(app.theme.metric_total_style()),
            Self::Cost => {
                Cell::from(self.fit(format_cost(p.cost))).style(Style::default().fg(Color::Green))
            }
            Self::LastActive => Cell::from(
                self.fit(
                    ms_to_local_naive(p.last_active_ms)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "\u{2014}".to_string()),
                ),
            )
            .style(Style::default().fg(app.theme.muted)),
        }
    }

    /// Clamp a fixed-width column's text to its natural width. A no-op on
    /// every value the column was sized for — `natural()` is the widest
    /// *realistic* one — but past that a clipped number would read as a
    /// plausible wrong value, so the cut keeps an ellipsis. Project and Models
    /// clamp against their own resolved widths at their call sites instead.
    fn fit(self, text: String) -> String {
        truncate_to_width(&text, self.natural() as usize)
    }
}

/// Position in the display order. Total rather than `unwrap()`: a column
/// missing from `WIDE_ORDER` sorts last instead of panicking inside the draw
/// loop, which would leave the terminal in raw mode. The permutation test is
/// what actually catches it.
fn order_index(c: ProjectColumn) -> usize {
    WIDE_ORDER
        .iter()
        .position(|o| *o == c)
        .unwrap_or(usize::MAX)
}

/// Cells a column set occupies: the widths themselves plus one separator
/// between every pair. Measured against `inner`, which already has the block
/// borders removed.
fn required(set: &[ProjectColumn]) -> u16 {
    let widths: u16 = set.iter().map(|c| c.natural()).sum();
    widths + COLUMN_SPACING * set.len().saturating_sub(1) as u16
}

/// Admit whole priority groups while they fit, then hand the slack to Project.
/// Stop at the first group that does not fit rather than skipping to a
/// narrower one, so the admitted set stays monotonic in width.
fn admit_and_distribute(available: u16) -> WideLayout {
    let mut chosen: Vec<ProjectColumn> = Vec::new();
    for group in WIDE_PRIORITY {
        let mut next = chosen.clone();
        next.extend(group);
        if required(&next) <= available {
            chosen = next;
        } else {
            break;
        }
    }
    chosen.sort_by_key(|c| order_index(*c));

    let project_width = if chosen.contains(&ProjectColumn::Project) {
        ProjectColumn::Project.natural() + available.saturating_sub(required(&chosen))
    } else {
        ProjectColumn::Project.natural()
    };

    WideLayout {
        chosen,
        project_width,
    }
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
    let lang = app.settings.tui_language;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(AMBIENT_STABLE_BORDER_SET)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            format!(" {} ", tr(lang, MessageKey::TabProjects)),
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
        let empty_msg = Paragraph::new(tr(lang, MessageKey::EmptyNoProjectData))
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
                SortDirection::Ascending => " ▴",
                SortDirection::Descending => " ▾",
            }
        } else {
            ""
        }
    };

    // The wide layout is budget-driven: a column is admitted at its natural
    // width or it is not shown. The core group is atomic, so an `inner`
    // narrower than its budget admits nothing at all; the very-narrow fallback
    // below is percentage-based and renders something at any width.
    let wide = (!is_very_narrow)
        .then(|| admit_and_distribute(inner.width))
        .filter(|layout| !layout.chosen.is_empty());

    let header_cells: Vec<String> = if let Some(layout) = &wide {
        // Headers carry the sort arrow, so this is one derivation and not two.
        layout
            .chosen
            .iter()
            .map(|c| {
                let indicator = c.sort_field().map(sort_indicator).unwrap_or("");
                format!("{}{}", c.header(lang), indicator)
            })
            .collect()
    } else {
        [
            tr(lang, MessageKey::ColProject),
            tr(lang, MessageKey::ColCost),
        ]
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let indicator = if i == 1 {
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

    let rows: Vec<Row> = projects[start..end]
        .iter()
        .enumerate()
        .map(|(i, usage)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if let Some(layout) = &wide {
                layout
                    .chosen
                    .iter()
                    .map(|c| c.cell(idx + 1, usage, app, layout))
                    .collect()
            } else {
                vec![
                    Cell::from(truncate_text(&usage.label, 20)).style(
                        Style::default()
                            .fg(theme_accent)
                            .add_modifier(Modifier::BOLD),
                    ),
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

    let widths: Vec<Constraint> = if let Some(layout) = &wide {
        layout.chosen.iter().map(|c| c.constraint()).collect()
    } else {
        vec![Constraint::Percentage(60), Constraint::Percentage(40)]
    };

    let table = Table::new(rows, widths)
        .column_spacing(COLUMN_SPACING)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if projects_len > visible_height {
        let scrollbar = ambient_stable_scrollbar();

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
/// of its models did not fit, so it never renders as a single-model one. The
/// marker is [`MIDDLE_ELLIPSIS`], which is one cell in every terminal locale;
/// U+2026 is East-Asian-Ambiguous and would be two cells under a CJK locale,
/// overflowing the budget this cell promises to keep.
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
                spans.push(Span::styled(
                    MIDDLE_ELLIPSIS,
                    Style::default().fg(app.theme.muted),
                ));
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
                format!("{head}{MIDDLE_ELLIPSIS}"),
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
    use unicode_width::UnicodeWidthStr;

    /// Inner width (`area` minus the two border cells) at which each priority
    /// group first fits. **Literals, never derived from `admit_and_distribute`**:
    /// a test that recomputes the formula it is testing cannot fail. Amending
    /// `WIDE_PRIORITY` or `natural()` moves these numbers, and the diff then
    /// names which threshold moved and by how much.
    const THRESHOLDS: [(u16, &[ProjectColumn]); 8] = [
        (
            49,
            &[
                ProjectColumn::Project,
                ProjectColumn::Sessions,
                ProjectColumn::Total,
                ProjectColumn::Cost,
            ],
        ),
        (54, &[ProjectColumn::Rank]),
        (76, &[ProjectColumn::Input, ProjectColumn::Output]),
        (98, &[ProjectColumn::CacheRead, ProjectColumn::CacheWrite]),
        (107, &[ProjectColumn::CacheHit]),
        (124, &[ProjectColumn::LastActive]),
        (143, &[ProjectColumn::Models]),
        (158, &[ProjectColumn::Sources]),
    ];

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
        app.settings.tui_language = TuiLanguage::En;
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

    fn header_line(app: &mut App, width: u16) -> String {
        render_body(app, width, 6)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    // ---- the descriptor arrays -------------------------------------------

    /// The exhaustive `match`es cover the descriptor methods and stop there:
    /// `WIDE_ORDER` and `WIDE_PRIORITY` are hand-maintained arrays that compile
    /// fine while missing a variant. Absent from `WIDE_PRIORITY` a column is
    /// never admitted at any width and silently never renders; absent from
    /// `WIDE_ORDER` its cells sort behind every admitted column.
    #[test]
    fn every_column_is_ordered_and_prioritized() {
        let mut order = WIDE_ORDER.to_vec();
        let mut priority: Vec<ProjectColumn> = WIDE_PRIORITY.concat();
        let mut all = ALL.to_vec();
        for v in [&mut order, &mut priority, &mut all] {
            v.sort_by_key(|c| *c as usize);
        }
        assert_eq!(order, all, "WIDE_ORDER is not a permutation of ALL");
        assert_eq!(priority, all, "WIDE_PRIORITY is not a permutation of ALL");
    }

    // ---- the budget -------------------------------------------------------

    #[test]
    fn group_thresholds_are_where_the_review_put_them() {
        for (threshold, group) in THRESHOLDS {
            let at = admit_and_distribute(threshold).chosen;
            for column in group {
                assert!(
                    at.contains(column),
                    "{column:?} should be admitted at an inner width of {threshold}, got {at:?}"
                );
            }
            let below = admit_and_distribute(threshold - 1).chosen;
            for column in group {
                assert!(
                    !below.contains(column),
                    "{column:?} should not be admitted at an inner width of {}, got {below:?}",
                    threshold - 1
                );
            }
        }
    }

    /// The admitted set never costs more cells than the terminal has.
    #[test]
    fn the_admitted_set_always_fits_the_terminal() {
        for available in 0u16..=240 {
            let chosen = admit_and_distribute(available).chosen;
            assert!(
                required(&chosen) <= available,
                "admitted set overflows at inner width {available}: {chosen:?}"
            );
        }
    }

    /// Widening the terminal never removes a column.
    #[test]
    fn admitted_columns_are_monotonic_in_width() {
        let mut previous: Vec<ProjectColumn> = Vec::new();
        for available in 0u16..=240 {
            let current = admit_and_distribute(available).chosen;
            for column in &previous {
                assert!(
                    current.contains(column),
                    "{column:?} disappeared going from {} to {available} inner columns",
                    available - 1
                );
            }
            previous = current;
        }
    }

    /// The point of the change: the Input/Output/Cache breakdown shows on
    /// ordinary terminals instead of waiting for 152 columns. Thresholds are
    /// terminal widths, so the assertions run through an actual render.
    #[test]
    fn token_breakdown_columns_render_once_they_fit() {
        let cases: [(u16, &[&str], &[&str]); 6] = [
            (60, &["Sessions", "Total", "Cost ▾"], &["Input", "Cache R"]),
            // Inner 75: one cell short of the Input/Output group.
            (77, &["Sessions"], &["Input", "Output"]),
            (78, &["Input", "Output"], &["Cache R", "Cache W"]),
            (100, &["Cache R", "Cache W"], &["Cache✕", "Last Active"]),
            (109, &["Cache✕"], &["Last Active", "Models"]),
            (126, &["Last Active"], &["Models", "Sources"]),
        ];
        for (width, present, absent) in cases {
            let mut app = make_app(width);
            app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
            let header = header_line(&mut app, width);
            for label in present {
                assert!(
                    header.contains(label),
                    "{label:?} missing at width {width}\n{header}"
                );
            }
            for label in absent {
                assert!(
                    !header.contains(label),
                    "{label:?} showed at width {width}\n{header}"
                );
            }
        }
    }

    /// A column that is not admitted leaves nothing behind — no stub header
    /// and, more importantly, no half of a number.
    #[test]
    fn unadmitted_columns_leave_no_trace() {
        for width in 60u16..160 {
            let mut app = make_app(width);
            app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
            let header = header_line(&mut app, width);
            let admitted = admit_and_distribute(width - 2).chosen;
            for column in ALL {
                if admitted.contains(&column) {
                    continue;
                }
                assert!(
                    !header.contains(column.header(TuiLanguage::En)),
                    "{column:?} header showed at width {width}\n{header}"
                );
            }
        }
    }

    // ---- full layout ------------------------------------------------------

    #[test]
    fn wide_header_lists_every_column() {
        let mut app = make_app(200);
        app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
        let header = header_line(&mut app, 200);
        for c in WIDE_ORDER {
            assert!(
                header.contains(c.header(TuiLanguage::En)),
                "wide header is missing {:?} ({:?})",
                c,
                c.header(TuiLanguage::En)
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
            "tokscale", "3", "1.2M", "234K", "45.7M", "2.3M", "12.8x", "49.5M", "$12.35",
        ] {
            assert!(row.contains(expected), "row is missing {expected:?}: {row}");
        }
    }

    #[test]
    fn full_layout_fits_at_its_minimum_width() {
        // Inner 158 is exactly the sum of every natural width plus spacing.
        let mut app = make_app(160);
        let last_ms = 1_736_000_000_000;
        app.data.projects = vec![project("tokscale", 12.3456, last_ms)];
        let body = render_body(&mut app, 160, 6);
        let header = body.lines().nth(1).unwrap();
        let row = body.lines().nth(2).unwrap();
        for column in WIDE_ORDER {
            assert!(header.contains(column.header(TuiLanguage::En)), "{header}");
        }
        let last_active = ms_to_local_naive(last_ms)
            .unwrap()
            .format("%Y-%m-%d %H:%M")
            .to_string();
        for value in [
            "234K",
            "45.7M",
            "2.3M",
            "12.8x",
            "49.5M",
            "$12.35",
            &last_active,
        ] {
            assert!(row.contains(value), "{row}");
        }
    }

    #[test]
    fn project_column_truncates_to_granted_width_not_requested() {
        // At any width, the label cell must be cut to what admission actually
        // granted the Project column — cutting to the request clips with no
        // ellipsis when the column shrinks.
        for total in 60u16..=240 {
            let layout = admit_and_distribute(total - 2);
            let fitted = fit_workspace_label_to_width(
                "a/very/long/workspace/label/that/keeps/going",
                layout.project_width as usize,
            );
            assert!(
                display_width(&fitted) <= layout.project_width as usize,
                "at {total} cols the label exceeds its granted {} cells",
                layout.project_width
            );
        }
    }

    #[test]
    fn sort_indicator_is_legible_exactly_when_its_column_is_admitted() {
        for (field, column) in [
            (SortField::Cost, ProjectColumn::Cost),
            (SortField::Tokens, ProjectColumn::Total),
            (SortField::Date, ProjectColumn::LastActive),
        ] {
            for width in 60u16..=200 {
                let mut app = make_app(width);
                app.data.projects = vec![project("tokscale", 12.3456, 1_736_000_000_000)];
                app.sort_field = field;
                let header = header_line(&mut app, width);
                if admit_and_distribute(width - 2).chosen.contains(&column) {
                    assert!(
                        header.contains(&format!("{} ▾", column.header(TuiLanguage::En))),
                        "{field:?} ▾ clipped at width {width}\n{header}"
                    );
                } else {
                    assert!(
                        !header.contains('▾'),
                        "{field:?} drew ▾ with its column unadmitted at width {width}\n{header}"
                    );
                }
            }
        }
    }

    // ---- narrow fallback ---------------------------------------------------

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
    fn empty_state_renders_hint() {
        let mut app = make_app(120);
        let body = render_body(&mut app, 120, 6);
        assert!(body.contains("No project usage data found"));
    }

    // ---- the Models cell ---------------------------------------------------

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

    /// Checked in both ambients: `unicode-width` resolves East-Asian-Ambiguous
    /// characters to one cell by default and to two under `width_cjk`, which is
    /// what a terminal in a CJK locale does, so a marker that is ambiguous keeps
    /// the budget on one machine and blows it on another.
    #[test]
    fn models_cell_marks_truncation_within_the_available_width() {
        for (width, expected) in [
            (0, ""),
            (1, "⋯"),
            (5, "gpt-⋯"),
            (6, "gpt-5⋯"),
            (7, "gpt-5⋯"),
            (8, "gpt-5, ⋯"),
        ] {
            let rendered = render_models(&["gpt-5", "k3"], width);
            assert_eq!(rendered, expected, "at {width} columns");
            assert!(display_width(&rendered) <= width as usize);
            assert!(
                UnicodeWidthStr::width_cjk(rendered.as_str()) <= width as usize,
                "at {width} columns in a CJK locale: {rendered:?}"
            );
        }
        assert_eq!(render_models(&["a", "🇺🇸x"], 5), "a, ⋯");
        assert_eq!(render_models(&["a", "🇺🇸x"], 6), "a, 🇺🇸x");
    }
}
