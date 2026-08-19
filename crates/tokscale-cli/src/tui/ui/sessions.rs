use chrono::{Local, NaiveDateTime, TimeZone};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table,
};

use super::widgets::{
    display_width, format_cache_hit_rate, format_cost, format_cost_per_million, format_tokens,
    get_compact_client_display_name, prefix_to_width, total_tokens_cell, truncate_text,
    truncate_to_width, viewport_scrollbar_state,
};
use crate::tui::app::{App, SortDirection, SortField};
use crate::tui::data::{SessionModel, SessionUsage};

/// Widest the Model column ever grows; surplus past this goes to Session.
const MODEL_COLUMN_MAX_CHARS: u16 = 36;

/// Session title width that slack distribution buys before Model gets any.
/// Model's 18-cell minimum already renders most model names in full, while
/// session titles routinely run 40–80 characters, so the marginal cell is worth
/// more to the title. (Decision B in `docs/sessions-column-budget.md`, which is
/// #965 and lands with it — the path does not resolve in this tree until it
/// does, which is why the PR number is here too.)
const SESSION_COLUMN_COMFORTABLE_CHARS: u16 = 40;

/// One column of the wide Sessions layout.
///
/// The wide layout used to be six hand-synced vectors — headers, cells,
/// constraints, sort indices, a duplicated width sum, and a truncation limit —
/// none of which the compiler checked against each other. It also asked for 161
/// columns while turning on at 80, so across the whole 80–160 band ratatui
/// shrank every column and clipped cell *content* with no ellipsis: 234K tokens
/// rendered as `234`, $12.35 as `$12.`. Plausible numbers, wrong by 1000x.
///
/// Now every per-column fact hangs off this enum and a column is either
/// admitted at its natural width or not shown at all. Each method below is an
/// exhaustive `match` with no `_` arm, so adding a variant fails to compile
/// until every one of them has an answer for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionColumn {
    Session,
    Client,
    Model,
    Turn,
    Msgs,
    Input,
    Output,
    CacheRead,
    CacheWrite,
    CacheHit,
    Total,
    Cost,
    CostPerMillion,
    Duration,
    LastActive,
}

/// Every variant, exactly once. `WIDE_ORDER` and `WIDE_PRIORITY` are checked
/// against this by `every_column_is_ordered_and_prioritized`, because the
/// compiler never looks at those two arrays. It lives here rather than in the
/// test module because it belongs beside the arrays it audits.
///
/// Note what that test cannot see: it compares the three arrays *to each other*,
/// and `ALL` is hand-maintained too, so a variant missing from all three passes
/// it — verified, with the whole suite green. That shape is caught by the
/// compiler instead: these arrays are the only places a variant is ever
/// constructed and `ALL` is `#[cfg(test)]`, so a column absent from all three
/// is `dead_code` in the non-test build, which CI compiles with `-D warnings`.
/// The property holds, but by two mechanisms and neither one alone.
#[cfg(test)]
const ALL: [SessionColumn; 15] = [
    SessionColumn::Session,
    SessionColumn::Client,
    SessionColumn::Model,
    SessionColumn::Turn,
    SessionColumn::Msgs,
    SessionColumn::Input,
    SessionColumn::Output,
    SessionColumn::CacheRead,
    SessionColumn::CacheWrite,
    SessionColumn::CacheHit,
    SessionColumn::Total,
    SessionColumn::Cost,
    SessionColumn::CostPerMillion,
    SessionColumn::Duration,
    SessionColumn::LastActive,
];

/// Left-to-right display order. Cosmetic: reshuffling it never changes what
/// fits, only where it sits.
const WIDE_ORDER: [SessionColumn; 15] = [
    SessionColumn::Session,
    SessionColumn::Client,
    SessionColumn::Model,
    SessionColumn::Turn,
    SessionColumn::Msgs,
    SessionColumn::Input,
    SessionColumn::Output,
    SessionColumn::CacheRead,
    SessionColumn::CacheWrite,
    SessionColumn::CacheHit,
    SessionColumn::Total,
    SessionColumn::Cost,
    SessionColumn::CostPerMillion,
    SessionColumn::Duration,
    SessionColumn::LastActive,
];

/// Admission order: earlier groups are admitted first and dropped last. Each
/// group is all-or-nothing — `Input` without `Output` invites a reader to take
/// Input for a total, and `Cache R` without `Cache W` is the same trap.
///
/// The core is one group rather than six so admission can never stop *inside*
/// it: the wide layout always shows at least the column set the narrow layout
/// shows, which makes crossing 80 columns widen the table instead of reshaping
/// it. The tail order is decision A in `docs/sessions-column-budget.md` (#965),
/// ranked by whether a column helps you find and compare sessions in a list.
const WIDE_PRIORITY: [&[SessionColumn]; 8] = [
    &[
        SessionColumn::Session,
        SessionColumn::Client,
        SessionColumn::Total,
        SessionColumn::Cost,
        SessionColumn::Msgs,
        SessionColumn::Turn,
    ],
    &[SessionColumn::LastActive],
    &[SessionColumn::Model],
    &[SessionColumn::Input, SessionColumn::Output],
    &[SessionColumn::CacheRead, SessionColumn::CacheWrite],
    &[SessionColumn::Duration],
    &[SessionColumn::CacheHit],
    &[SessionColumn::CostPerMillion],
];

/// Everything known *before* admission runs. Widths that depend on the admitted
/// set are deliberately absent: `natural()` is what the budget sums, so it has
/// to be answerable before anything has been admitted.
struct WideCtx {
    has_turn: bool,
    last_active_fmt: &'static str,
}

/// Produced by admission, consumed by rendering. Every width that depends on
/// the chosen set lives here and nowhere else.
struct WideLayout {
    chosen: Vec<SessionColumn>,
    /// Cells the Model column is laid out with, `natural(Model)` plus its share
    /// of the slack. `build_model_cell`'s ellipsis budget must equal this or
    /// multi-model sessions get re-cut by the solver and render as single-model.
    model_width: u16,
    /// What `Session`'s cell truncates its title to. Session is the sole `Min`
    /// column, so ratatui hands it every cell slack distribution did not give
    /// Model; this has to match that or the title is clipped without an ellipsis.
    session_width: u16,
}

