# Sessions tab: width-budgeted columns

Design proposal for [#964](https://github.com/junhoyeo/tokscale/issues/964). The
wide Sessions layout asks for 161 terminal columns and turns on at 80. This
document measures the gap, proposes replacing the hand-maintained column vectors
with a single priority-ordered descriptor list, and asks the maintainer to
settle one product question — the priority order itself.

Status: **proposal**. Nothing here is implemented. Accept, reject, or amend.

Everything numeric below was computed or rendered against `226dfc75`
(`feat(tui): show model names with family-shade colors in Sessions tab (#956)`);
each section says how to re-derive it.

## The problem

`crates/tokscale-cli/src/tui/ui/sessions.rs` renders three layouts, selected by
terminal width:

| layout | condition | source |
|---|---|---|
| very narrow | `app.is_very_narrow()` — width < 60 | `Session`, `Cost` |
| narrow | `app.is_narrow()` — width < 80 | `Session`, `Client`, `[Turn]`, `Msgs`, `Tokens`, `Cost` |
| wide | everything else — width ≥ 80 | 13–15 columns |

The wide layout's constraint vector, summed at natural width:

```
Session 20 (Min) + Client 12 + Turn 5 + Msgs 5
  + Input 10 + Output 10 + Cache R 10 + Cache W 10 + Cache× 8
  + Total 10 + Cost 10 + Cost/1M 10 + Duration 9 + Last Active 17  = 146
  + 13 inter-column separators (ratatui `column_spacing`, default 1)  = 159
  + 2 block borders                                                  = 161
```

| layout | cells | terminal columns needed |
|---|---|---|
| wide, turn data, no Model | 159 | **161** |
| wide, turn data, with Model (18) | 178 | **180** |
| wide, no turn data, no Model | 154 | **156** |
| wide, no turn data, with Model (18) | 173 | **175** |
| **actually activates at** | | **80** |

So across the entire 80–160 band the ratatui solver has to shrink every column
to fit. That band covers essentially every terminal anyone uses.

### How to re-derive

The numbers live in two places that must already agree with each other:

- the `widths` vector in the wide branch of `render` (`sessions.rs:353-376`)
- `wide_layout_fixed_width` (`sessions.rs:30-41`), which hardcodes the same sum

Sum the `Constraint::Length` values, add `n - 1` for separators, add 2 for the
block borders. ratatui 0.29's `Table` lays out with
`Layout::horizontal(widths).spacing(column_spacing)`, so the separator count is
exactly one fewer than the column count.

To see what actually renders, add a temporary test next to the existing ones in
`sessions.rs` — the module's `render_body` / `header_line` helpers already do
the work:

```rust
#[test]
fn probe() {
    for width in [80u16, 100, 120, 161, 180] {
        let mut app = make_app(width);
        app.data.sessions = vec![session("abc-123", "opencode", 1.5, 1_736_000_000_000)];
        println!("w={width} |{}|", header_line(&mut app, width));
    }
}
```

then `cargo test -p tokscale-cli --bins probe -- --nocapture`.

### What that actually looks like

Rendered header and one data row, turn data present, sorted by Cost. Session
`my-session`, 1,234,567 input / 234,567 output / 45,678,901 cache read /
2,345,678 cache write, $12.3456, 428 messages, 137 turns, 1h span:

```
w=80  │Session              Cli Turn Msg Inpu Out Cach Cac Cach Tot Cost Cos Dura Las│
      │my-session           Ope 137  428 1.2M 234 45.7 2.3 12.8 49. $12. $0. 1h 0 202│

w=100 │Session              Clien Turn  Msgs  Input Outpu Cache Cache Cache Total Cost  Cost/ Durat Last │
      │my-session           OpenC 137   428   1.2M  234K  45.7M 2.3M  12.8x 49.5M $12.3 $0.25 1h 0m 2025-│

w=120 │Session              Client       Turn  Msgs  Input  Output  Cache  Cache  Cache×  Total  Cost ▼ Cost/1 Duratio Last A│
      │my-session           OpenCode     137   428   1.2M   234K    45.7M  2.3M   12.8x   49.5M  $12.35 $0.25  1h 0m   2025-0│

w=161 │Session              Client       Turn  Msgs  Input      Output     Cache R    Cache W    Cache×   Total      Cost ▼     Cost/1M    Duration  Last Active      │
      │my-session           OpenCode     137   428   1.2M       234K       45.7M      2.3M       12.8x    49.5M      $12.35     $0.25      1h 0m     2025-01-04 23:13 │
```

Three things are worse than "headers degrade to stubs".

**Values are silently truncated into plausible wrong numbers.** ratatui clips
cell content to the cell width with no ellipsis. At 80 columns the Output cell
reads `234` for 234,567 tokens — the `K` suffix is gone, and `234` is a
perfectly valid token count. Total reads `49.` and Cost reads `$12.` At 100
columns Cost reads `$12.3` for `$12.35`. Nothing marks these as truncated. This
is a correctness problem in a tab whose entire purpose is reporting numbers,
and it is the strongest argument for the change; the header stubs are only the
visible half of it.

**The sort indicator is missing for most of the range.** Measured by sweeping
80–200 and asserting the rendered header contains `"<label> ▼"`:

| indicator | turn data | first legible | clipped again at |
|---|---|---|---|
| `Cost ▼` | yes | 111 | 113 |
| `Cost ▼` | no | 96 | 97, 98, 101 |
| `Total ▼` | yes | 123 | 124, 126 |
| `Total ▼` | no | 118 | 119, 121 |
| `Last Active ▼` | yes | 157 | — |
| `Last Active ▼` | no | 152 | — |

Sorting by Date is a first-class, key-bound sort whose arrow is invisible below
157 columns. A user who presses the sort key sees the row order change with no
feedback about which column drives it.

**It is not monotonic.** `Cost ▼` is legible at 111 and 112, gone at 113, back
at 114. That is the ratatui solver's remainder distribution, and it is why no
hand-picked constant can work here: widening your terminal by one column can
remove information.

The equivalent numbers for full header labels:

| label | first renders in full |
|---|---|
| `Client` | 101 |
| `Cache×` | 108 |
| `Cache R` | 119 |
| `Cost/1M` | 119 |
| `Cache W` | 121 |
| `Duration` | 130 |
| `Last Active` | 155 |

None of this is new. It predates #956 by a long way; #956 only made someone
count.

## The Model gate is not the bug

#956 added a Model column and gated it on `wide_layout_fixed_width`, so it
appears only once every other column has its natural width. That gate is
correct: it is derived rather than guessed, and whenever Model shows, all three
sort indicators are structurally legible. An earlier hand-picked "about 120
columns" gate was worse than useless — at 120 it *removed* the `Cost ▼`
indicator that 119 had, which is exactly the non-monotonicity in the table
above.

The gate's premise is "all ten metric columns render at natural width". The
measurement above says that state begins at 161 columns, which is why the
derived answer landed at 180. The gate is a correct response to a broken
budget. Fix the budget and the gate stops being a special case.

## Where the numbers live today

The issue names four places that must stay in sync. There are five:

| # | what | location |
|---|---|---|
| 1 | `header_cells` vector | `sessions.rs:114-136` |
| 2 | pushed row cells | `sessions.rs:270-320` |
| 3 | `widths` / `Constraint` vector | `sessions.rs:353-376` |
| 4 | sort-indicator indices `8/9/12 + optional_cols` | `sessions.rs:151-181` |
| 5 | `wide_layout_fixed_width`'s hardcoded sum | `sessions.rs:30-41` |

(1)–(3) desyncing misaligns the table: a header over the wrong data. (4)
desyncing puts the sort arrow on the wrong column. (5) desyncing moves the
Model gate to a width that no longer means what it says. None of the five is
checked by the compiler. `has_turn_data` and `show_model` each have to be
threaded through all five by hand, correctly, in the same commit — which #956
did, and which was the single riskiest part of that PR.

This is a house pattern, not a one-off. `daily.rs:70` carries
`let full_layout_width: u16 = if has_turn_data { 112 } else { 105 };` under a
comment reading "keep it in sync with the `widths` block below". Both constants
happen to be correct today. Nothing keeps them that way.

## Proposed design

One descriptor list. Everything else derives from it.

### Column descriptors

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionColumn {
    Session, Client, Model, Turn, Msgs,
    Input, Output, CacheRead, CacheWrite, CacheHit,
    Total, Cost, CostPerMillion, Duration, LastActive,
}

/// Resolved once per render, before any column is built.
struct WideCtx {
    has_turn: bool,
    has_model: bool,
    model_width: u16,
    session_width: u16,
    last_active_fmt: &'static str,
    // theme-derived styles, extracted from `App` the way `render` already does
}

impl SessionColumn {
    fn header(self) -> &'static str { /* match, no wildcard arm */ }
    fn natural(self, ctx: &WideCtx) -> u16 { /* match, no wildcard arm */ }
    fn constraint(self, ctx: &WideCtx) -> Constraint { /* Session => Min, else Length */ }
    fn sort_field(self) -> Option<SortField> { /* Total/Cost/LastActive => Some */ }
    fn available(self, ctx: &WideCtx) -> bool { /* Turn => ctx.has_turn, ... */ }
    fn cell(self, s: &SessionUsage, app: &App, ctx: &WideCtx) -> Cell<'static> { /* match */ }
}
```

The load-bearing rule: **every one of those methods is an exhaustive `match`
with no `_ =>` arm.** Adding a variant then fails to compile until the header,
the width, the constraint, the sort field, the availability predicate and the
cell builder all exist. That is the mechanism that replaces the current
comment-and-hope discipline, and it is the main reason to prefer an enum over,
say, a vector of closures.

Two orderings, deliberately separate:

```rust
/// Left-to-right display order. Cosmetic; changing it never changes what fits.
const WIDE_ORDER: [SessionColumn; 15] = [
    Session, Client, Model, Turn, Msgs,
    Input, Output, CacheRead, CacheWrite, CacheHit,
    Total, Cost, CostPerMillion, Duration, LastActive,
];

/// Admission order: earlier groups are admitted first and dropped last.
/// Each group is all-or-nothing. This array is the product decision below.
const WIDE_PRIORITY: [&[SessionColumn]; 12] = [
    &[Session], &[Client], &[Total], &[Cost], &[Msgs], &[Turn],   // core
    &[LastActive],
    &[Model],
    &[Input, Output],
    &[CacheRead, CacheWrite],
    &[Duration],
    &[CacheHit],
    &[CostPerMillion],
];
```

Grouping matters. `Input` without `Output` is worse than neither, because a
reader will take Input for a total. Same for `Cache R` without `Cache W`.
Admitting groups atomically encodes that.

### The budget

```
available     = inner.width                       // block borders already removed
required(set) = Σ natural(c) + (|set| - 1)        // ratatui column_spacing == 1
```

Measure against `inner`, not `area` or `app.terminal_width`. #956 established
this and it matters: the budget has to equal the width ratatui actually divides
up, or `build_model_cell`'s ellipsis budget stops matching the rendered cell —
which was a real bug there (`a7fba35e`), where multi-model sessions silently
rendered as single-model.

### Admission

```rust
fn admit(available: u16, ctx: &WideCtx) -> Vec<SessionColumn> {
    let mut chosen: Vec<SessionColumn> = Vec::new();
    for group in WIDE_PRIORITY {
        let g: Vec<_> = group.iter().copied().filter(|c| c.available(ctx)).collect();
        if g.is_empty() {
            continue;               // e.g. Turn with no turn data
        }
        let mut next = chosen.clone();
        next.extend(&g);
        if required(&next, ctx) <= available {
            chosen = next;
        } else {
            break;                  // stop at the first group that does not fit
        }
    }
    chosen.sort_by_key(|c| WIDE_ORDER.iter().position(|o| o == c).unwrap());
    chosen
}
```

**`break`, not `continue`.** Skipping a group that does not fit and trying the
next (narrower) one packs more columns in, but makes the admitted set
non-monotonic in width: at width *W* you would see Cache×, at *W+1* Duration
instead and Cache× gone. That is the same disorienting behavior the ratatui
solver already produces, and reproducing it deliberately would be worse.
Breaking gives a property worth having and worth asserting in a test:

> The set of columns shown at width *W* is a subset of the set shown at *W+1*,
> for every *W*. Widening a terminal never removes information.

The slack left by breaking early is not wasted. Session is `Constraint::Min`,
so ratatui hands every leftover cell to it — the slack becomes session-title
width, which is a good use for it.

### Slack distribution

After admission, `slack = available - required(chosen)`. Two claimants, Session
and Model, and they need an order. #956's rule is Model-first (up to
`MODEL_COLUMN_MAX_CHARS`, remainder to Session).

Recommend flipping it to **Session-first up to a comfortable 40, then Model up
to 36, then Session takes the rest**:

```rust
let session_extra = slack.min(SESSION_COMFORTABLE - SESSION_MIN);        // 40 - 20
let model_extra   = (slack - session_extra).min(MODEL_MAX - MODEL_MIN);  // 36 - 18
// whatever remains lands on Session automatically via Constraint::Min
```

Rationale: model display names are short — `gpt-4o` is 6, `claude-sonnet-4` 15,
`gemini-2-5-flash-lite` 21 — so Model's 18-cell minimum already renders most
single-model sessions in full, and its growth to 36 only buys anything for
multi-model sessions. Session titles are routinely 40–80 characters. In the
100–130 band, where slack is 10–20 cells, those cells are worth much more to
the title. This is a knob, not a principle; it is listed as decision B below.

Model's width must be resolved before cells are built, so `ctx.model_width` is
computed in this step and read by `SessionColumn::Model::cell`.

### The five derivations

```rust
let chosen  = admit(inner.width, &ctx);
let headers = chosen.iter().map(|c| c.header());
let widths  = chosen.iter().map(|c| c.constraint(&ctx));
let cells   = chosen.iter().map(|c| c.cell(session, app, &ctx));   // per row
let arrow   = |f: SortField| chosen.iter().position(|c| c.sort_field() == Some(f));
```

Sync point (4) stops being arithmetic. `position()` cannot land on the wrong
column, and when the sorted column is not admitted it returns `None` and no
arrow is drawn — which is what `usize::MAX` currently fakes for the narrow
branches. Sync point (5), `wide_layout_fixed_width`, is deleted: `required()`
computes the same thing from the descriptor list instead of from a second copy
of the numbers.

## Decision A: the priority order

This is the product call. It is not arithmetic and it should not be an
accident of array position.

### First, a correction to the framing

The issue asks which of Cache×, Cost/1M, Duration or Cache R/W goes first at
100 columns. Under any order, the honest answer at 100 columns is *all of
them*, plus Input/Output, plus Model. The core six columns (Session 20, Client
12, Turn 5, Msgs 5, Total 10, Cost 10) already need 69 terminal columns, and
`Last Active` alone adds 18 more. A hundred columns holds seven columns, not
thirteen. The real question is the order of the whole tail, and the widths at
which each tier appears.

### The existing narrow layout is an anchor

The shipped narrow layout — `Session`, `Client`, `[Turn]`, `Msgs`, `Tokens`,
`Cost` — is already a considered priority judgment, and very narrow reduces it
to `Session`, `Cost`. Any wide-layout order should be continuous with it, or
crossing 80 columns changes the column set discontinuously. Putting exactly
those six in the core tier means the 79 → 80 transition shows the same columns
and only widens them, which is a nicer property than today's jump from six
columns to fourteen stubs.

(Cosmetic mismatch worth noting: the narrow layout labels the total column
`Tokens`, wide labels it `Total`. Aligning them is a one-word change and is not
part of this proposal.)

### Recommendation

Rank the tail by **whether the column helps you find and compare sessions in a
list**, and treat "recomputable from columns ranked above it" as a tiebreak,
not as the primary rule.

| tier | group | why here |
|---|---|---|
| 1 | `Last Active` | The only temporal anchor in a list of sessions, and a sort target. Expensive (17) but a session list with no timestamps is hard to navigate. |
| 2 | `Model` | Explains cost. Highest information-per-cell of anything remaining, and the reason #964 exists. |
| 3 | `Input` + `Output` | The fundamental token split. Not recoverable from anything else shown. |
| 4 | `Cache R` + `Cache W` | Real information, and dominant for Claude-family usage — but a second-order breakdown. |
| 5 | `Duration` | Not recoverable from displayed columns, but weak: wall-clock span between first and last message, which for a resumed session is mostly noise. |
| 6 | `Cache×` | A rate. Diagnostic rather than navigational — you read it after picking a session, not to pick one. Its three inputs are all ranked above it. |
| 7 | `Cost/1M` | Same argument, one tier weaker: both of its inputs (Cost, Total) are in the core tier and always present. |

Cache× and Cost/1M rank last not merely because they are derived — neither
division is one you do in your head from `$12.35` and `49.5M` — but because
they answer a *diagnostic* question ("was this session efficient?") rather than
a *navigational* one ("which session was this, and what did it cost?"), and the
Sessions tab is a navigational list. Cost/1M is last of the two because its
inputs are guaranteed present at every width.

Duration sits above them because it is not recoverable at all, and below the
token columns because it is the least decision-relevant number in the table.

### Resulting thresholds

Terminal width at which each group first appears:

| admitted | with turn data | without turn data |
|---|---|---|
| core (Session, Client, [Turn], Msgs, Total, Cost) | 69 | 64 |
| + `Last Active` | 87 | 82 |
| + `Model` | **106** | **101** |
| + `Input`, `Output` | 128 | 123 |
| + `Cache R`, `Cache W` | 150 | 145 |
| + `Duration` | 160 | 155 |
| + `Cache×` | 169 | 164 |
| + `Cost/1M` | 180 | 175 |

The last row reproduces #956's derived gate exactly (180 / 175), which is the
arithmetic check that the descriptor widths match the constraint vector they
replace. Model moves from 180 to **106**.

### Alternatives considered

**Order B — rank by cost in cells**, cheapest columns first, to maximize column
count. Rejected: it optimizes for how many columns fit rather than what they
tell you, and it would rank Cache× (8) and Duration (9) above Model (18) and
Last Active (17) purely for being small.

**Order C — token accounting first**: `Input`/`Output`, `Cache R`/`Cache W`,
then `Model`, then `Last Active`. This is the "Sessions is a token-accounting
view" position, and it is defensible. Thresholds with turn data would be 91,
113, 132, 150. Rejected because `Last Active` disappearing until 150 leaves the
list with no timestamp and leaves `SortField::Date`'s arrow nowhere to live for
most of the range.

**Order D — abbreviate instead of dropping**: keep all fifteen columns and
shorten labels (`Cache R` → `CR`). Rejected: it fixes only the header half of
the problem. At 100 columns, fifteen columns share ~84 cells — six each — and
`$999.99` does not fit in six no matter what the header says. The truncated
*values* are the real defect, and abbreviation leaves them exactly as wrong.

**Order E — horizontal scrolling.** Rejected as scope: ratatui's `Table` has no
column scrolling, it needs new key bindings that collide with the existing sort
and navigation keys, and it makes the visible column set depend on hidden
state.

**Order F — hide columns that are all zero across visible rows.** Tempting (a
client that never caches shows two columns of zeros) and explicitly deferred:
the admitted set would then change as you scroll, which breaks the monotonicity
property above in a much more jarring way than width ever could.

## Decision B: slack distribution

Secondary, and independent of A.

| option | at 120 columns | at 260 columns |
|---|---|---|
| B1 — Model first (today's rule) | Model 32, Session 20 | Model 36, Session 82 |
| B2 — Session to 40 first, then Model (**recommended**) | Model 18, Session 34 | Model 36, Session 82 |

They differ only in the 106–200 band. B2 is recommended for the reason above:
an 18-cell Model column already renders most model names in full, and the
marginal cell is worth more to the session title.

B2's cost, stated plainly: Model stops growing until Session has 40 cells, so a
multi-model session with two long names stays truncated to 20 columns wider
than it does today. It also moves the width at which Model is exactly 22 cells
from 179 to 199, which is the pin the two `build_model_cell` truncation tests
rely on.

## What changes for users

Turn data present, recommended order, B2 slack:

| terminal width | today | proposed |
|---|---|---|
| **80** | 14 columns, every header a stub (`Cli`, `Inpu`, `Cach`, `Tot`, `Cos`, `Las`), Output renders `234` for 234K, Cost renders `$12.`, no sort arrow | 6 columns — Session (31), Client, Turn, Msgs, Total, Cost — all headers and values complete, `Cost ▼` legible. Same column set as narrow at 79. |
| **100** | 14 columns, `Client`/`Cache R`/`Duration`/`Last Active` still stubs, Cost renders `$12.3`, no sort arrow | 7 columns — core + Last Active (full `2025-01-04 23:13`), Session 33 |
| **120** | 14 columns, `Cost ▼` legible, `Cost/1`, `Duratio`, `Last A` stubs, Last Active shows `2025-0` | 8 columns — core + Last Active + **Model**, Session 34 |
| **161** | 14 columns, everything legible for the first time, no Model | 13 columns — everything except Cache× and Cost/1M, **plus Model**, Session 21 |
| **180** | 15 columns, everything legible, Model appears here for the first time | 15 columns, identical to today |

Read that table honestly: below 161 this trades breadth for correctness. A user
at 120 columns loses six columns they can currently see something in. What they
see is a stub header over a value that may be silently truncated — but
`format_tokens` is compact enough that many of those values do read correctly
today, and someone who has learned to recognize the stubs will experience this
as a regression. That is the real cost of the proposal, and the priority order
is the lever for arguing about it.

At 161 the trade is unambiguous: Model, in exchange for Cache× and Cost/1M,
both of which are recomputable by eye from columns that are still on screen.

## Migration and compatibility

### Constants and functions

| symbol | disposition |
|---|---|
| `MODEL_COLUMN_MIN_CHARS` (18) | Survives as `SessionColumn::Model::natural()`. Same value, no longer a gate. |
| `MODEL_COLUMN_MAX_CHARS` (36) | Survives as the slack cap in decision B. |
| `wide_layout_fixed_width` | **Deleted.** It exists only to hold a second copy of the constraint sum; `required(&chosen, &ctx)` computes it from the descriptors. |
| `show_model` / `model_budget` | Deleted. Replaced by `chosen.contains(&Model)` and the admission loop. |
| `optional_cols`, `last_active_idx`, `total_idx`, `cost_idx` | **Deleted.** Replaced by `chosen.iter().position(...)`. |
| `has_turn_data` | Kept as-is, becomes `WideCtx::has_turn` and feeds `Turn::available()`. |

### `has_model_data`

Today there is no data-availability gate on Model: a dataset where every
session has an empty `models` renders a column of em-dashes. Adding
`has_model_data` symmetric with `has_turn_data` would free 19 cells for real
data. This is a small, separable behavior change — recommended, but call it out
in review rather than smuggling it in.

### Existing tests

| test | fate |
|---|---|
| `wide_terminal_renders_session_and_duration_columns` (200) | Passes unchanged — Duration is admitted from 160. |
| `model_column_shows_model_names` (200) | Passes unchanged. |
| `model_column_shows_multiple_models` (220) | Passes unchanged; Model reaches 36 at 220 under B2. |
| `session_title_displayed_when_available` (200) | Passes unchanged. |
| `session_column_expands_on_wide_terminal` (260) | Passes; Session gets 82 cells against the 54-char title asserted. |
| `narrow_terminal_drops_token_breakdown_columns` (70) | Untouched — narrow branch. |
| `empty_sessions_shows_refresh_message` (120) | Untouched — early return. |
| `model_column_hidden_below_its_width_threshold` | **Rewrite.** Its premise inverts: at 179 Model is now on and Cost/1M is off. |
| `model_column_appears_at_its_width_threshold` | **Rewrite** as the threshold table below. |
| `model_column_gate_keeps_the_rest_of_the_header_legible` | **Generalize** from the gate width to a full sweep — this assertion becomes the primary invariant. |
| `cost_sort_indicator_tracks_model_and_turn_gating` | **Generalize** from six hand-picked widths to the sweep. |
| `tokens_and_date_sort_indicators_land_on_their_columns` | Keep; subsumed by the sweep but cheap. |
| `multi_model_cell_marks_models_that_did_not_fit` | **Adjust, and the width moves.** It derives its width as `wide_layout_fixed_width(false) + 1 + 22 + 2` = 179 to pin Model at exactly 22 cells. With `wide_layout_fixed_width` deleted the width must be a literal. Under B1 it stays 179; under B2 it becomes **199** (no turn data: all groups admitted at 175, required 173, so slack = width − 175; Session takes the first 20, Model needs 4 more → slack 24). |
| `single_model_cell_truncates_without_a_second_ellipsis` | Same adjustment, same width. |
| `format_duration_*` | Untouched — pure function. |

### Narrow and very-narrow must not move

The wide path keeps `!is_narrow() && !is_very_narrow()` as its entry condition,
unchanged. The narrow branches keep their `Constraint::Percentage` vectors,
their hardcoded sort indices (`3/4`, `4/5`, `1`), and their `usize::MAX`
sentinel. Three reasons not to fold them in now:

1. They use percentage constraints, which fill rather than sum. The cell-sum
   budget model does not describe them without modification.
2. The wide core needs 69 columns with turn data. It cannot serve the 60–68
   band, so very narrow has to survive regardless.
3. The point of this change is to be reviewable on its own terms. Rewriting all
   three layouts at once is the opposite of that.

Once the wide path is budget-driven, unifying the narrow branches becomes a
plausible follow-up — the wide core is already the narrow column set — but it
is a separate change with its own risk.

## Testing strategy

### Why the current suite could not have caught this

Every wide-layout test in `sessions.rs` runs at 200, 220 or 260 columns, or at
a width derived from the function under test:

```rust
fn model_gate_width(has_turn: bool) -> u16 {
    wide_layout_fixed_width(has_turn) + 1 + MODEL_COLUMN_MIN_CHARS + 2
}
```

That helper recomputes the production formula. It pins *consistency* between
test and implementation; it cannot detect that the formula's premise is wrong,
because the test moves whenever the code does. The only other wide-layout width
in the file is 120, in `empty_sessions_shows_refresh_message`, which returns
before rendering a table.

The result: **no test exercises the header between 71 and 174** — the entire
band where the defect lives. The suite is not weak, it is aimed somewhere else.

### What would catch it

Four assertions, all over a full width sweep. Rendering a 6-row `TestBackend` at
~180 widths is fast enough to run unconditionally.

**1. Legibility invariant.** For every width in `80..=240` and both turn
states: every admitted column's header renders in full in the buffer.

```rust
for width in 80u16..=240 {
    let header = header_line(&mut app, width);
    for label in expected_labels(width, has_turn) {
        assert!(header.contains(label), "label {label} squeezed at width {width}");
    }
}
```

This fails today at every width from 80 to 154. It is the single assertion that
would have turned #964 into a red test instead of a review comment.

**2. Sort-indicator invariant.** For every width in the sweep and each of
`SortField::{Cost, Tokens, Date}`: if the field's column is admitted, the
rendered header contains `"<label> ▼"`; if not admitted, no arrow appears
anywhere in the header. This generalizes #956's six-point matrix to the whole
range and is what makes the header/cells/constraints/indices agreement
*verified* rather than *argued*. Today it fails at 77 of 121 widths for
`Last Active ▼` alone.

**3. Monotonicity.** For every width *W* in the sweep, the set of headers at *W*
is a superset of the set at *W - 1*. Catches the non-monotonic admission the
ratatui solver produces today (`Cost ▼` legible at 111 and 112, gone at 113)
and catches a priority list whose groups are ordered inconsistently with their
widths.

**4. Threshold table.** Assert the exact width at which each group first
appears — 69, 87, 106, 128, 150, 160, 169, 180 with turn data; 64, 82, 101,
123, 145, 155, 164, 175 without.

> **These must be literal numbers in the test, never derived from the
> production function.** That is the mistake `model_gate_width` makes. A test
> that recomputes the formula it is testing cannot fail. Hardcoding turns the
> priority order into a reviewed artifact: change the order and the test names
> exactly which threshold moved, and a reviewer sees the user-visible
> consequence in the diff.

**5. Structural agreement**, cheap and near-tautological once the refactor
lands: `headers.len() == widths.len() == cells.len()` for every row over the
sweep and both turn states. It is worth writing precisely because it should be
impossible to fail — if it can fail, the derivations are not actually derived.

**Value truncation** deserves a note but not an assertion here. The proposal
prevents it structurally (a column is either admitted at natural width or
absent), so an assertion would only restate the legibility invariant. If a
future change reintroduces content that overflows its column, that belongs in a
formatter test next to `format_duration_*`.

## What this does not solve

1. **It does not make fifteen columns fit in 120.** Nothing does. The full
   table needs 180 columns and will still need 180 after this change. The
   proposal decides *what you lose* below that, not *whether* you lose
   something.

2. **The natural widths are still padded.** They are budget inputs, not
   measured content maxima. `format_tokens` produces at most `999.9B` (6) in
   10-cell columns; `format_cost` at most `$999.99` (7) in 10. Tightening
   Input/Output to 7, Cache R/W and Cost and Cost/1M to 8, Total to 7 and
   Cache× to 7 would free ~18 cells and pull every threshold down by up to 18 —
   the full table would land near 162. Deliberately *not* bundled: it is a
   readability tradeoff (columns lose their breathing room) and it makes widths
   content-dependent. This design makes it a one-line-per-column edit
   afterwards, which is the point.

3. **`Last Active` is 17 cells for `%Y-%m-%d %H:%M`.** The narrow path already
   uses `%m-%d %H:%M` (11). Making the format width-adaptive the way
   `daily.rs`'s `compact_full_date` does would free 6 cells in the widest
   single column in the table. Complementary and orthogonal; not part of this.

4. **Sessions only.** `daily.rs`, `models.rs`, `agents.rs`, `hourly.rs`,
   `monthly.rs` and `minutely.rs` all have the same header/cells/constraints/
   indices structure and have not been measured. `daily.rs`'s `full_layout_width`
   (112 / 105) is correct as of `226dfc75` — verified by hand, not by a test.

5. **Narrow and very narrow keep their hand-tuned percentage layouts**, by
   design (see above).

6. **Display width vs. character count.** Widths are counted in `chars()`, so a
   full-width grapheme occupies two cells while counting as one. The budget
   arithmetic is unaffected — the column widths are fixed — but truncation
   *inside* a cell can still overshoot by one cell per wide character. Model
   ids are ASCII in practice; session titles are not necessarily.

7. **No user-configurable columns**, no pinning, no per-client column sets. The
   descriptor list is the natural place to hang a config filter later; this
   proposal does not add one.

8. **No data-adaptive hiding** (order F above), deliberately.

## Decisions requested

1. **Decision A** — the priority order. Recommended:
   `Last Active` → `Model` → `Input`+`Output` → `Cache R`+`Cache W` →
   `Duration` → `Cache×` → `Cost/1M`. Amending it means editing one array and
   one table of literal thresholds in the tests.
2. **Decision B** — slack distribution. Recommended: Session to 40 first, then
   Model to 36.
3. **`has_model_data`** — add the availability gate symmetric with
   `has_turn_data`, or leave Model showing em-dashes when no session has model
   data?
4. **The 120-column regression** — is trading six stub columns for eight
   complete ones the right call at common terminal widths? This is the one
   place the proposal makes something visibly worse for some users, and it is
   the reason the priority order is a decision rather than an implementation
   detail.
