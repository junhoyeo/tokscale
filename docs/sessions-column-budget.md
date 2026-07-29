# Sessions tab: width-budgeted columns

Design proposal for [#964](https://github.com/junhoyeo/tokscale/issues/964). The
wide Sessions layout asks for 161 terminal columns and turns on at 80. This
document measures the gap, proposes replacing the hand-maintained column vectors
with a single priority-ordered descriptor list, and asks the maintainer to
settle one product question — the priority order itself.

> **Status: proposal. Nothing in this document is implemented.** It describes
> behavior the shipped binary does not have. Every `SessionColumn`,
> `WideCtx`, `WideLayout`, `ALL`, `WIDE_ORDER` and `WIDE_PRIORITY` below is a
> sketch, not a symbol you will find in the tree. Accept, reject, or amend.
> (`docs/` otherwise holds operational guides; if this lands as a standing
> document rather than an issue comment, `docs/design/` is the better home.)

Everything numeric below was computed or rendered against `226dfc75`
(`feat(tui): show model names with family-shade colors in Sessions tab (#956)`);
each section says how to re-derive it. Measured claims come from a
`TestBackend` sweep, not from estimation — where an earlier draft estimated
instead, the correction is called out in place.

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

The equivalent numbers for full header labels. The second column is the point:
"first renders in full" is not "renders in full from here on", because the
solver's remainder distribution keeps re-clipping labels above their first
appearance. Turn data present:

| label | first renders in full | clipped again at |
|---|---|---|
| `Msgs` | 81 | 84 |
| `Input` | 89 | 90, 91, 94, 95, 98 |
| `Total` | 91 | 92, 93, 96 |
| `Client` | 101 | — |
| `Cache×` | 108 | 109, 111 |
| `Output` | 110 | 112, 113, 114 |
| `Cache R` | 119 | 120, 121, 125 |
| `Cost/1M` | 119 | 120, 121, 122, 125 |
| `Cache W` | 121 | 122, 123 |
| `Duration` | 130 | 133, 134 |
| `Last Active` | 155 | — |

Every label is legible simultaneously only from **155** (turn data) / **150**
(without) — the point at which the last of them, `Last Active`, arrives and
nothing re-clips behind it.

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

The issue names four places that must stay in sync. There are six:

| # | what | location |
|---|---|---|
| 1 | `header_cells` vector | `sessions.rs:114-136` |
| 2 | pushed row cells | `sessions.rs:270-320` |
| 3 | `widths` / `Constraint` vector | `sessions.rs:353-376` |
| 4 | sort-indicator indices `8/9/12 + optional_cols` | `sessions.rs:151-181` |
| 5 | `wide_layout_fixed_width`'s hardcoded sum | `sessions.rs:30-41` |
| 6 | `truncate_text(session_label, 60)` | `sessions.rs:271` |

(1)–(3) desyncing misaligns the table: a header over the wrong data. (4)
desyncing puts the sort arrow on the wrong column. (5) desyncing moves the
Model gate to a width that no longer means what it says. (6) caps the Session
cell's *content* at 60 characters regardless of how wide the Session column is
actually rendered — at 260 columns the column is 82 cells and the title still
stops at 60. None of the six is checked by the compiler. `has_turn_data` and
`show_model` each have to be threaded through the first five by hand,
correctly, in the same commit — which #956 did, and which was the single
riskiest part of that PR.

(6) matters more under this proposal, not less: Session is the sole sink for
leftover cells, so the design deliberately grows it. `SessionColumn::Session`'s
`cell()` therefore has to be told what to truncate to, which is why `WideCtx`
below carries a resolved `session_width` rather than a constant.

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

/// Everything known *before* admission runs. Widths that depend on the
/// admitted set are deliberately absent — see the two-phase note below.
struct WideCtx {
    has_turn: bool,
    has_model: bool,
    last_active_fmt: &'static str,
    // theme-derived styles, extracted from `App` the way `render` already does
}

/// Produced by admission, consumed by rendering. This is where every width
/// that depends on the chosen set lives.
struct WideLayout {
    chosen: Vec<SessionColumn>,
    model_width: u16,    // >= MODEL_COLUMN_MIN_CHARS when Model is admitted
    session_width: u16,  // what Session::cell truncates to — see sync point 6
}

impl SessionColumn {
    fn header(self) -> &'static str { /* match, no wildcard arm */ }
    fn available(self, ctx: &WideCtx) -> bool { /* Turn => ctx.has_turn, ... */ }
    fn sort_field(self) -> Option<SortField> { /* Total/Cost/LastActive => Some */ }

    /// Minimum cells this column needs. The *only* width input to admission.
    fn natural(self, ctx: &WideCtx) -> u16 { /* match, no wildcard arm */ }

    /// Whether leftover cells may flow here (`Min`) or not (`Length`).
    fn grows(self) -> bool { matches!(self, Self::Session) }

    fn cell(self, s: &SessionUsage, app: &App, l: &WideLayout) -> Cell<'static> { /* match */ }
}
```

**One width method, not two.** An earlier draft of this design had both
`natural()` and a separate `constraint() -> Constraint`. Nothing would force
`constraint(c) == Length(natural(c))`, so a single mistyped literal would
reintroduce exactly the budget-vs-layout divergence this design exists to
kill — and hide it inside a descriptor that looks authoritative. The
constraint is therefore *derived*, never written:

```rust
// `natural()` is the ADMISSION width — what a column must be given to be let
// in at all. What it actually RECEIVES after slack distribution is the only
// width the renderer may use. For every fixed column the two are equal; for
// Session and Model they diverge, which is the entire point of slack
// distribution, so the resolved layout has to be in scope here.
impl WideLayout {
    fn constraint(&self, c: SessionColumn, ctx: &WideCtx) -> Constraint {
        let width = match c {
            // The allocated width, not the natural one — this is the whole point.
            SessionColumn::Model => self.model_width,
            other => other.natural(ctx),
        };
        if c.grows() { Constraint::Min(width) } else { Constraint::Length(width) }
    }
}
```

`Session` needs no arm: it is the sole `Min` column, so ratatui hands it every
cell slack distribution did not give `Model`. `self.session_width` exists for
`cell()` to truncate the title against, and must equal what the solver actually
allocates or the title is clipped with no ellipsis.

**`constraint()` must have the resolved layout in scope, not only `WideCtx`.**
An earlier revision of this document derived `constraint` from `ctx` alone
while `cell` took `layout`, which is a real bug two independent reviewers
caught: `Model.grows()` is `false`, so its constraint came out as
`Length(natural(Model))` — always 18 — while `cell()` formatted content to
`layout.model_width`, up to 36. ratatui allocates what the constraint asks for,
so every cell wider than 18 was silently clipped and the whole `model_extra`
computation was dead code. `Session` survived only because it is
`Constraint::Min` and absorbs surplus regardless.

The first attempt at this correction was itself wrong, and a reviewer caught
that too: it introduced a free function reading `layout.ctx`, a field
`WideLayout` does not have, so it would not compile. Making `constraint` a
method on `WideLayout` that *also* takes `&WideCtx` is what the implementation
in #966 does, and it is the right shape — the layout supplies the widths
admission computed, `ctx` supplies the fixed naturals, and neither is
reachable without the other.

The lesson generalises: any width that admission *computes* must flow to the
constraint, not just to the formatter. Having both derivations hang off the
resolved layout is what makes that structural rather than remembered.

**`Msgs` is 5 cells with turn data and 6 without.** `sessions.rs:362` reads
`Constraint::Length(if has_turn_data { 5 } else { 6 })` — Msgs borrows the
column Turn gives up. This is not cosmetic: every no-turn threshold in this
document depends on it (64 = 20 + 12 + **6** + 10 + 10 + 4 separators + 2
borders). `natural(Msgs, ctx)` must return `if ctx.has_turn { 5 } else { 6 }`,
keyed on `ctx.has_turn` and *not* on whether `Turn` was admitted. An
implementer who writes `Msgs => 5` shifts all eight no-turn thresholds down by
one, to 63/81/100/122/144/154/163/174, and fails the literal-threshold test
below with no indication of where the off-by-one came from.

Keying it off `ctx.has_turn` rather than off `chosen.contains(&Turn)` is what
makes the width answerable before admission runs — see the two-phase note
below — and putting `Turn` and `Msgs` in the same atomic group is what makes
the two formulations agree in the first place. With a per-column core they
could disagree: `Msgs` admitted at width 5 while `Turn` was dropped would
leave the table one cell short of what `required()` promised.

**Two phases, and they must not be circular.** `natural()` is what `required()`
sums, so it must be answerable before anything is admitted — which is why
`WideCtx` holds no widths. Slack is distributed *after* admission, producing
`WideLayout`, and only `cell()` sees it. Model's rendered width is therefore
`layout.model_width`, never `natural(Model)`; if `natural(Model)` returned the
post-slack width, admission would depend on its own output.

The load-bearing rule: **every one of those methods is an exhaustive `match`
with no `_ =>` arm.** Adding a variant then fails to compile until the header,
the width, the sort field, the availability predicate and the cell builder all
exist. That is the mechanism that replaces the current comment-and-hope
discipline, and it is the main reason to prefer an enum over, say, a vector of
closures.

Three arrays, deliberately separate. `ALL` and `WIDE_ORDER` happen to be
element-identical today because the display order and the declaration order
coincide — they are not the same constant, because `WIDE_ORDER` is free to be
reshuffled for looks and `ALL` is not. The completeness test below compares
them as multisets, so reordering `WIDE_ORDER` never breaks it and dropping a
column from it always does.

```rust
/// Every variant, exactly once. The other two arrays are checked against this.
const ALL: [SessionColumn; 15] = [
    Session, Client, Model, Turn, Msgs,
    Input, Output, CacheRead, CacheWrite, CacheHit,
    Total, Cost, CostPerMillion, Duration, LastActive,
];

/// Left-to-right display order. Cosmetic; changing it never changes what fits.
const WIDE_ORDER: [SessionColumn; 15] = [
    Session, Client, Model, Turn, Msgs,
    Input, Output, CacheRead, CacheWrite, CacheHit,
    Total, Cost, CostPerMillion, Duration, LastActive,
];

/// Admission order: earlier groups are admitted first and dropped last.
/// Each group is all-or-nothing. This array is the product decision below.
const WIDE_PRIORITY: [&[SessionColumn]; 8] = [
    &[Session, Client, Total, Cost, Msgs, Turn],   // core — one group, not six
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

**The core is one group, not six.** An earlier draft listed `&[Session]`,
`&[Client]`, `&[Total]`, `&[Cost]`, `&[Msgs]`, `&[Turn]` as six independent
all-or-nothing groups, which permits admission to stop *inside* the core and
render Session + Client + Total and nothing else. That is unreachable today
(the wide path starts at inner ≥ 78, the core needs 67), but the follow-up
floated at the end of this document — lowering the wide entry condition to
absorb the narrow layouts — would make those sets live at 60–68. Making the
core atomic turns "the wide layout always shows at least the narrow column
set" from an unstated consequence of `is_narrow() == width < 80` into a
property of the priority array. The floor it implies, `inner >= 67` with turn
data and `>= 62` without, is worth asserting directly.

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
    // Total, not partial: a column missing from WIDE_ORDER sorts last instead
    // of panicking. See "the arrays are not compiler-checked" below.
    chosen.sort_by_key(|c| WIDE_ORDER.iter().position(|o| o == c).unwrap_or(usize::MAX));
    chosen
}
```

**`break`, not `continue`.** Skipping a group that does not fit and trying the
next (narrower) one packs more columns in, but makes the admitted set
non-monotonic in width: at width *W* you would see Cache×, at *W+1* Duration
instead and Cache× gone. That is the same disorienting behavior the ratatui
solver already produces, and reproducing it deliberately would be worse.
Breaking gives a property worth having and worth asserting in a test:

> The **set of columns** shown at width *W* is a subset of the set shown at
> *W+1*, for every *W*.

State it that way and no more. The tempting stronger form — "widening a
terminal never removes information" — is false, and it is false because of the
next paragraph.

#### The Session sawtooth

The slack left by breaking early is not wasted: Session is `Constraint::Min`,
so ratatui hands every leftover cell to it and the slack becomes session-title
width. That is a good use for it, and it is also why the column *set* being
monotonic does not make the *display* monotonic. Slack is reclaimed the instant
the next group is admitted, so the Session column collapses back to its
minimum at every threshold:

```
W=149 (turn data): admitted through Input/Output, required 126, inner 147,
                   slack 21  → B2: Session 40, Model 19
W=150            : + Cache R/W,                  required 148, inner 148,
                   slack 0   → B2: Session 20, Model 18
```

Widening 149 → 150 halves the visible session title from 40 characters to 20.
It happens at all seven thresholds (turn data, B2 slack):

| widening | Session before → after |
|---|---|
| 86 → 87 | 37 → 20 |
| 105 → 106 | 38 → 20 |
| 127 → 128 | 40 → 20 |
| 149 → 150 | 40 → 20 |
| 159 → 160 | 29 → 20 |
| 168 → 169 | 28 → 20 |
| 179 → 180 | 30 → 20 |

Under decision B1 (Model-first) it is Model that sawtooths instead — 36 → 18 at
149 → 150 — and Session moves 23 → 20. The slack rule chooses *which* column
sawtooths; no slack rule avoids it. The cause is `break` plus a growing
column, not the distribution policy.

The "what changes for users" table further down already shows this: Session is
31 at 80, 33 at 100, 34 at 120, and 21 at 161. That is not a typo.

**This is accepted, not solved.** Two reasons. First, the alternative — pin
Session and Model to `Length(minimum)` and spend slack only past the last
threshold — removes the sawtooth but costs title width at every width below
180: Session would be 20 rather than 31 at 80 columns, 20 rather than 34 at
120, and the reclaimed cells would sit as dead space at the right edge of the
table. That is a worse trade at exactly the widths most people use. Second,
losing title characters is a soft degradation of one column, whereas losing a
column is a hard loss of a number; the monotonicity that matters is the one
this design does guarantee.

It does, however, need to be *visible*. Proposed test #3 below asserts header
**sets** only, so it stays green while a Session-width regression ships. If the
sawtooth is ever meant to be bounded, that needs its own assertion.

#### The arrays are not compiler-checked — and that is the whole risk

The exhaustive-`match` rule above is real, but it covers the six *methods*. It
says nothing about `WIDE_ORDER` and `WIDE_PRIORITY`, which are hand-maintained
arrays of column names. Adding `SessionColumn::Reasoning` compiles cleanly
while absent from both. Then:

- **absent from `WIDE_PRIORITY`** → the column is never admitted at any width
  and silently never renders;
- **present in `WIDE_PRIORITY`, absent from `WIDE_ORDER`** → the `position()`
  lookup in the sort key has no answer. Written as `.unwrap()` — which is how
  the first draft of this section had it — that is a **panic inside `render`**,
  i.e. inside the draw loop, leaving the terminal in raw mode. Worse, it fires
  only once the column is actually admitted, so a test at 200 columns passes
  and a user at 250 crashes.

That would be strictly worse than the misalignment it replaces: a silent
mis-render degrades, a panic in the draw loop does not. Drift is not designed
out by the enum; it is *relocated* from six call sites to two arrays.

This document supplies its own evidence. An earlier draft of the block above
declared `const WIDE_PRIORITY: [&[SessionColumn]; 12]` and then listed
thirteen elements. A hand-maintained count of columns drifted inside the
document arguing that hand-maintained counts of columns drift.

So the arrays need their own check, and it is about ten lines:

```rust
fn order_index(c: SessionColumn) -> usize {
    WIDE_ORDER.iter().position(|o| *o == c).unwrap_or(usize::MAX)  // total, never panics
}

#[test]
fn every_column_is_ordered_and_prioritized() {
    let mut order = WIDE_ORDER.to_vec();
    let mut prio: Vec<SessionColumn> = WIDE_PRIORITY.concat();
    let mut all = ALL.to_vec();
    for v in [&mut order, &mut prio, &mut all] {
        v.sort_by_key(|c| *c as usize);
    }
    assert_eq!(order, all, "WIDE_ORDER is not a permutation of ALL");
    assert_eq!(prio, all, "WIDE_PRIORITY is not a permutation of ALL");
}
```

`ALL` is still hand-written, but it is one flat list checked against two
others, and `#[deny(unreachable_patterns)]`-style pressure does not help here —
only this assertion does. **It is the only test in this proposal that actually
enforces the no-drift claim the design is sold on**, and it belongs in the
first commit, not a follow-up.

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

This step is what produces `WideLayout`: it resolves `model_width` (read by
`SessionColumn::Model`'s `cell()`) and `session_width` (read by
`SessionColumn::Session`'s `cell()` as the `truncate_text` limit — sync point 6
above, which today is the hardcoded 60). Both are outputs of admission, which
is why neither appears in `WideCtx`.

### The derivations

```rust
let layout  = admit_and_distribute(inner.width, &ctx);   // -> WideLayout
let chosen  = &layout.chosen;

let widths  = chosen.iter().map(|c| constraint(*c, &ctx));
let cells   = chosen.iter().map(|c| c.cell(session, app, &layout));   // per row

// Headers carry the sort arrow, so these are one derivation, not two: the
// arrow is chosen by identity (`sort_field()`), never by column index.
let headers = chosen.iter().map(|c| match c.sort_field() {
    Some(f) if f == app.sort_field => format!("{}{}", c.header(), sort_indicator(f)),
    _ => c.header().to_string(),
});
```

Sync point (4) stops being arithmetic. Nothing computes an index at all, so
the arrow cannot land on the wrong column; and when the sorted column is not
admitted, no descriptor matches and no arrow is drawn — which is what
`usize::MAX` currently fakes for the narrow branches. Sync point (5),
`wide_layout_fixed_width`, is deleted: `required()` computes the same thing
from the descriptor list instead of from a second copy of the numbers. Sync
point (6)'s `60` becomes `layout.session_width`.

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
| **157** | 14 columns, every header label and the active sort arrow legible for the first time — but `Last Active` still renders `2025-01-04 23` | 12 columns — through Cache R/W, Session 27 |
| **161** | 14 columns, every column finally at its natural width; first width at which no *value* is truncated either | 13 columns — everything except Cache× and Cost/1M, **plus Model**, Session 21 |
| **180** | 15 columns, everything legible, Model appears here for the first time | 15 columns, identical to today |

Three different widths get called "legible", so be precise about which is
which (turn data present; without it, subtract 5):

- **155** — every header *label* renders in full and stays that way.
- **157** — the active sort arrow fits too, for all three sort fields; this is
  the first width at which the header is completely honest.
- **160** — the last truncated *value* (`Last Active`) becomes complete.
- **161** — every column reaches its natural width. Nothing changes visually
  from 160 for this row; it is the arithmetic milestone, and it is the number
  #956's Model gate is derived from.

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
| `has_turn_data` | Kept as-is, becomes `WideCtx::has_turn`. Feeds both `Turn::available()` **and** `natural(Msgs)` (5 vs 6) — see the descriptor section. |

### `has_model_data`

Today there is no data-availability gate on Model: a dataset where every
session has an empty `models` renders a column of em-dashes. Adding
`has_model_data` symmetric with `has_turn_data` would free 19 cells for real
data. This is a small, separable behavior change — recommended, but call it out
in review rather than smuggling it in.

**It is not independent of decision A, and adopting it changes the threshold
table the tests below hardcode.** Dropping Model frees its 18 cells plus a
separator, and under `break` semantics that budget flows down to the groups
behind it, so every threshold after `Last Active` moves:

| admitted | turn data, model data | turn data, **no** model data |
|---|---|---|
| core | 69 | 69 |
| + `Last Active` | 87 | 87 |
| + `Model` | 106 | — |
| + `Input`, `Output` | 128 | **109** |
| + `Cache R`, `Cache W` | 150 | **131** |
| + `Duration` | 160 | **141** |
| + `Cache×` | 169 | **150** |
| + `Cost/1M` | 180 | **161** |

Without turn data the no-model column reads 64 / 82 / — / 104 / 126 / 136 /
145 / **156**.

Those two final numbers are a free cross-check worth keeping: 161 and 156 are
exactly today's full-table widths for the no-Model case (`sessions.rs:30-41`),
which they have to be, because dropping Model from the descriptor set leaves
precisely the constraint vector that ships today. If an implementation of this
design produces anything else there, the descriptor widths are wrong.

So decision A's test table becomes two-dimensional if this is adopted. Decide
them together, or land `has_model_data` in a separate commit with its own
threshold table.

### Existing tests

| test | fate |
|---|---|
| `wide_terminal_renders_session_and_duration_columns` (200) | Passes unchanged — Duration is admitted from 160. |
| `model_column_shows_model_names` (200) | Passes unchanged. |
| `model_column_shows_multiple_models` (220) | Passes unchanged; Model reaches 36 at 220 under B2. |
| `session_title_displayed_when_available` (200) | Passes unchanged. |
| `session_column_expands_on_wide_terminal` (260) | Passes; Session gets 82 cells against the 53-char title asserted — but only because 53 < the hardcoded `truncate_text(.., 60)`. That 60 is sync point 6 and this test is what would catch it regressing, so it should be re-pointed at `layout.session_width`. |
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

This fails today at every width from **80 to 154** with turn data, and **80 to
149** without — contiguous bands in both cases, ending where `Last Active`
finally fits. It is the single assertion that would have turned #964 into a red
test instead of a review comment.

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

> Note what this does **not** cover: it compares header *sets*, so it stays
> green through the Session sawtooth documented above (Session 40 at 149, 20 at
> 150). That is deliberate — the sawtooth is accepted — but it means this test
> must not be cited as evidence that widening never costs the user anything.

**4. Threshold table.** Assert the exact width at which each group first
appears — 69, 87, 106, 128, 150, 160, 169, 180 with turn data; 64, 82, 101,
123, 145, 155, 164, 175 without. (If `has_model_data` is adopted, this becomes
four rows, not two — see that section for the other eight numbers.)

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

**6. Array completeness** — `WIDE_ORDER` and `WIDE_PRIORITY.concat()` are both
permutations of `ALL` (code in the admission section above). Unlike the other
five, this one is not about widths at all. **It is the only assertion in this
set that enforces the no-drift property the whole design is sold on**, because
the exhaustive-`match` rule covers the six descriptor methods and stops there —
the two arrays are hand-maintained and the compiler never looks at them. Add a
variant, forget the arrays, and you get either a column that never renders or a
sort-key lookup with no answer. Without this test the refactor moves drift from
six visible call sites into two arrays that look authoritative.

**Value truncation** needs an assertion after all, and an earlier revision of
this document was wrong to say otherwise. It claimed the proposal "prevents it
structurally (a column is either admitted at natural width or absent)". That is
true of the *squeeze* — no admitted column is ever narrowed below its natural
width — but it is not true of truncation in general, because a formatter can
produce content wider than the natural width it was sized for. Two reviewers
caught this independently, and the document already contradicted itself: the
"what this does not solve" section notes `format_cost_per_million` is unbounded.

The concrete case: `format_cost_per_million` is `format!("${:.2}", per_m)`
(`widgets.rs:54-60`) with no compact branch. A session with `cost = 100.0` over
`100` tokens yields `$1000000.00` — 11 cells in a 10-cell column, clipped to
`$1000000.` **even at 180 columns where every column is admitted at natural
width**. Rare, but it is precisely the "plausible-looking wrong number" failure
this whole document is about, and admission cannot fix it because the column is
already as wide as it asked to be.

So two things are required, and neither is optional:

1. A per-formatter assertion that each column's output fits its `natural()`
   width for a set of adversarial inputs — very large costs, very small token
   counts, long durations, multi-model sessions. This is the formatter-level
   twin of the layout-level invariant, and it belongs next to
   `format_duration_*`.
2. `format_cost_per_million` needs a compact branch (or a wider `natural()`).
   Deciding which is a judgement call: widening Cost/1M to 11 raises every
   threshold that admits it by one, whereas a compact branch keeps the
   thresholds and changes what a handful of extreme sessions display. The
   compact branch is the better trade — the thresholds in this document are
   already tight, and `$1.00M/M` reads fine.

Neither is scoped out. A design that prevents column squeeze but still ships a
formatter that overflows its own column has not solved the reported problem.

## What this does not solve

1. **It does not make fifteen columns fit in 120.** Nothing does. The full
   table needs 180 columns today and **183** after this change. The proposal
   decides *what you lose* below that, not *whether* you lose something.

   The extra three cells are not slippage. Implementing this surfaced a second
   silent-truncation bug of exactly the kind the document is about: `Client` at
   its current 12 cells clips three shipped client names with no marker, and
   renders `Antigravity CLI` and `Antigravity` **identically** — a
   misattribution in the one column whose job is saying which tool a session
   came from. Its natural width becomes 15, the widest name the client registry
   can produce, pinned by a test that enumerates the registry so a new client
   cannot reintroduce the clip.

   Verified by rendering, not arithmetic: `Cost/1M` — the last column in the
   priority order, so the one that marks the full set — is absent at 182 and
   present at 183.

2. **The natural widths are still padded — but by less than they look, and
   two of them are not padded at all.** They are budget inputs, not measured
   content maxima. `format_tokens` produces at most `999.9B` (6) in 10-cell
   columns, so those have real slack. The cost formatters do not:

   | formatter | actual maximum | column | measured |
   |---|---|---|---|
   | `format_cost` | `$1234.6K` (8) — there is a `>= 1000.0` branch at `widgets.rs:41-50` that switches to `${:.1}K`, so `$999.99` is *not* the ceiling | 10 | `format_cost(1_234_567.0)` |
   | `format_cost_per_million` | **unbounded** — `widgets.rs:54-60` is a plain `${:.2}` with no compact branch at all | 10 | `format_cost_per_million(10.0, 100)` = `$100000.00` (10), exactly filling today's column |

   So an earlier draft's proposal to tighten Cost and Cost/1M to 8 cells would
   truncate on reachable inputs — a session costing over $1,000 already needs 8,
   and Cost/1M has no ceiling to design against. Tightening Input/Output to 7,
   Cache R/W to 8, Total to 7 and Cache× to 7 is still sound and frees 14 cells
   (`3+3+2+2+3+1`), putting the full table at 166; the further 4 cells that
   would reach 162 come only from the two cost columns, and require giving
   `format_cost_per_million` a compact branch first.

   Deliberately *not* bundled either way: it is a readability tradeoff (columns
   lose their breathing room) and it makes widths content-dependent. This design
   makes it a one-line-per-column edit afterwards, which is the point. The
   lesson worth carrying is that this document's own first draft asserted a
   formatter maximum it had not measured, in a section arguing that unmeasured
   width constants are the bug.

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

7. **The Session column sawtooths at every threshold** (40 cells at 149, 20 at
   150, and six more like it — full table in the admission section). The column
   *set* is monotonic in width; the space given to the title is not. Accepted
   rather than fixed, because the alternative costs title width at every common
   terminal size. Under decision B1 it is Model that sawtooths instead.

8. **Date sort still has no visible target at the narrowest wide widths.**
   This document opens by complaining that "a user who presses the sort key
   sees the row order change with no feedback about which column drives it".
   Under the recommended order that is true from 80 to 86 with turn data, and
   80 to 81 without, because `Last Active` is not admitted until 87 / 82 —
   `position()` returns `None` and no arrow is drawn, by design. It is a
   reduction from 77 of 121 widths to 7, not an elimination. The obvious fix,
   force-admitting whichever column is currently sorted, is rejected: it breaks
   the subset property, since changing the sort at a fixed width would change
   the column set. Worth seeing before choosing a priority order — any order
   that ranks `Last Active` lower makes this band wider (Order C leaves it
   unanswered to 150).

9. **No user-configurable columns**, no pinning, no per-client column sets. The
   descriptor list is the natural place to hang a config filter later; this
   proposal does not add one.

10. **No data-adaptive hiding** (order F above), deliberately.

## Decisions requested

1. **Decision A** — the priority order. Recommended:
   `Last Active` → `Model` → `Input`+`Output` → `Cache R`+`Cache W` →
   `Duration` → `Cache×` → `Cost/1M`. Amending it means editing one array and
   one table of literal thresholds in the tests.
2. **Decision B** — slack distribution. Recommended: Session to 40 first, then
   Model to 36. Note this decides *which* column sawtooths at the seven
   thresholds, not whether one does: B2 sawtooths Session (40 → 20 at 149 →
   150), B1 sawtooths Model (36 → 18 at the same step).
3. **`has_model_data`** — add the availability gate symmetric with
   `has_turn_data`, or leave Model showing em-dashes when no session has model
   data? **Not independent of A**: adopting it doubles decision A's threshold
   table (69/87/**109/131/141/150/161** with turn data). Decide with A or land
   it separately.
4. **The 120-column regression** — is trading six stub columns for eight
   complete ones the right call at common terminal widths? This is the one
   place the proposal makes something visibly worse for some users, and it is
   the reason the priority order is a decision rather than an implementation
   detail.

5. **`format_cost_per_million` overflow** — it is unbounded (`widgets.rs:54-60`,
   a plain `${:.2}`), so `format_cost_per_million(100.0, 100)` renders
   `$1000000.00`, 11 cells clipped into a 10-cell column *even at 180 columns*.
   Recommended: give it a compact branch rather than widening the column to 11,
   which would raise every threshold that admits Cost/1M. Raised by review; the
   fix is required, only the form of it is a choice.

Two things this document asks reviewers *not* to treat as decisions, because
they are corrections rather than choices: the `ALL` / `WIDE_ORDER` /
`WIDE_PRIORITY` permutation test (#6 in the testing section) must land in the
first commit or the design's central no-drift claim is unenforced; and
`natural()` must be the single source of every width, with `Constraint`
derived from it rather than written alongside it.