impl SessionColumn {
    fn header(self) -> &'static str {
        match self {
            Self::Session => "Session",
            Self::Client => "Client",
            Self::Model => "Model",
            Self::Turn => "Turn",
            Self::Msgs => "Msgs",
            Self::Input => "Input",
            Self::Output => "Output",
            Self::CacheRead => "Cache R",
            Self::CacheWrite => "Cache W",
            Self::CacheHit => "Cache×",
            Self::Total => "Total",
            Self::Cost => "Cost",
            Self::CostPerMillion => "Cost/1M",
            Self::Duration => "Duration",
            Self::LastActive => "Last Active",
        }
    }

    /// Whether the data supports this column at all, independent of width.
    fn available(self, ctx: &WideCtx) -> bool {
        match self {
            Self::Turn => ctx.has_turn,
            Self::Session
            | Self::Client
            | Self::Model
            | Self::Msgs
            | Self::Input
            | Self::Output
            | Self::CacheRead
            | Self::CacheWrite
            | Self::CacheHit
            | Self::Total
            | Self::Cost
            | Self::CostPerMillion
            | Self::Duration
            | Self::LastActive => true,
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
            Self::Session
            | Self::Client
            | Self::Model
            | Self::Turn
            | Self::Msgs
            | Self::Input
            | Self::Output
            | Self::CacheRead
            | Self::CacheWrite
            | Self::CacheHit
            | Self::CostPerMillion
            | Self::Duration => None,
        }
    }

    /// Cells this column needs to render its widest realistic value, and the
    /// only width input to admission. `Constraint`s are *derived* from this
    /// rather than written beside it: a second literal is exactly the
    /// budget-versus-layout divergence this design exists to kill.
    ///
    /// "Realistic" is doing real work in that sentence, so `fit()` enforces
    /// these numbers on the cell as well rather than trusting them.
    fn natural(self, ctx: &WideCtx) -> u16 {
        match self {
            Self::Session => 20,
            // The widest name the client registry can produce
            // (`Antigravity CLI`), pinned by
            // `client_column_fits_every_registered_client`. At 12 this column
            // clipped three shipped clients with no marker, and it rendered
            // `Antigravity CLI` and `Antigravity` identically — a
            // misattribution in the column whose only job is saying which tool
            // the session came from.
            Self::Client => 15,
            Self::Model => 18,
            Self::Turn => 5,
            // Msgs borrows the cell Turn gives up. Keyed on the data rather
            // than on whether Turn was admitted, so the answer exists before
            // admission runs — and every no-turn threshold depends on it.
            Self::Msgs => {
                if ctx.has_turn {
                    5
                } else {
                    6
                }
            }
            Self::Input | Self::Output | Self::CacheRead | Self::CacheWrite => 10,
            Self::CacheHit => 8,
            // `format_cost` switches to `${:.1}K` past $1000 and
            // `format_cost_per_million` has no compact branch at all, so
            // neither of these tightens below 10 without truncating.
            Self::Total | Self::Cost | Self::CostPerMillion => 10,
            Self::Duration => 9,
            Self::LastActive => 17,
        }
    }

    /// Whether leftover cells may flow here. Session is the sole sink, which is
    /// what turns admission's slack into session-title width.
    fn grows(self) -> bool {
        match self {
            Self::Session => true,
            Self::Client
            | Self::Model
            | Self::Turn
            | Self::Msgs
            | Self::Input
            | Self::Output
            | Self::CacheRead
            | Self::CacheWrite
            | Self::CacheHit
            | Self::Total
            | Self::Cost
            | Self::CostPerMillion
            | Self::Duration
            | Self::LastActive => false,
        }
    }

    fn cell(
        self,
        s: &SessionUsage,
        app: &App,
        ctx: &WideCtx,
        layout: &WideLayout,
    ) -> Cell<'static> {
        match self {
            Self::Session => Cell::from(truncate_to_width(
                session_label(s),
                layout.session_width as usize,
            ))
            .style(
                Style::default()
                    .fg(app.theme.muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Self::Client => Cell::from(self.fit(get_compact_client_display_name(&s.client), ctx))
                .style(Style::default().fg(app.theme.muted)),
            Self::Model => build_model_cell(&s.models, layout.model_width as usize, app),
            Self::Turn => Cell::from(self.fit(
                if s.turn_count > 0 {
                    s.turn_count.to_string()
                } else {
                    "\u{2014}".to_string()
                },
                ctx,
            )),
            Self::Msgs => Cell::from(self.fit(s.message_count.to_string(), ctx)),
            Self::Input => Cell::from(self.fit(format_tokens(s.tokens.input), ctx))
                .style(app.theme.metric_input_style()),
            Self::Output => Cell::from(self.fit(format_tokens(s.tokens.output), ctx))
                .style(app.theme.metric_output_style()),
            Self::CacheRead => Cell::from(self.fit(format_tokens(s.tokens.cache_read), ctx))
                .style(app.theme.metric_cache_read_style()),
            Self::CacheWrite => Cell::from(self.fit(format_tokens(s.tokens.cache_write), ctx))
                .style(app.theme.metric_cache_write_style()),
            Self::CacheHit => Cell::from(self.fit(
                format_cache_hit_rate(s.tokens.cache_read, s.tokens.input, s.tokens.cache_write),
                ctx,
            ))
            .style(Style::default().fg(Color::Cyan)),
            // Spelled out rather than `total_tokens_cell` — same style, but the
            // helper has no width to clamp against.
            Self::Total => Cell::from(self.fit(format_tokens(s.tokens.total()), ctx))
                .style(app.theme.metric_total_style()),
            Self::Cost => Cell::from(self.fit(format_cost(s.cost), ctx))
                .style(Style::default().fg(Color::Green)),
            Self::CostPerMillion => {
                Cell::from(self.fit(format_cost_per_million(s.cost, s.tokens.total()), ctx))
                    .style(Style::default().fg(Color::Rgb(150, 200, 150)))
            }
            Self::Duration => {
                Cell::from(self.fit(format_duration(s.first_active_ms, s.last_active_ms), ctx))
                    .style(Style::default().fg(Color::Yellow))
            }
            Self::LastActive => Cell::from(
                self.fit(
                    ms_to_local_naive(s.last_active_ms)
                        .map(|dt| dt.format(ctx.last_active_fmt).to_string())
                        .unwrap_or_else(|| "\u{2014}".to_string()),
                    ctx,
                ),
            )
            .style(Style::default().fg(app.theme.muted)),
        }
    }

    /// Clamp a fixed-width column's text to the width it was laid out with.
    ///
    /// A no-op on every value the column was sized for — `natural()` is the
    /// widest *realistic* one — but "realistic" is an assumption about parsed
    /// data, not a guarantee about it, and the failure mode when it is wrong is
    /// the one this whole file exists to remove: 1,234,567 messages rendering
    /// as `12345`, a bad `first_active_ms` rendering `20092d 14h` as
    /// `20092d 14`. Clipped by ratatui those are plausible wrong numbers;
    /// clipped here they are `12...` and `20092d...`, which are visibly
    /// incomplete. `TokscaleConfig` client names are the one case that is
    /// user-controlled rather than absurd.
    ///
    /// Session and Model are absent because their width comes from the
    /// resolved `WideLayout` rather than from `natural()`; they clamp against
    /// `session_width` and `model_width` at their own call sites.
    fn fit(self, text: String, ctx: &WideCtx) -> String {
        truncate_to_width(&text, self.natural(ctx) as usize)
    }
}

impl WideLayout {
    /// The constraint a column is laid out with, derived from `natural()`.
    /// Model is the one fixed column that absorbs slack, so its length comes
    /// from the resolved layout; Session is `Min` and takes the remainder.
    fn constraint(&self, c: SessionColumn, ctx: &WideCtx) -> Constraint {
        let width = match c {
            SessionColumn::Model => self.model_width,
            SessionColumn::Session
            | SessionColumn::Client
            | SessionColumn::Turn
            | SessionColumn::Msgs
            | SessionColumn::Input
            | SessionColumn::Output
            | SessionColumn::CacheRead
            | SessionColumn::CacheWrite
            | SessionColumn::CacheHit
            | SessionColumn::Total
            | SessionColumn::Cost
            | SessionColumn::CostPerMillion
            | SessionColumn::Duration
            | SessionColumn::LastActive => c.natural(ctx),
        };
        if c.grows() {
            Constraint::Min(width)
        } else {
            Constraint::Length(width)
        }
    }
}

/// Position in the display order. Total rather than `unwrap()`: a column
/// missing from `WIDE_ORDER` sorts last instead of panicking inside the draw
/// loop, which would leave the terminal in raw mode. The permutation test is
/// what actually catches it.
fn order_index(c: SessionColumn) -> usize {
    WIDE_ORDER
        .iter()
        .position(|o| *o == c)
        .unwrap_or(usize::MAX)
}

/// Cells a column set occupies: the widths themselves plus one separator
/// between every pair (ratatui's `column_spacing`, which defaults to 1).
/// Measured against `inner`, which already has the block borders removed.
///
/// The `1` is not a free-floating assumption: at every group threshold the
/// admitted set exactly fills `inner`, so a second cell of spacing pushes the
/// last column past the border and
/// `admitted_columns_render_header_and_value_in_full` goes red. Setting
/// `.column_spacing(2)` on the table fails six of the tests below.
fn required(set: &[SessionColumn], ctx: &WideCtx) -> u16 {
    let widths: u16 = set.iter().map(|c| c.natural(ctx)).sum();
    widths + set.len().saturating_sub(1) as u16
}

/// Admit whole priority groups while they fit, then resolve the slack.
fn admit_and_distribute(available: u16, ctx: &WideCtx) -> WideLayout {
    let mut chosen: Vec<SessionColumn> = Vec::new();
    for group in WIDE_PRIORITY {
        let admissible: Vec<SessionColumn> =
            group.iter().copied().filter(|c| c.available(ctx)).collect();
        if admissible.is_empty() {
            continue; // e.g. Turn with no turn data
        }
        let mut next = chosen.clone();
        next.extend(&admissible);
        if required(&next, ctx) <= available {
            chosen = next;
        } else {
            // Stop, don't skip. Trying the next (narrower) group packs more
            // columns in but makes the admitted set non-monotonic in width —
            // Cache× at W, Duration and no Cache× at W+1 — which is the same
            // disorientation the ratatui solver produces today.
            break;
        }
    }
    chosen.sort_by_key(|c| order_index(*c));

    let slack = available.saturating_sub(required(&chosen, ctx));
    let session_min = SessionColumn::Session.natural(ctx);
    let model_min = SessionColumn::Model.natural(ctx);
    // Saturating because the caps are constants and the minima come from
    // `natural()`: raising a minimum past its cap should cost that column its
    // growth, not panic inside the draw loop.
    let session_extra = slack.min(SESSION_COLUMN_COMFORTABLE_CHARS.saturating_sub(session_min));
    let model_extra = if chosen.contains(&SessionColumn::Model) {
        (slack - session_extra).min(MODEL_COLUMN_MAX_CHARS.saturating_sub(model_min))
    } else {
        0
    };

    WideLayout {
        model_width: model_min + model_extra,
        // Whatever Model did not take lands on Session through its `Min`
        // constraint, so this is what ratatui will actually hand the column.
        session_width: session_min + slack - model_extra,
        chosen,
    }
}

/// The human-readable title when the source client stored one, falling back to
/// the session ID for clients that don't.
fn session_label(s: &SessionUsage) -> &str {
    s.title
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(&s.session_id)
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(
            " Sessions ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(app.theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height.saturating_sub(1) as usize;
    app.set_max_visible_items(visible_height);

    let sessions = app.get_sorted_sessions();
    if sessions.is_empty() {
        let empty_msg = Paragraph::new("No session usage data found. Press 'r' to refresh.")
            .style(Style::default().fg(app.theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let is_narrow = app.is_narrow();
    let is_very_narrow = app.is_very_narrow();
    let has_turn_data = sessions.iter().any(|s| s.turn_count > 0);
    let sort_field = app.sort_field;
    let sort_direction = app.sort_direction;
    let scroll_offset = app.scroll_offset;
    let selected_index = app.selected_index;
    let theme_accent = app.theme.accent;
    let theme_muted = app.theme.muted;
    let theme_selection = app.theme.selection;
    let striped_row_style = app.theme.striped_row_style();

    // Timestamp format adapts to width: full date+time when there's room,
    // compact "mm-dd HH:MM" when the full layout would overflow.
    let last_active_fmt: &'static str = if is_narrow || is_very_narrow {
        "%m-%d %H:%M"
    } else {
        "%Y-%m-%d %H:%M"
    };

    // The wide layout is budget-driven: a column is admitted at its natural
    // width or it is not shown. Measured against `inner` rather than the
    // terminal so the budget equals the width ratatui actually divides up.
    let wide = (!is_narrow && !is_very_narrow)
        .then(|| {
            let ctx = WideCtx {
                has_turn: has_turn_data,
                last_active_fmt,
            };
            let layout = admit_and_distribute(inner.width, &ctx);
            (ctx, layout)
        })
        // The core group is atomic, so an `inner` narrower than its budget
        // admits nothing at all and the table would render as an empty block —
        // no header, no rows, no message. The gate above makes that
        // unreachable through `ui::render`, which passes an `area` as wide as
        // the terminal, but `render` is `pub` and takes `area` from its caller,
        // so the precondition is one call site away rather than impossible.
        // Falling back to the narrow layout costs nothing: it is
        // percentage-based and renders something at any width.
        .filter(|(_, layout)| !layout.chosen.is_empty());

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

    let header_cells: Vec<String> = if let Some((_, layout)) = &wide {
        // Headers carry the sort arrow, so this is one derivation and not two.
        // Nothing computes a column index at all, so the arrow cannot land on
        // the wrong column, and a sorted-but-unadmitted column simply has no
        // descriptor to match — which is what the narrow branches fake with a
        // `usize::MAX` sentinel below.
        layout
            .chosen
            .iter()
            .map(|c| {
                let indicator = c.sort_field().map(sort_indicator).unwrap_or("");
                format!("{}{}", c.header(), indicator)
            })
            .collect()
    } else {
        let labels: &[&str] = if is_very_narrow {
            &["Session", "Cost"]
        } else if has_turn_data {
            &["Session", "Client", "Turn", "Msgs", "Tokens", "Cost"]
        } else {
            &["Session", "Client", "Msgs", "Tokens", "Cost"]
        };
        // The narrow layouts keep their hand-picked indices, and `usize::MAX`
        // still stands for "this sort has no column here".
        let (total_idx, cost_idx) = if is_very_narrow {
            (usize::MAX, 1)
        } else if has_turn_data {
            (4, 5)
        } else {
            (3, 4)
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

    let sessions_len = sessions.len();
    let start = scroll_offset.min(sessions_len);
    let end = (start + visible_height).min(sessions_len);

    if start >= sessions_len {
        return;
    }

    let rows: Vec<Row> = sessions[start..end]
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let idx = i + start;
            let is_selected = idx == selected_index;
            let is_striped = idx % 2 == 1;

            let cells: Vec<Cell> = if let Some((ctx, layout)) = &wide {
                layout
                    .chosen
                    .iter()
                    .map(|c| c.cell(session, app, ctx, layout))
                    .collect()
            } else if is_very_narrow {
                vec![
                    Cell::from(truncate_text(session_label(session), 20)).style(
                        Style::default()
                            .fg(theme_muted)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(format_cost(session.cost)).style(Style::default().fg(Color::Green)),
                ]
            } else {
                let mut cells = vec![Cell::from(truncate_text(session_label(session), 24)).style(
                    Style::default()
                        .fg(theme_muted)
                        .add_modifier(Modifier::BOLD),
                )];
                cells.push(
                    Cell::from(get_compact_client_display_name(&session.client))
                        .style(Style::default().fg(theme_muted)),
                );
                if has_turn_data {
                    let turn_str = if session.turn_count > 0 {
                        session.turn_count.to_string()
                    } else {
                        "\u{2014}".to_string()
                    };
                    cells.push(Cell::from(turn_str));
                }
                cells.extend([
                    Cell::from(session.message_count.to_string()),
                    total_tokens_cell(session.tokens.total(), &app.theme),
                    Cell::from(format_cost(session.cost)).style(Style::default().fg(Color::Green)),
                ]);
                cells
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

    let widths: Vec<Constraint> = if let Some((ctx, layout)) = &wide {
        layout
            .chosen
            .iter()
            .map(|c| layout.constraint(*c, ctx))
            .collect()
    } else if is_very_narrow {
        vec![Constraint::Percentage(60), Constraint::Percentage(40)]
    } else if has_turn_data {
        vec![
            Constraint::Percentage(28),
            Constraint::Percentage(16),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
        ]
    } else {
        vec![
            Constraint::Percentage(32),
            Constraint::Percentage(18),
            Constraint::Percentage(12),
            Constraint::Percentage(18),
            Constraint::Percentage(20),
        ]
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme_selection));

    frame.render_widget(table, inner);

    if sessions_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));

        let mut scrollbar_state =
            viewport_scrollbar_state(sessions_len, scroll_offset, visible_height);

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

/// Build a table cell for the Model column with each model name colored by the
/// same family-shade system used in the Overview and Models tabs. Multiple
/// models are joined with `", "`. The total width is truncated to `max_cells`
/// with an ellipsis when it overflows, counted in terminal cells to match
/// `truncate_to_width`, which measures the Session and Client cells in this
/// table, and to match the unit ratatui lays the column out in.
///
/// A multi-model session always ends in an ellipsis when some of its models did
/// not fit, so a session that switched models never renders as a single-model
/// one. One cell is held back for that marker when there is more than one model.
fn build_model_cell(models: &[SessionModel], max_cells: usize, app: &App) -> Cell<'static> {
    if models.is_empty() {
        return Cell::from("\u{2014}".to_string()).style(Style::default().fg(app.theme.muted));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut budget = max_cells.saturating_sub(usize::from(models.len() > 1));
    let mut shown = 0usize;
    let mut ellipsis = false;

    for (i, model) in models.iter().enumerate() {
        if i > 0 {
            if budget < 3 {
                break;
            }
            spans.push(Span::styled(", ", Style::default().fg(app.theme.muted)));
            budget -= 2;
        }

        if budget == 0 {
            break;
        }

        let color = app.model_color_for(&model.provider, &model.color_key);
        let name = &model.display_name;
        let model_len = display_width(name);

        if model_len <= budget {
            spans.push(Span::styled(name.clone(), Style::default().fg(color)));
            budget -= model_len;
        } else if budget <= 1 {
            spans.push(Span::styled("…".to_string(), Style::default().fg(color)));
            budget = 0;
            ellipsis = true;
        } else {
            let head = prefix_to_width(name, budget - 1);
            spans.push(Span::styled(
                format!("{}…", head),
                Style::default().fg(color),
            ));
            budget = 0;
            ellipsis = true;
        }
        shown += 1;
    }

    // Dropping whole models silently would misreport the session; a truncated
    // name already carries its own ellipsis, so only mark the clean-break case.
    if shown < models.len() && !ellipsis {
        spans.push(Span::styled(
            "…".to_string(),
            Style::default().fg(app.theme.muted),
        ));
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

/// Human-readable elapsed time between first and last message in a session.
/// Returns "—" when the bounds are missing or inverted.
fn format_duration(first_ms: i64, last_ms: i64) -> String {
    if first_ms <= 0 || last_ms <= 0 || last_ms < first_ms {
        return "\u{2014}".to_string();
    }
    let secs = (last_ms - first_ms) / 1000;
    if secs <= 0 {
        return "0s".to_string();
    }
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        format!("{}s", s)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{Tab, TuiConfig};
    use crate::tui::data::TokenBreakdown;
    use ratatui::{backend::TestBackend, Terminal};
    use tokscale_core::ClientId;

    /// Terminal width at which each priority group first appears.
    ///
    /// **Literals, never derived from `admit_and_distribute`.** A test that
    /// recomputes the formula it is testing cannot fail — which is exactly why
    /// the old `model_gate_width` helper never caught that the wide layout asked
    /// for 161 columns and turned on at 80. Amending decision A moves these
    /// numbers, and the diff then names which threshold moved and by how much.
    ///
    /// Every number here is three higher than the first draft of this table:
    /// `natural(Client)` went 12 → 15 so `Antigravity CLI` stops rendering as
    /// `Antigravity`, and a widened column pushes every later group right by
    /// exactly its three cells.
    const THRESHOLDS_WITH_TURN: [(u16, &[SessionColumn]); 8] = [
        (
            72,
            &[
                SessionColumn::Session,
                SessionColumn::Client,
                SessionColumn::Turn,
                SessionColumn::Msgs,
                SessionColumn::Total,
                SessionColumn::Cost,
            ],
        ),
        (90, &[SessionColumn::LastActive]),
        (109, &[SessionColumn::Model]),
        (131, &[SessionColumn::Input, SessionColumn::Output]),
        (153, &[SessionColumn::CacheRead, SessionColumn::CacheWrite]),
        (163, &[SessionColumn::Duration]),
        (172, &[SessionColumn::CacheHit]),
        (183, &[SessionColumn::CostPerMillion]),
    ];

    /// Same table without turn data. Every threshold is five narrower: Turn's
    /// five cells and its separator go away, and Msgs takes one of them back.
    const THRESHOLDS_NO_TURN: [(u16, &[SessionColumn]); 8] = [
        (
            67,
            &[
                SessionColumn::Session,
                SessionColumn::Client,
                SessionColumn::Msgs,
                SessionColumn::Total,
                SessionColumn::Cost,
            ],
        ),
        (85, &[SessionColumn::LastActive]),
        (104, &[SessionColumn::Model]),
        (126, &[SessionColumn::Input, SessionColumn::Output]),
        (148, &[SessionColumn::CacheRead, SessionColumn::CacheWrite]),
        (158, &[SessionColumn::Duration]),
        (167, &[SessionColumn::CacheHit]),
        (178, &[SessionColumn::CostPerMillion]),
    ];

    /// Widest terminal the sweeps cover. Past the last threshold (183) nothing
    /// changes but the Session column's share of the slack.
    const SWEEP_MAX: u16 = 240;

    fn thresholds(has_turn: bool) -> &'static [(u16, &'static [SessionColumn])] {
        if has_turn {
            &THRESHOLDS_WITH_TURN
        } else {
            &THRESHOLDS_NO_TURN
        }
    }

    /// Columns the reviewed threshold table says a terminal of `width` shows.
    fn expected_columns(width: u16, has_turn: bool) -> Vec<SessionColumn> {
        let mut columns: Vec<SessionColumn> = Vec::new();
        for (threshold, group) in thresholds(has_turn) {
            if width < *threshold {
                break;
            }
            columns.extend(*group);
        }
        columns.sort_by_key(|c| *c as usize);
        columns
    }

    /// Columns whose header label appears in a rendered header line. `Cost` is a
    /// substring of `Cost/1M`, so the longer label is stripped before looking for
    /// the shorter one; no other pair of labels overlaps.
    fn header_columns(header: &str) -> Vec<SessionColumn> {
        let without_cost_per_million = header.replace(SessionColumn::CostPerMillion.header(), "");
        ALL.iter()
            .copied()
            .filter(|c| {
                let haystack = if *c == SessionColumn::Cost {
                    without_cost_per_million.as_str()
                } else {
                    header
                };
                haystack.contains(c.header())
            })
            .collect()
    }

    fn wide_ctx(has_turn: bool) -> WideCtx {
        WideCtx {
            has_turn,
            last_active_fmt: "%Y-%m-%d %H:%M",
        }
    }

    fn session(id: &str, client: &str, cost: f64, last_ms: i64) -> SessionUsage {
        SessionUsage {
            session_id: id.to_string(),
            client: client.to_string(),
            title: None,
            models: vec![SessionModel {
                display_name: "test-model".to_string(),
                provider: "test".to_string(),
                color_key: "test-model".to_string(),
            }],
            tokens: TokenBreakdown {
                input: 1000,
                output: 500,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            message_count: 4,
            turn_count: 2,
            first_active_ms: last_ms.saturating_sub(3_600_000),
            last_active_ms: last_ms,
        }
    }

    const FAT_SESSION_LAST_MS: i64 = 1_736_000_000_000;

    /// A session whose every column has a value long enough that clipping it
    /// would still look like a number. This is the shape of the original bug:
    /// at 80 columns Output rendered `234` for 234,567 tokens.
    fn fat_session() -> SessionUsage {
        let mut s = session("my-session", "opencode", 12.3456, FAT_SESSION_LAST_MS);
        s.models = vec![SessionModel {
            display_name: "claude-sonnet-4".to_string(),
            provider: "anthropic".to_string(),
            color_key: "claude-sonnet-4".to_string(),
        }];
        s.tokens = TokenBreakdown {
            input: 1_234_567,
            output: 234_567,
            cache_read: 45_678_901,
            cache_write: 2_345_678,
            reasoning: 0,
        };
        s.message_count = 428;
        s.turn_count = 137;
        s
    }

    /// What `fat_session()` must render, in full, in every column that is
    /// admitted. Exhaustive so a new column cannot skip this assertion.
    fn expected_value(c: SessionColumn) -> String {
        match c {
            SessionColumn::Session => "my-session".to_string(),
            SessionColumn::Client => "OpenCode".to_string(),
            SessionColumn::Model => "claude-sonnet-4".to_string(),
            SessionColumn::Turn => "137".to_string(),
            SessionColumn::Msgs => "428".to_string(),
            SessionColumn::Input => "1.2M".to_string(),
            SessionColumn::Output => "234K".to_string(),
            SessionColumn::CacheRead => "45.7M".to_string(),
            SessionColumn::CacheWrite => "2.3M".to_string(),
            SessionColumn::CacheHit => "12.8x".to_string(),
            SessionColumn::Total => "49.5M".to_string(),
            SessionColumn::Cost => "$12.35".to_string(),
            SessionColumn::CostPerMillion => "$0.25".to_string(),
            SessionColumn::Duration => "1h 0m".to_string(),
            // The one value that depends on the machine's time zone, so it is
            // formatted rather than hardcoded. Still catches clipping: the claim
            // is that all sixteen characters survive the layout.
            SessionColumn::LastActive => ms_to_local_naive(FAT_SESSION_LAST_MS)
                .expect("fixture timestamp is in range")
                .format("%Y-%m-%d %H:%M")
                .to_string(),
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
        app.current_tab = Tab::Sessions;
        app.sort_field = SortField::Cost;
        app.sort_direction = SortDirection::Descending;
        app
    }

    fn app_with(width: u16, sessions: Vec<SessionUsage>) -> App {
        let mut app = make_app(width);
        app.data.sessions = sessions;
        app
    }

    fn render_body(app: &mut App, width: u16, height: u16) -> String {
        render_into(app, width, height, Rect::new(0, 0, width, height))
    }

    /// `render_body` with the drawing area decoupled from the terminal, which is
    /// the only way to reach an `area` narrower than `app.terminal_width`.
    ///
    /// A wide grapheme occupies two cells and ratatui stores its symbol in the
    /// first and an empty string in the second, so a returned line has one
    /// `char` per grapheme, not one per cell.
    fn render_into(app: &mut App, width: u16, height: u16, area: Rect) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app, area)).unwrap();
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

    /// The header is the first line inside the block border.
    fn header_line(app: &mut App, width: u16) -> String {
        render_body(app, width, 6)
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    }

    /// The first data row, directly under the header.
    fn first_row_line(app: &mut App, width: u16) -> String {
        render_body(app, width, 6)
            .lines()
            .nth(2)
            .unwrap_or_default()
            .to_string()
    }

    // ---- the descriptor arrays -------------------------------------------

    /// The exhaustive `match`es cover the six descriptor methods and stop there:
    /// `WIDE_ORDER` and `WIDE_PRIORITY` are hand-maintained arrays of column
    /// names that compile fine while missing a variant. Absent from
    /// `WIDE_PRIORITY` a column is never admitted at any width and silently
    /// never renders; absent from `WIDE_ORDER` its sort key has no answer. This
    /// is the only assertion that enforces the no-drift property the whole
    /// design is sold on.
    #[test]
    fn every_column_is_ordered_and_prioritized() {
        let mut order = WIDE_ORDER.to_vec();
        let mut priority: Vec<SessionColumn> = WIDE_PRIORITY.concat();
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
        for has_turn in [true, false] {
            let ctx = wide_ctx(has_turn);
            for (threshold, group) in thresholds(has_turn) {
                let at = admit_and_distribute(threshold - 2, &ctx).chosen;
                for column in *group {
                    assert!(
                        at.contains(column),
                        "{:?} should be admitted at {threshold} columns (turn={has_turn}), got {at:?}",
                        column
                    );
                }
                let below = admit_and_distribute(threshold - 3, &ctx).chosen;
                for column in *group {
                    assert!(
                        !below.contains(column),
                        "{:?} should not be admitted at {} columns (turn={has_turn}), got {below:?}",
                        column,
                        threshold - 1
                    );
                }
            }
        }
    }

    /// Every threshold above the wide layout's 80-column entry point, checked
    /// through an actual render rather than through `admit_and_distribute`.
    #[test]
    fn group_thresholds_hold_when_actually_rendered() {
        for has_turn in [true, false] {
            for (threshold, group) in thresholds(has_turn) {
                if *threshold <= 80 {
                    continue; // below the wide layout's entry condition
                }
                for (width, want) in [(threshold - 1, false), (*threshold, true)] {
                    let mut s = fat_session();
                    if !has_turn {
                        s.turn_count = 0;
                    }
                    let mut app = app_with(width, vec![s]);
                    let header = header_line(&mut app, width);
                    for column in *group {
                        assert_eq!(
                            header.contains(column.header()),
                            want,
                            "{:?} at width {width} (turn={has_turn}) should be {}\n{header}",
                            column,
                            if want { "shown" } else { "hidden" }
                        );
                    }
                }
            }
        }
    }

    /// The wide core is the narrow column set, so crossing 80 columns widens the
    /// table instead of reshaping it. The core is one atomic priority group
    /// precisely so admission can never stop inside it.
    #[test]
    fn wide_layout_never_drops_below_the_narrow_column_set() {
        for has_turn in [true, false] {
            let ctx = wide_ctx(has_turn);
            let core = thresholds(has_turn)[0].1;
            let floor = if has_turn { 70 } else { 65 };
            assert_eq!(
                required(core, &ctx),
                floor,
                "the core column set costs {floor} cells (turn={has_turn})"
            );
            // 78 is the narrowest `inner` the wide path ever sees (80 - borders).
            for available in floor..=SWEEP_MAX {
                let chosen = admit_and_distribute(available, &ctx).chosen;
                for column in core {
                    assert!(
                        chosen.contains(column),
                        "{:?} dropped from the core at inner width {available} (turn={has_turn})",
                        column
                    );
                }
            }
        }
    }

    /// Admission takes whole groups, so an area narrower than the core group's
    /// budget admits nothing and the wide branch would draw an empty block —
    /// no header, no rows, no message. `ui::render` cannot produce that, but
    /// `render` takes `area` from its caller and asserts the precondition
    /// nowhere, so the fallback is what makes the blank panel unreachable
    /// rather than merely unreached.
    #[test]
    fn an_area_below_the_core_budget_falls_back_instead_of_rendering_nothing() {
        for area_width in [30u16, 40, 50, 60, 68, 71] {
            let mut app = app_with(200, vec![fat_session()]);
            let body = render_into(&mut app, 200, 6, Rect::new(0, 0, area_width, 6));
            let mut lines = body.lines().skip(1);
            let header = lines.next().unwrap_or_default();
            let row = lines.next().unwrap_or_default();
            // Cost is the last column of every narrow variant, so its header
            // and its value are present whichever one the fallback picked.
            assert!(
                header.contains("Cost"),
                "blank Sessions header at area width {area_width}\n{body}"
            );
            assert!(
                row.contains("$12"),
                "blank Sessions row at area width {area_width}\n{body}"
            );
        }
    }

    #[test]
    fn the_admitted_set_always_fits_the_terminal() {
        for width in 80u16..=SWEEP_MAX {
            for has_turn in [true, false] {
                let ctx = wide_ctx(has_turn);
                let available = width - 2;
                let chosen = admit_and_distribute(available, &ctx).chosen;
                assert!(
                    required(&chosen, &ctx) <= available,
                    "admitted set overflows at width {width} (turn={has_turn}): {chosen:?}"
                );
            }
        }
    }

    // ---- the invariants over a full width sweep ---------------------------

    /// Every admitted column renders its header *and* its value in full, at
    /// every width. This is the assertion that would have turned #964 into a red
    /// test: today it fails from 80 to 154 with turn data, because ratatui clips
    /// cell content with no ellipsis and 234,567 tokens render as `234`.
    #[test]
    fn admitted_columns_render_header_and_value_in_full() {
        for width in 80u16..=SWEEP_MAX {
            for has_turn in [true, false] {
                let mut s = fat_session();
                if !has_turn {
                    s.turn_count = 0;
                }
                let mut app = app_with(width, vec![s]);
                let body = render_body(&mut app, width, 6);
                let mut lines = body.lines().skip(1);
                let header = lines.next().unwrap_or_default();
                let row = lines.next().unwrap_or_default();
                for column in expected_columns(width, has_turn) {
                    assert!(
                        header.contains(column.header()),
                        "header {:?} squeezed at width {width} (turn={has_turn})\n{header}",
                        column
                    );
                    // Turn renders an em-dash when the session has no turns.
                    if column == SessionColumn::Turn && !has_turn {
                        continue;
                    }
                    assert!(
                        row.contains(&expected_value(column)),
                        "value for {:?} truncated at width {width} (turn={has_turn}); \
                         expected {:?}\n{header}\n{row}",
                        column,
                        expected_value(column)
                    );
                }
            }
        }
    }

    /// A column that is not admitted leaves nothing behind — no stub header and,
    /// more importantly, no half of a number.
    #[test]
    fn unadmitted_columns_leave_no_trace() {
        for width in 80u16..=SWEEP_MAX {
            for has_turn in [true, false] {
                let mut s = fat_session();
                if !has_turn {
                    s.turn_count = 0;
                }
                let mut app = app_with(width, vec![s]);
                let body = render_body(&mut app, width, 6);
                let mut lines = body.lines().skip(1);
                let header = lines.next().unwrap_or_default();
                let row = lines.next().unwrap_or_default();
                let admitted = expected_columns(width, has_turn);
                for column in ALL {
                    if admitted.contains(&column) {
                        continue;
                    }
                    assert!(
                        !header.contains(column.header()),
                        "{:?} header showed at width {width} (turn={has_turn})\n{header}",
                        column
                    );
                    assert!(
                        !row.contains(&expected_value(column)),
                        "{:?} value showed at width {width} (turn={has_turn})\n{row}",
                        column
                    );
                }
            }
        }
    }

    /// The active sort arrow is legible whenever its column is admitted, and
    /// absent from the whole header when it is not. Today `Last Active ▼` is
    /// missing at 77 of 121 widths.
    ///
    /// Both directions, because `make_app` fixes the sort to descending and an
    /// ascending-only clipping bug would otherwise pass. Placement is
    /// direction-independent by construction — `sort_indicator` picks the glyph
    /// and the header is formatted the same either way — so this is coverage of
    /// the glyph, not of a second code path.
    #[test]
    fn sort_indicator_is_legible_exactly_when_its_column_is_admitted() {
        for width in 80u16..=SWEEP_MAX {
            for has_turn in [true, false] {
                for (direction, arrow) in [
                    (SortDirection::Descending, '▼'),
                    (SortDirection::Ascending, '▲'),
                ] {
                    for (field, column) in [
                        (SortField::Cost, SessionColumn::Cost),
                        (SortField::Tokens, SessionColumn::Total),
                        (SortField::Date, SessionColumn::LastActive),
                    ] {
                        let mut s = fat_session();
                        if !has_turn {
                            s.turn_count = 0;
                        }
                        let mut app = app_with(width, vec![s]);
                        app.sort_field = field;
                        app.sort_direction = direction;
                        let header = header_line(&mut app, width);
                        if expected_columns(width, has_turn).contains(&column) {
                            assert!(
                                header.contains(&format!("{} {arrow}", column.header())),
                                "{:?} {arrow} clipped at width {width} (turn={has_turn})\n{header}",
                                field
                            );
                        } else {
                            assert!(
                                !header.contains(arrow),
                                "{:?} drew {arrow} with its column unadmitted at width {width} \
                                 (turn={has_turn})\n{header}",
                                field
                            );
                        }
                        // The other glyph must never appear at all — one active
                        // sort, one arrow.
                        let other = if arrow == '▼' { '▲' } else { '▼' };
                        assert!(
                            !header.contains(other),
                            "both arrows drawn at width {width} (turn={has_turn})\n{header}"
                        );
                    }
                }
            }
        }
    }

    /// Widening the terminal never removes a column. `break` in the admission
    /// loop is what buys this: skipping a group that does not fit and trying the
    /// next, narrower one would show Cache× at W and Duration instead at W+1.
    ///
    /// Note what this does **not** cover: it compares header *sets*, so it stays
    /// green through the Session sawtooth (Session is 40 cells at 149 and 20 at
    /// 150). That is accepted, but it means this test is not evidence that
    /// widening never costs the user anything.
    #[test]
    fn admitted_columns_are_monotonic_in_width() {
        for has_turn in [true, false] {
            let mut previous: Vec<SessionColumn> = Vec::new();
            for width in 80u16..=SWEEP_MAX {
                let mut s = fat_session();
                if !has_turn {
                    s.turn_count = 0;
                }
                let mut app = app_with(width, vec![s]);
                let header = header_line(&mut app, width);
                let current = header_columns(&header);
                for column in &previous {
                    assert!(
                        current.contains(column),
                        "{:?} disappeared going from {} to {width} columns (turn={has_turn})\n{header}",
                        column,
                        width - 1
                    );
                }
                previous = current;
            }
        }
    }

    /// Headers, cells and constraints are all derived from the same `chosen`
    /// vector, so this should be impossible to fail — which is the reason to
    /// assert it. If it can fail, they are not actually derived.
    #[test]
    fn header_cell_and_constraint_counts_agree() {
        for width in 80u16..=SWEEP_MAX {
            for has_turn in [true, false] {
                let ctx = wide_ctx(has_turn);
                let layout = admit_and_distribute(width - 2, &ctx);
                let constraints: Vec<Constraint> = layout
                    .chosen
                    .iter()
                    .map(|c| layout.constraint(*c, &ctx))
                    .collect();
                assert_eq!(constraints.len(), layout.chosen.len());

                let mut s = fat_session();
                if !has_turn {
                    s.turn_count = 0;
                }
                let mut app = app_with(width, vec![s]);
                let header = header_line(&mut app, width);
                assert_eq!(
                    header_columns(&header).len(),
                    layout.chosen.len(),
                    "rendered header does not match the admitted set at width {width} \
                     (turn={has_turn})\n{header}"
                );
            }
        }
    }

    /// `natural()` is sized for realistic values, and every one of these is
    /// unrealistic — a `first_active_ms` of 1, a session that paid for one
    /// input token and read 45M from cache, a six-digit message count. None of
    /// them is reachable through an honest session; all of them are reachable
    /// through a bad timestamp or a bad token count, and this file exists
    /// because of what ratatui does to a value that does not fit.
    ///
    /// Each line below names what the value rendered as before `fit()`: a
    /// number that looks like a number and is wrong by two to five orders of
    /// magnitude, which is #964 again in a column nobody was looking at.
    #[test]
    fn absurd_values_are_marked_rather_than_clipped_to_plausible_ones() {
        let width = 240;
        let mut s = fat_session();
        s.first_active_ms = 1; // "20092d 14h" in nine cells -> `20092d 14`
        s.tokens.input = 1;
        s.tokens.cache_write = 0;
        s.tokens.cache_read = 45_678_901; // "45678901.0x" in eight -> `45678901`
        s.message_count = 1_234_567; // in five -> `12345`
        s.turn_count = 999_999; // in five -> `99999`
        let mut app = app_with(width, vec![s]);
        let row = first_row_line(&mut app, width);
        for plausible in ["12345", "99999", "45678901", "20092d 14"] {
            assert!(
                !row.contains(plausible),
                "{plausible:?} is a clipped value passing for a whole one\n{row}"
            );
        }
        // What they render as instead, so this test fails if the marker moves.
        for marked in ["12...", "99...", "45678...", "20092d..."] {
            assert!(
                row.contains(marked),
                "expected {marked:?} in the row\n{row}"
            );
        }
    }

    // ---- the Client budget is a claim about the client registry ------------

    /// `natural(Client)` asserts something about `clients.rs`, so check it
    /// against `clients.rs` rather than against a fixture. Every name the
    /// registry can produce has to fit at its natural width, or the column that
    /// says which tool a session came from starts answering with a prefix.
    ///
    /// Adding a client with a longer display name fails here. The fix is to
    /// raise `natural(Client)` and move every threshold in the tables above by
    /// the same number of cells — not to lower the budget back down.
    #[test]
    fn client_column_fits_every_registered_client() {
        let budget = SessionColumn::Client.natural(&wide_ctx(true)) as usize;
        for client in ClientId::iter() {
            let name = get_compact_client_display_name(client.as_str());
            assert!(
                display_width(&name) <= budget,
                "{client:?} renders {name:?} ({} cells) in a {budget}-cell Client column",
                display_width(&name)
            );
        }
    }

    /// The concrete failure the budget above prevents: at 12 cells these rows
    /// could become byte-identical at every width, which is a misattribution
    /// and not a cosmetic clip.
    #[test]
    fn clients_sharing_a_prefix_render_distinguishably() {
        let width = 200;
        for (a, b) in [
            ("antigravity-cli", "antigravity"),
            ("opencodereview", "opencode"),
            ("devin-desktop", "devin"),
            ("openclaw", "opencode"),
        ] {
            let render_one = |client: &str| {
                let mut s = fat_session();
                s.client = client.to_string();
                let mut app = app_with(width, vec![s]);
                first_row_line(&mut app, width)
            };
            assert_ne!(
                render_one(a),
                render_one(b),
                "{a} and {b} render the same row at width {width}"
            );
        }
    }

    /// `TokscaleConfig` lets a user name a client anything, and an unrecognised
    /// id is echoed through verbatim, so the budget has to be enforced at the
    /// cell instead of trusted. Past it the name is marked, not silently cut.
    #[test]
    fn an_over_long_client_name_is_marked_truncated() {
        let width = 200;
        let mut s = fat_session();
        s.client = "a-client-with-a-very-long-name".to_string();
        let mut app = app_with(width, vec![s]);
        let row = first_row_line(&mut app, width);
        assert!(
            row.contains("a-client-wit..."),
            "an over-long client name must carry an ellipsis\n{row}"
        );
    }

    // ---- slack distribution ------------------------------------------------

    /// Session is the sole `Min` column, so ratatui hands it every cell that
    /// slack distribution did not give Model. `session_width` has to equal that
    /// or the title is clipped by the solver with no ellipsis — the same silent
    /// truncation this change exists to remove, one column over.
    #[test]
    fn session_cell_truncates_to_the_width_it_is_actually_given() {
        let title = "X".repeat(200);
        for width in 80u16..=SWEEP_MAX {
            for has_turn in [true, false] {
                let ctx = wide_ctx(has_turn);
                let layout = admit_and_distribute(width - 2, &ctx);
                let mut s = fat_session();
                s.title = Some(title.clone());
                if !has_turn {
                    s.turn_count = 0;
                }
                let mut app = app_with(width, vec![s]);
                let row = first_row_line(&mut app, width);
                let rendered = row.chars().filter(|c| *c == 'X').count();
                assert_eq!(
                    rendered as u16 + 3,
                    layout.session_width,
                    "Session rendered {rendered} title chars but the layout budgeted \
                     {} at width {width} (turn={has_turn})\n{row}",
                    layout.session_width
                );
                assert!(
                    row.contains("X..."),
                    "an over-long title must be marked as truncated at width {width}\n{row}"
                );
            }
        }
    }

    /// The same claim with a title whose graphemes are two cells wide. A
    /// code-point budget calls 28 Hangul syllables comfortable in a 34-cell
    /// column, returns them untouched, and ratatui clips them at 17 with no
    /// marker — the exact silent truncation this change exists to remove, in
    /// the one column that was still measuring in the wrong unit.
    #[test]
    fn a_full_width_title_is_budgeted_in_cells_not_code_points() {
        let title = "가".repeat(60);
        for width in 80u16..=SWEEP_MAX {
            for has_turn in [true, false] {
                let ctx = wide_ctx(has_turn);
                let layout = admit_and_distribute(width - 2, &ctx);
                let mut s = fat_session();
                s.title = Some(title.clone());
                if !has_turn {
                    s.turn_count = 0;
                }
                let mut app = app_with(width, vec![s]);
                let row = first_row_line(&mut app, width);
                // One `char` per grapheme, and ratatui blanks the second cell
                // of a wide one, so each syllable shows up as "가 ".
                let cells = row.chars().filter(|c| *c == '가').count() * 2;
                assert!(
                    cells + 3 <= layout.session_width as usize,
                    "title took {cells} cells plus an ellipsis out of a {} cell budget at width \
                     {width} (turn={has_turn})\n{row}",
                    layout.session_width
                );
                // The marker has to sit in the Session cell, before Client.
                let marker = row.find("...").unwrap_or(usize::MAX);
                let client = row.find("OpenCode").unwrap_or(0);
                assert!(
                    marker < client,
                    "an over-long full-width title must be marked as truncated at width \
                     {width} (turn={has_turn})\n{row}"
                );
            }
        }
    }

    /// `build_model_cell` measures in cells for the same reason the Session and
    /// Client cells do, and needs its own fixture because every other model in
    /// this file is ASCII. On a code-point budget a full-width name fills 17
    /// graphemes of an 18-cell column, and the "…" it appends is the first
    /// thing the solver cuts — a truncated name that does not look truncated.
    #[test]
    fn a_full_width_model_name_is_budgeted_in_cells_not_code_points() {
        for width in [120u16, 160, 220] {
            for has_turn in [true, false] {
                let ctx = wide_ctx(has_turn);
                let layout = admit_and_distribute(width - 2, &ctx);
                let mut s = fat_session();
                if !has_turn {
                    s.turn_count = 0;
                }
                s.models = vec![SessionModel {
                    display_name: "모".repeat(30),
                    provider: "test".to_string(),
                    color_key: "test-model".to_string(),
                }];
                let mut app = app_with(width, vec![s]);
                let row = first_row_line(&mut app, width);
                let cells = row.chars().filter(|c| *c == '모').count() * 2;
                // Strictly less than the budget: the name has to leave room for
                // the one ellipsis cell that marks what was dropped.
                assert!(
                    cells < layout.model_width as usize,
                    "model took {cells} cells, leaving no room for the ellipsis in a {} cell \
                     budget at width {width} (turn={has_turn})\n{row}",
                    layout.model_width
                );
                assert!(
                    row.contains('…'),
                    "an over-long full-width model name must be marked as truncated at width \
                     {width} (turn={has_turn})\n{row}"
                );
            }
        }
    }

    /// Decision B: the session title gets slack up to 40 cells before Model gets
    /// any, then Model grows to 36, then the rest lands back on Session.
    #[test]
    fn slack_goes_to_the_session_title_before_the_model_column() {
        for (width, has_turn, session_width, model_width) in [
            (120u16, true, 31, 18), // Model admitted, all slack to the title
            (152, true, 40, 19),    // title comfortable, Model starts growing
            (153, true, 20, 18),    // Cache R/W admitted, slack reclaimed
            (199, true, 36, 18),
            // The width the model-truncation tests pin: without turn data every
            // group is admitted at 178, so 202 leaves 24 cells of slack — 20 to
            // the title, 4 to Model, which lands it on exactly 22.
            (202, false, 40, 22),
            (260, true, 79, 36), // Model capped, remainder back to the title
        ] {
            let ctx = wide_ctx(has_turn);
            let layout = admit_and_distribute(width - 2, &ctx);
            assert_eq!(
                (layout.session_width, layout.model_width),
                (session_width, model_width),
                "slack split wrong at width {width} (turn={has_turn})"
            );
        }
    }

    // ---- narrow and very narrow must not move ------------------------------

    /// The narrow branches keep their percentage constraints, their hardcoded
    /// sort indices and their `usize::MAX` sentinel. These lines are byte-for-byte
    /// what the pre-budget implementation rendered.
    #[test]
    fn narrow_layouts_render_exactly_as_before() {
        for (width, has_turn, field, expected_header, expected_row) in [
            (
                59u16,
                true,
                SortField::Cost,
                "│Session                           Cost ▼                 │",
                "│A fairly long ses...              $12.35                 │",
            ),
            (
                59,
                true,
                SortField::Date,
                "│Session                           Cost                   │",
                "│A fairly long ses...              $12.35                 │",
            ),
            (
                60,
                true,
                SortField::Cost,
                "│Session     Client     Turn   Msgs    Tokens     Cost ▼   │",
                "│A fairly lo OpenCode   137    428     49.5M      $12.35   │",
            ),
            (
                60,
                false,
                SortField::Tokens,
                "│Session         Client     Msgs    Tokens ▼   Cost        │",
                "│A fairly long s OpenCode   428     49.5M      $12.35      │",
            ),
            (
                79,
                true,
                SortField::Tokens,
                "│Session           Client       Turn      Msgs      Tokens ▼      Cost        │",
                "│A fairly long ses OpenCode     137       428       49.5M         $12.35      │",
            ),
            (
                79,
                false,
                SortField::Date,
                "│Session               Client         Msgs      Tokens         Cost           │",
                "│A fairly long session OpenCode       428       49.5M          $12.35         │",
            ),
        ] {
            let mut s = fat_session();
            s.session_id = "ses_09d2eac0-long-id".to_string();
            s.title = Some("A fairly long session title here".to_string());
            if !has_turn {
                s.turn_count = 0;
            }
            let mut app = app_with(width, vec![s]);
            app.sort_field = field;
            let body = render_body(&mut app, width, 6);
            let mut lines = body.lines().skip(1);
            assert_eq!(
                lines.next().unwrap_or_default(),
                expected_header,
                "narrow header moved at width {width} (turn={has_turn}, sort={field:?})"
            );
            assert_eq!(
                lines.next().unwrap_or_default(),
                expected_row,
                "narrow row moved at width {width} (turn={has_turn}, sort={field:?})"
            );
        }
    }

    /// Neither narrow layout ever grows a wide-only column, at any width below
    /// the 80-column entry point.
    #[test]
    fn narrow_layouts_never_show_wide_only_columns() {
        for width in 20u16..80 {
            for has_turn in [true, false] {
                let mut s = fat_session();
                if !has_turn {
                    s.turn_count = 0;
                }
                let mut app = app_with(width, vec![s]);
                let header = header_line(&mut app, width);
                for label in [
                    "Model",
                    "Input",
                    "Output",
                    "Cache R",
                    "Cache W",
                    "Cache×",
                    "Total",
                    "Cost/1M",
                    "Duration",
                    "Last Active",
                ] {
                    assert!(
                        !header.contains(label),
                        "wide-only header {label} leaked into the narrow layout at width \
                         {width} (turn={has_turn})\n{header}"
                    );
                }
                if width >= 60 {
                    assert!(
                        header.contains("Tokens"),
                        "expected narrow Tokens\n{header}"
                    );
                    assert_eq!(
                        header.contains("Turn "),
                        has_turn,
                        "narrow Turn gating wrong at width {width}\n{header}"
                    );
                } else {
                    assert!(
                        !header.contains("Client"),
                        "very narrow keeps two columns\n{header}"
                    );
                }
            }
        }
    }

    // ---- behavioral tests that predate the budget --------------------------

    #[test]
    fn wide_terminal_renders_session_and_duration_columns() {
        let mut app = app_with(
            200,
            vec![
                session("abc-123", "opencode", 1.5, 1_736_000_000_000),
                session("def-456", "claude", 0.25, 1_736_000_000_000),
            ],
        );
        let body = render_body(&mut app, 200, 12);
        assert!(body.contains("abc-123"), "expected session id\n{body}");
        assert!(
            body.contains("Duration"),
            "expected duration header\n{body}"
        );
        assert!(
            body.contains("Last Active"),
            "expected last active header\n{body}"
        );
    }

    #[test]
    fn model_column_shows_model_names() {
        let mut s = session("abc-123", "opencode", 1.5, 1_736_000_000_000);
        s.models = vec![SessionModel {
            display_name: "claude-sonnet-4".to_string(),
            provider: "anthropic".to_string(),
            color_key: "claude-sonnet-4".to_string(),
        }];
        let mut app = app_with(200, vec![s]);
        let body = render_body(&mut app, 200, 12);
        assert!(body.contains("Model"), "expected Model header\n{body}");
        assert!(
            body.contains("claude-sonnet-4"),
            "expected model name in body\n{body}"
        );
    }

    #[test]
    fn model_column_shows_multiple_models() {
        let mut s = session("abc-123", "opencode", 1.5, 1_736_000_000_000);
        s.models = vec![
            SessionModel {
                display_name: "gpt-4o".to_string(),
                provider: "openai".to_string(),
                color_key: "gpt-4o".to_string(),
            },
            SessionModel {
                display_name: "o3-mini".to_string(),
                provider: "openai".to_string(),
                color_key: "o3-mini".to_string(),
            },
        ];
        let mut app = app_with(220, vec![s]);
        let body = render_body(&mut app, 220, 12);
        assert!(
            body.contains("gpt-4o") && body.contains("o3-mini"),
            "expected both model names in body\n{body}"
        );
    }

    #[test]
    fn session_title_displayed_when_available() {
        let mut s = session("ses_09d2eac0", "opencode", 1.5, 1_736_000_000_000);
        s.title = Some("This is a long title for a session that could be used".to_string());
        let mut app = app_with(200, vec![s]);
        let body = render_body(&mut app, 200, 12);
        assert!(
            body.contains("This is a long"),
            "expected session title in body\n{body}"
        );
    }

    /// The Session column is the sole sink for leftover cells, so a wide
    /// terminal shows the whole title. What it truncates to is
    /// `WideLayout::session_width`, not a hardcoded 60.
    #[test]
    fn session_column_expands_on_wide_terminal() {
        let title = "This is a long title for a session that could be used";
        let mut s = session("ses_09d2eac0", "opencode", 1.5, 1_736_000_000_000);
        s.title = Some(title.to_string());
        let mut app = app_with(260, vec![s]);
        let body = render_body(&mut app, 260, 12);
        assert!(
            body.contains(title),
            "expected full title on wide terminal\n{body}"
        );
        let layout = admit_and_distribute(258, &wide_ctx(true));
        assert!(
            layout.session_width as usize >= title.chars().count(),
            "the layout must budget at least the title it renders, got {}",
            layout.session_width
        );
    }

    #[test]
    fn narrow_terminal_drops_token_breakdown_columns() {
        let mut app = app_with(
            70,
            vec![session("abc-123", "opencode", 1.5, 1_736_000_000_000)],
        );
        let body = render_body(&mut app, 70, 12);
        assert!(body.contains("abc-123"), "expected session id\n{body}");
        assert!(
            !body.contains("Cache×"),
            "cache hit rate should be dropped in narrow mode\n{body}"
        );
    }

    #[test]
    fn empty_sessions_shows_refresh_message() {
        let mut app = make_app(120);
        let body = render_body(&mut app, 120, 12);
        assert!(
            body.contains("No session usage data"),
            "expected empty message\n{body}"
        );
    }

    #[test]
    fn tokens_and_date_sort_indicators_land_on_their_columns() {
        for has_turn in [true, false] {
            let mut s = session("abc-123", "opencode", 1.5, 1_736_000_000_000);
            if !has_turn {
                s.turn_count = 0;
            }

            let mut app = app_with(200, vec![s.clone()]);
            app.sort_field = SortField::Tokens;
            let header = header_line(&mut app, 200);
            assert!(
                header.contains("Total ▼"),
                "tokens sort indicator misplaced (turn={has_turn})\n{header}"
            );

            let mut app = app_with(200, vec![s]);
            app.sort_field = SortField::Date;
            let header = header_line(&mut app, 200);
            assert!(
                header.contains("Last Active ▼"),
                "date sort indicator misplaced (turn={has_turn})\n{header}"
            );
        }
    }

    /// Terminal width at which the Model column is exactly 22 cells with no turn
    /// data: the 21-char first name plus the reserved marker cell, with nothing
    /// left for the ", " a second name would need. Under decision B Session
    /// takes the first 20 cells of slack, so this sits 20 columns above where
    /// the Model-first rule put it. Pinned by `slack_goes_to_the_session_title_*`.
    const MODEL_WIDTH_22_TERMINAL: u16 = 202;

    /// A session that switched models must never render as a single-model
    /// session. When the next name cannot fit, a trailing "…" marks that models
    /// were dropped instead of them vanishing silently.
    #[test]
    fn multi_model_cell_marks_models_that_did_not_fit() {
        let width = MODEL_WIDTH_22_TERMINAL;
        let mut s = session("abc-123", "opencode", 1.5, 1_736_000_000_000);
        s.turn_count = 0;
        s.models = vec![
            SessionModel {
                display_name: "gemini-2-5-flash-lite".to_string(),
                provider: "google".to_string(),
                color_key: "gemini-2-5-flash-lite".to_string(),
            },
            SessionModel {
                display_name: "claude-sonnet-4-5".to_string(),
                provider: "anthropic".to_string(),
                color_key: "claude-sonnet-4-5".to_string(),
            },
        ];
        let mut app = app_with(width, vec![s]);
        let body = render_body(&mut app, width, 12);
        assert!(
            body.contains("gemini-2-5-flash-lite…"),
            "expected a truncation marker after the model that did fit\n{body}"
        );
        assert!(
            !body.contains("claude"),
            "second model was not supposed to fit at width {width}\n{body}"
        );
    }

    /// A single model that overflows carries its own ellipsis, so the
    /// dropped-model marker must not double up on it.
    #[test]
    fn single_model_cell_truncates_without_a_second_ellipsis() {
        let width = MODEL_WIDTH_22_TERMINAL;
        let mut s = session("abc-123", "opencode", 1.5, 1_736_000_000_000);
        s.turn_count = 0;
        s.models = vec![SessionModel {
            display_name: "a-very-long-model-name-that-overflows".to_string(),
            provider: "anthropic".to_string(),
            color_key: "a-very-long-model-name-that-overflows".to_string(),
        }];
        let mut app = app_with(width, vec![s]);
        let body = render_body(&mut app, width, 12);
        assert!(
            body.contains("a-very-long-model-nam…"),
            "expected a single trailing ellipsis on the truncated name\n{body}"
        );
        assert!(
            !body.contains("……"),
            "truncated name must not pick up a second ellipsis\n{body}"
        );
    }

    #[test]
    fn format_duration_handles_days_hours_minutes_seconds() {
        assert_eq!(format_duration(0, 1000), "—");
        assert_eq!(format_duration(1000, 500), "—");
        assert_eq!(format_duration(1000, 1000), "0s");
        assert_eq!(format_duration(0, 0), "—");
        assert_eq!(format_duration(1000, 2000), "1s");
        assert_eq!(format_duration(1000, 61_000), "1m");
        assert_eq!(format_duration(1000, 3_661_000), "1h 1m");
        assert_eq!(format_duration(1000, 90_061_000), "1d 1h");
    }

    #[test]
    fn format_duration_boundary_units() {
        // exactly one hour without extra minutes (use a real first_ms since
        // first_ms == 0 short-circuits to "—")
        assert_eq!(format_duration(1_000, 3_601_000), "1h 0m");
        // exactly one day
        assert_eq!(format_duration(1_000, 86_401_000), "1d 0h");
    }
}
