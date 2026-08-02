# Ratchet inflation recovery

Design for [#960](https://github.com/junhoyeo/tokscale/issues/960). Rescanning
the same local history under a different timezone permanently inflates stored
totals, and the monotonic guard that causes it exists to prevent a real
production data loss. This document proposes a correction that heals the
inflation without weakening that protection.

> **Status: none of Phases 1–6 is implemented.** Accept, reject, or amend.
> Several *adjacent* defects this document catalogued have since been fixed and
> are marked inline — see "Already fixed" below.

### Already fixed

Found while investigating #960, shipped separately, and no longer open:

| | Fix | Commit |
|---|---|---|
| OpenCodeReview never reached the submit path | parse it there, through the source cache | `b9fd593a` (#990) |
| Alias fold mutated the stored raw breakdown, compounding per-model totals | `cloneClientBreakdownForFold` | `cdfd1d0c` (#989) |
| Period leaderboards credited whole daily rows to one client | `scopeBreakdownToDirectives` | `cb65bbbf` (#988), `e80b44a3` (#991) |
| Profile day aggregator mutated query-result rows (latent) | `cloneClientModels` | `ce555ab6` (#992) |

None of these was the ratchet. They are recorded because the analysis below
originally leaned on them, and because #990 in particular changes what the
census will see.

### Verification pin

Every `file:line` below was read at `ce555ab6`, against these files only:

```
packages/frontend/src/app/api/submit/route.ts
packages/frontend/src/lib/db/helpers.ts
packages/frontend/src/lib/leaderboard/getLeaderboard.ts
crates/tokscale-core/src/{lib.rs, sessionize.rs, clients.rs, message_cache.rs}
crates/tokscale-core/src/sessions/mod.rs
```

**Pin to those files, not to `main`'s HEAD.** An earlier version pinned to HEAD
and read as stale after three commits that touched nothing it cited — a
staleness marker that fires on unrelated changes trains readers to ignore it.
Line numbers *did* rot for real in the #988–#992 batch (`route.ts` shifted a
uniform +28), so before trusting one, re-run "How to re-derive"; each row there
greps for content rather than seeking to an offset.

## What the census found (2026-07-31)

Run read-only against production. It uses the one timezone-invariant reference
that already exists: `submitted_devices.total_active_time_ms` comes from the
CLI's `timeMetrics`, which sums raw interval durations, while
`daily_breakdown.active_time_ms` apportions the same intervals across local
days. Their ratio measures the re-split directly, with **no migration, no
deploy, and no waiting for resubmits**.

```
coverage        3959 devices total
                 650 carry the invariant   (16%)
                2457 have daily rows

bands (of 650)   438 clean   (<1.05x)      67%
                 131 mild    (1.05-1.5x)   20%
                  56 clear   (1.5-3x)       8.6%
                  25 severe  (>3x)          3.8%

of the 212 above 1.05x, split by whether the ratio tracks history length:
                   4 artifact-like    ratio ~ scan-window count (incl. one 2300x)
                  11 ambiguous
                 197 NOT explained by scan windowing  (1.05-12.43x)
```

**Three findings, and the third is the one that matters.**

1. **The inflation is real and not rare.** Only 4 devices are cleanly explained
   by the `--since`/`--date` artifact. 197 of 650 measurable devices — 30% —
   show inflation that scan windowing does not account for. The mechanism is
   active, not theoretical.

2. **The extreme tail is artifact, not ratchet.** 2300x cannot be a re-split; a
   re-split moves usage between adjacent days, it cannot multiply a total by
   three orders of magnitude. Do not size the problem from the worst ratios.

3. **The census cannot see the accounts that hold the tokens.** Devices above
   1.5x hold **11.95% of the measurable slice but 0.02% of site-wide daily
   tokens** — because the measurable slice itself is only ~0.17% of all tokens.
   `total_active_time_ms` requires a recent CLI, so the 650 visible devices are
   recent, low-history adopters. **The 84% this cannot measure hold essentially
   all of the tokens.**

So the incidence question is answered (common) and the magnitude question is
not (unmeasured, on the population that matters). That is precisely the gap
Phase 1 exists to close, and it is why Phase 1 survives while the phases
downstream of it stay gated.

**Reproduce:** the script is not committed; rebuild it from the ratio above
against `submitted_devices` joined to `daily_breakdown` on
`submitted_device_id`, filtering `total_active_time_ms > 0`.

## The two failures are one mechanism

`compute_daily_active_time` (`crates/tokscale-core/src/sessionize.rs:285`)
delegates to `compute_daily_active_time_with_timezone(intervals, &chrono::Local)`
at `:288`, and `timestamp_to_date`
(`crates/tokscale-core/src/sessions/mod.rs:443-444`) does the same for token
attribution. So which calendar day a unit of usage lands in is a function of the
machine's timezone at scan time.

`mergeClientBreakdownsWithRegressionGuard`
(`packages/frontend/src/lib/db/helpers.ts:154`) then defends every per-day,
per-client decrease. A timezone change moves usage from day `d` to `d-1`; the
guard defends the stale value on `d` and accepts the new value on `d-1`, so the
account is credited twice.

The guard is not a mistake. `d9df8c9c` ("fix(submit): preserve usage history
after local session cleanup") added it after a user deleted local session files,
resubmitted, and had real history erased:

> `Rejected: Replace stored metrics with the newest snapshot | repeats the production data loss caused by local session cleanup`

Both failures present as the same signal — *a per-day client total went down* —
and the payload carries nothing that separates them.

## The fix already exists here, applied to the wrong column

`route.ts:791-802` states the problem and its solution in the tree today:

> Session-shape totals come from the PER-DEVICE high-water marks, not from
> `SUM(daily_breakdown.active_time_ms)`. … (2) Timezone stability: the daily rows
> apportion each interval across LOCAL calendar days, so rescanning the same
> history under a different TZ re-splits it; combined with the monotonic per-day
> merge that permanently inflates `SUM(daily)`.

So `totalActiveTimeMs` is derived from `submittedDevices` (`:815-818`) and is
immune. `totalTokens` is still `SUM(dailyBreakdown.tokens)` (`:779`, written at
`:862`) and is not — only because no per-device token total exists to derive
from. Tokens have the same invariant: a re-split moves them between days without
creating or destroying them, so a total over a window wider than the shift is
invariant to it.

**Which window is not a detail.** It is the central design decision, and the
next section is the honest accounting of what each choice costs.

### Why not active time as the sensor

An earlier draft gated on `total_active_time_ms`. It is timezone-invariant, but
it is a **proxy** for token coverage rather than a measure of it: only 11 of 45
session parsers populate `duration_ms`, the rest falling back to the wall-clock
span between messages (`sessionize.rs:155-160`), and a single-message session
contributes zero active time regardless of its tokens.

## Bucket width: a bounded-error trade-off, not an exact fix

Any bucket keyed on a **local** date inherits the instability that causes the
bug. Widening the bucket reduces the error; it never removes it. Two errors
trade against each other, and they are not symmetric in cost.

**Boundary leak.** Offsets span UTC−12 to UTC+14, so shifted instants differ by
up to 26 hours and a date can move by up to **two** calendar days in the extreme
(`2026-01-01T10:00Z` is Jan 2 in UTC+14 and Dec 31 in UTC−12); one day is typical
for zone pairs one user alternates between. At each bucket boundary that sliver
is counted in both buckets, permanently.

**Swallowing.** Inside a bucket, a deletion freezes it. 100 tokens submitted,
sessions deleted, 30 tokens earned afterwards in the same bucket: the payload
reports 30, `GREATEST` holds 100, and the new work is invisible until it exceeds
the old peak on its own. Today's per-day rows get this right — January preserved,
February inserted alongside, total 130 — so a bucket that is too wide is a
**regression against current behavior**, landing on the `d9df8c9c` user.

| width | boundaries/year | swallow window |
|---|---|---|
| daily | 365 | none |
| weekly | 52 | ≤ 7 days |
| monthly | 12 | ≤ 31 days |
| yearly | 1 | ≤ 366 days |

A boundary leaks only the midnight-crossing sliver of one or two days. Swallowing
hides all of a user's recent work and is immediately visible as "my number
stopped moving". **The costs are lopsided toward preferring narrower buckets**,
which is the opposite of what the previous draft assumed when it picked monthly.

Rather than settle this by intuition, Phase 1 measures it.

## Storage

```
submitted_device_client_totals(
  submitted_device_id, client, origin, bucket_width, bucket_key,
  tokens_highwater, cost_highwater)
PRIMARY KEY (submitted_device_id, client, origin, bucket_width, bucket_key)
```

maintained with `GREATEST` on conflict, exactly as `route.ts:440` does. Buckets
are folded server-side from the payload's `contributions` on their `date`, so no
CLI change is required.

`bucket_width` lets Phase 1 record `week` and `month` side by side and retire the
loser. `bucket_key` is a stable string (`YYYY-MM`, or ISO `YYYY-Www`).

**`origin` is part of the key, and this is load-bearing.** `getSubmitDevice`
(`:182-196`) falls back to `LEGACY_SUBMIT_DEVICE_KEY` when a payload omits
`device`, so a `tokscale import` backfill and a legacy CLI submit land on the
*same* device row. Keyed without origin, `GREATEST` would take the **max** of
imported and locally-scanned history instead of their sum, silently dropping
whichever is smaller from the ranked total.

`submissions.totalTokens` cannot serve as its own reference: it is recomputed
from the daily rows every submit (`:779`) and is itself inflated.

## Phase 1 — Populate only. No behavior change.

Write the table at both widths. Change nothing that reads.

Inert by design, and it converts two guesses into measurements:

```sql
-- per client, per device, per bucket
SUM(daily tokens in that bucket) / tokens_highwater
```

- **How much inflation exists, and when** — a per-bucket ratio shows which
  periods drifted, which a lifetime ratio cannot. This is the census every later
  phase is gated on, and it is on tokens rather than the active-time proxy in
  #960's first comment.
- **What the boundary leak actually costs** — comparing the weekly and monthly
  reconstructions of the same account measures the leak directly, settling the
  width empirically instead of by argument.

Nothing can regress, because nothing reads it yet.

### What it costs, measured (2026-07-31)

"Inert" describes the read side. It says nothing about the write side, so this
was measured against production rather than asserted.

```
rows per full-history submit — actual distinct (client, bucket) pairs
  both widths      p50   33    p95   122    max   349
  month only       p50    9    p95    32    max    91
  daily_breakdown
  writes today     p50   64    p95   218    max  1257

steady-state table   108,866 rows (both widths) / 29,640 (month only)
daily_breakdown      204,696 rows / 305 MB
```

**Phase 1 writes roughly half of what the submit path already writes**, and its
rows are narrow — five key columns and two numerics, against
`daily_breakdown`'s JSONB payload. Storage is not a consideration.

A first estimate put this 6–7x higher by multiplying bucket count by client
count. That is an upper bound, not a count: a device with nine clients runs one
to three of them in any given month, so the real cardinality is far below the
product. Measure this rather than deriving it.

### The real exposure is placement, not volume

The submit path runs inside a single `db.transaction` (`route.ts:322`) and
takes `.for('update')` on the submissions row inside it (`:341`), which
serializes every submit for a user across all their devices until commit. A
Phase 1 write placed there inherits both properties:

- it extends the lock hold, modestly — about +50% on write volume at p95;
- **a defect in it fails the user's submit entirely.** "Nothing reads it" is no
  protection here. This repo has already seen a `COALESCE(SUM(x),0)::int`
  overflow abort a transaction, and `tokens_highwater` is exactly the kind of
  column that invites the same mistake.

**It does not have to live there.** The write is a `GREATEST` upsert, so it is
idempotent: applying it twice is applying it once. That means it can run after
the submit transaction commits, at which point a failure costs one deferred
measurement — repaired by the next submit — instead of a rejected submission.
The reconciliation Phase 4b needs is unaffected either way.

The cost of moving it out is that the table lags the daily rows by one submit
for any request that dies between commit and the follow-up write. Phase 2 must
therefore treat a missing bucket as *unknown*, never as zero — which is already
required for a different reason, see that phase's coverage gate.

Writing only one bucket width halves everything above again, and Phase 3 makes
the width question much less interesting by permitting daily buckets outright.

**One thing the census cannot see — fixed, but it still distorts the baseline.**
`ClientId::OpenCodeReview` declared `submit_default: true`
(`crates/tokscale-core/src/clients.rs:509-517`) while being parsed only by
`parse_local_clients` (`lib.rs:3265`), which the submit path never called. Its
usage showed up in `tokscale report` and had never once reached the server.

Fixed in `b9fd593a` (#990) — the submit path now parses it through the shared
source cache. But every affected account's *history* is still missing it, so as
those users resubmit they will show a step increase. **That is real usage
arriving late, not drift, and the census must not score it as inflation.**

Audited against the whole registry at the time: of 37 `submit_default` clients
it was the only one missing (Goose, Hermes and Kilo look absent from a naive
`ClientId::` grep but are handled at `lib.rs:1171`, `:1161`, and via
`ClientId::KiloCode` at `:1087`).

## Phase 1.5 — Dual read: compute both, serve the old, record the delta

Phase 1 and Phase 2 are already a dual-*write* followed by a cutover. What sits
between them is a **flip gated by a rule**, not by evidence: the coverage gate
below asks "do rows exist?", never "do the two derivations agree?". Nothing in
the plan as written ever compares them on live traffic before the switch.

So add a stage that does. On each submission recompute, derive the total **both**
ways — `SUM(dailyBreakdown.tokens)` and `SUM(tokens_highwater)` — serve the old
one, and record the pair.

It costs one extra aggregate per submit and carries no read risk, because the
value served is unchanged. In exchange:

- **The token-axis census arrives as a by-product**, on real traffic, without
  anyone running a script. This is the measurement the 2026-07-31 census could
  not reach.
- **Phase 2's gate stops being a rule and becomes a measurement**: switch when
  the recorded deltas agree within tolerance, for a stated share of users, over
  a stated window. "Coverage exists" is a much weaker claim than "the numbers
  match".
- **The zero-out failure is observed rather than reasoned about.** A user whose
  high-water sum is 0 or partial shows up as an enormous recorded delta long
  before any read depends on it.

### The table cannot be seeded from `daily_breakdown` — the warm-up is forced

This is the part that makes dual mode mandatory rather than merely prudent, and
it is worth stating plainly because the obvious shortcut is actively harmful.

Backfilling `tokens_highwater` from existing daily rows would set it to
`SUM(daily in bucket)` — **the inflated value**. A later full scan reports the
true, lower bucket total, and `GREATEST(inflated, true)` keeps the inflated one.
Permanently. Seeding the table from the data it exists to correct would cement
the very inflation it is measuring, with no path back.

So the table starts empty and can only be filled by incoming payloads. The
warm-up period is not a convenience to be optimised away; it is forced by the
merge semantics. That is also the real reason Phase 2 needs a coverage gate at
all, and why row 9 of the state table insists a missing baseline must never be
read as `0`.

### What can be deprecated afterwards, and what cannot

- **Can be retired:** the `SUM(daily)` derivation *for submission totals*, once
  Phase 2 is stable and the deltas have stayed flat.
- **Cannot be retired:** `daily_breakdown` itself. The heatmap and per-day views
  read it, Phase 4 repairs it, and the high-water table holds bucket totals with
  no day-level detail to replace it with.

Dual mode ends by retiring a *derivation*, not a table.

### Make the write killable without a deploy

The migration is effectively irreversible — drizzle stores the content hash of
each applied migration, so migrations are immutable once applied (see
`AGENTS.md`). The *write* need not inherit that property. Put it behind a flag
so a misbehaving Phase 1 can be switched off in seconds, without reverting a
migration or shipping a rollback. Combined with after-commit placement, that
leaves Phase 1 with no failure mode that reaches a user.

## Phase 2 — Switch the read path

Change `:779` to derive `totalTokens` and `totalCost` from
`SUM(tokens_highwater)` over the winning `bucket_width`, mirroring `:815-818`.

**The leaderboard becomes correct immediately** — `getLeaderboard.ts:353,355,380`
reads `submissions.totalTokens` — with no row rewrite, no delete path, no backup
table and no gate. The precedent sits twelve lines below the line being changed.

**This phase can zero out real accounts if shipped naively, and that is its one
sharp edge.** A user's high-water rows only exist once they have submitted since
Phase 1 landed. Switch the derivation before that and `SUM` over zero rows
returns 0 — the account's ranked total collapses to nothing. The damage is
display-only, since `daily_breakdown` is untouched and the next submit repairs
it, but it is a visible, alarming, self-inflicted outage for a user who did
nothing.

Two acceptable guards, in preference order:

1. **Coverage gate.** Do not switch until Phase 1 shows high-water rows covering
   every device that has submitted recently. Phase 1 already measures exactly
   this, so the gate costs nothing extra.
2. **Per-user fallback.** Derive from the high-water table only when the user has
   rows spanning their stored date range; otherwise fall back to
   `SUM(daily_breakdown)` for that user. Correct but leaves two code paths live,
   and the fallback path stays inflated — so it must be temporary, not permanent.

`COALESCE(SUM(highwater), SUM(daily))` looks like a third option and is not one:
a user with *partial* coverage returns a non-null but too-small sum, which
coalesce will happily accept.

| Surface | Source | Fixed by Phase 2? |
|---|---|---|
| Leaderboard, profile total, all-time | `submissions.totalTokens` | **yes, to within the boundary leak** |
| Heatmap, per-day views, weekly/monthly | `daily_breakdown` rows | no — Phase 4 |
| `inputTokens` / `outputTokens` (`:781-782`) | `daily_breakdown` | no — no payload-level invariant exists |

Deriving per-device also inherits the additivity the comment at `:794-797`
cites: two devices reporting 100 and 40 total 140, where a max would drop the
second machine.

**Known limit.** All device-less submissions share `LEGACY_SUBMIT_DEVICE_KEY`, so
a legacy user with two machines has both collapsed into one row and their
high-water is a max, not a sum. Pre-#517 behavior, not introduced here, but
Phase 2 makes it visible in the ranked total rather than hidden in the daily
merge.

## Phase 3 — Pin the bucket key (the only exact fix)

**The numbering here is misleading and worth saying plainly: this is not third in
sequence.** It is a CLI change with no server dependency, so it can and probably
should ship in parallel with Phases 1 and 2 rather than after them. It is also
the only item that stops the damage from growing while everything else is being
decided. Read the phases as two tracks — server (1, 2, 4) and client (3, 5) —
that meet only at Phase 4.

Every error above exists because the bucket key is derived from a mutable input.
Have the CLI **record its bucketing timezone in the config directory on first
scan and reuse it**, instead of reading `chrono::Local` each time, with
`tokscale config set timezone` to change it deliberately.

That makes local dates stable, which is qualitatively different from making them
coarser:

- **the boundary leak goes to zero** — no re-split ever happens, at any width;
- **swallowing goes to zero** — buckets can safely narrow to daily once a device
  reports a pinned zone, because per-day keys are now stable;
- **it removes the cause**, which no server-side change can do. Phases 1, 2 and 4
  only clean up after a re-split has already happened.

Requires a CLI release, so it lands on an adoption curve, and it does not repair
existing damage. Devices that report a pinned zone can be moved to daily buckets
individually, so the benefit arrives per-device as users upgrade rather than
waiting on full adoption.

Trade-off: a user who relocates keeps bucketing into their old zone until they
change the setting. For historical data that is arguably correct — day boundaries
stay stable — but it is a product decision.

### What it costs, verified (2026-07-31)

This phase was called "cheap" throughout earlier drafts on no evidence. It
mostly is, but via a route those drafts had not identified, and it carries a
dependency decision they never mentioned.

**Already in place, so not part of the cost:**

- Both bucketing functions already have injectable variants —
  `compute_daily_active_time_with_timezone` (`sessionize.rs:291`) and
  `timestamp_to_date_with_timezone` (`sessions/mod.rs:447`), each generic over
  `Tz`. Today's callers simply pass `&chrono::Local`.
- `settings.json` already exists, and adding a field to it is a documented
  drop-in pattern: `#[serde(default)]` so files written before the field still
  load (`tui/settings.rs:141`).
- `ScannerSettings` is a single shared type (`tokscale_core::scanner`, re-used
  by the TUI) and **already flows through the parse path** — `LocalParseOptions`
  carries it at `lib.rs:410`/`:510` and threads it to `:620`, `:1020`, `:2421`,
  `:2478`, `:2577`.
- `compute_daily_active_time` has exactly **one** caller (`lib.rs:2687`).

**The trap.** `timestamp_to_date` is called from `UnifiedMessage::new_full`, a
constructor. Threading a timezone through it means touching every parser:
`UnifiedMessage::new` has **92 call sites across 42 files**. Done naively, this
phase is not cheap at all.

**The cheap route, which exists by luck rather than design.**
`refresh_derived_fields` (`sessions/mod.rs:380`) already recomputes
`self.date` from `self.timestamp` after construction, and it is **already
called in a post-parse pass at `lib.rs:634`** — inside
`parse_all_messages_with_pricing_with_env_strategy`, which already holds
`scanner_settings`. So the date is already treated as a derived field that gets
re-normalised where the settings are in scope. Pinning the zone becomes a
parameter on that pass, not a change to 92 constructor calls.

**The unpriced decision: `chrono-tz` is not a dependency.** The workspace has
`chrono` only (`Cargo.toml:35`). That forces a choice the earlier drafts never
raised:

| | `chrono::FixedOffset` | `chrono-tz` named zone |
|---|---|---|
| new dependency | none | yes, embeds the tz database |
| binary size | unchanged | grows, across all 9 build targets |
| DST correctness | **drifts an hour twice a year** | correct |

A fixed offset reintroduces a bounded version of the very bug this phase
removes: after a DST transition the pinned offset no longer matches local
midnight, so usage within an hour of the boundary lands on the wrong day. It is
far smaller than the current failure — an hour twice a year, not a full re-split
on every rescan — but it is not zero, and calling this "the only exact fix"
above is only true of the named-zone variant.

## Phase 4 — Heal the daily rows

For the day-level surfaces, and only if Phase 1's census says the tail justifies
it.

### Do not put this in the submit path

An earlier draft made the heal a branch inside `submit/route.ts`, deciding
mid-request whether the in-flight payload was authoritative and rewriting rows on
the spot. Every trap catalogued below came from that choice, and they surfaced one
at a time over several readings rather than from any systematic sweep — the
`:735` double-ratchet, which would have made the whole phase a silent no-op,
turned up last. When hazards keep arriving in a code path, the honest conclusion
is that the path is not mapped, not that the list is finally complete.

The submit route is the worst place in this repository to add a conditional
destructive write. It already carries alias folding with a one-shot heal floor,
backfill provenance stamped inside the same JSONB, two independent monotonic
enforcement points, chunked batch inserts, a row lock, and a hash-based identity.
A new branch has to be correct against all of it, at request time, with no way to
review what it is about to do.

**Split it in two instead.**

### 4a — Record what the CLI actually reported (additive, safe)

Submit additionally writes each payload's **unguarded** per-`(device, date,
client)` values to a shadow table:

```
daily_breakdown_reported(submitted_device_id, date, client, tokens, cost,
                         input, output, active_time_ms, origin, reported_at)
PRIMARY KEY (submitted_device_id, date, client)
```

Last-write-wins, no `GREATEST`, no merge, no fold normalisation — it records what
the most recent scan said, which is the one thing the system currently throws
away. This is a pure insert alongside the existing write, inside the same
transaction. It cannot regress anything, because nothing reads it.

Sized at roughly one extra `daily_breakdown`, and bounded the same way: one row
per device per day per client, overwritten rather than accumulated.

### 4b — Reconcile offline

A separate, resumable job — not a request handler — compares shadow against
stored and applies the repair per `(device, client)` when both hold:

1. **Bucket coverage** — the shadow's dates span the bucket being repaired.
   Without it a `--since` scan's smaller total is not comparable to the stored
   high-water for that bucket.
2. **Invariant clears** — the shadow's token total for `C` in that bucket is at
   least the stored high-water.

**Both gates are evaluated per bucket**, and a bucket failing either is skipped
while its neighbours proceed. Checking per bucket, like checking per client,
keeps one shrunken period from blocking every other period's repair.

An earlier draft scoped gate 1 to the whole `(device, client)` date range, which
silently contradicted row 6c below: a device that deleted its sessions and then
kept working fails a range-wide check outright, even though its later buckets
are perfectly comparable. The two gates were at different granularities and the
text never said what a coarse-gate failure did. Per-bucket for both resolves it.

### Why this is materially safer

- **Two of the four traps below stop existing.** The shadow table has no monotonic
  guard, so there is no `:735` arm to bypass and no `mergeActiveTimeMs` to
  neutralise. It stores raw payload values, so the alias-fold machinery and its
  compounding `models` defect never touch it. The backfill constraint survives
  unchanged, and the transaction requirement changes shape rather than going
  away — both are marked as such below.
- **The diff is reviewable before it is applied.** `shadow ⊖ stored` is the census
  at row granularity — strictly better than the bucket ratio in Phase 1, and it
  can be read, sampled, and sanity-checked by a human before a single row changes.
- **The zero-out becomes trivial.** A day present in stored and absent from a
  shadow whose range covers it is unambiguously an emptied day. No inferring
  absence from an in-flight payload.
- **Idempotent, resumable, abortable.** Re-running converges; stopping halfway
  leaves consistent state; a bad batch stops the job instead of failing a user's
  submit.
- **The risky part is no longer on the hot path.** A defect in 4b degrades to "the
  repair did not run", not "submissions are failing".

The cost is one table and a job to operate. Given that the alternative is a
conditional destructive write inside the most interaction-dense function in the
codebase, that is the cheaper side of the trade.

### This is an existing pattern, not new machinery

`foldedClientFloors` (`helpers.ts:158-172`, applied at `:201-217`) already
implements *"a known-inflated stored value may be replaced by a smaller one when
the incoming value clears an invariant lower bound proving the scan was
complete"* — for alias folds:

> nothing proves an incoming submission covers the full day (partial re-parses
> are the exact case the guard exists for), so healing only happens when the
> incoming value is at least the largest single contribution.

Same structure, different axis: fold is client-keys-within-a-day, re-split is
days-within-a-client.

### Zero, do not delete

The route has **no delete path for `daily_breakdown`**, and adding one is the
largest source of risk here. It is also unnecessary: remove `C`'s entry from
`source_breakdown` and let `recalculateDayTotals` (`helpers.ts:68`) recompute. A
day that lands at zero keeps a zero row. `activeDays` already guards with
`COUNT(DISTINCT CASE WHEN tokens > 0 …)` (`:785`), so a zero row does not inflate
it; other consumers must be checked before shipping.

### The zero-out itself is mandatory

**Rewriting the day that gained while the day that lost keeps its old value
reproduces the double count exactly** — a heal without the zero-out repairs
nothing.

This is where the split earns its keep. In the submit path the signal was
ambiguous: the per-day loop visits only days present in the payload (`:632`), and
`aggregate_by_date` emits only days with activity, so an emptied day and a day
outside the scan's scope are indistinguishable at request time. Against the
shadow table the question is decidable — a day present in stored, absent from
shadow, and inside a shadow range that covers it, is unambiguously emptied.

### Assert, then commit

After rewriting `C`, verify `SUM(stored daily for C over the range) == SUM(shadow
for C over the same range)` inside the batch transaction and roll back on
mismatch. This turns a silent corruption into a caught error and a preserved
account — and unlike the in-submit design, a rollback here costs a retry of one
batch rather than a failed user submission.

### Bound the writes

Touch only rows where stored differs from shadow. A re-split changes a small
fraction of a long history, and the comparison is now a plain table diff rather
than something reconstructed per request.

### Interactions — what the split neutralises, and what survives it

These were catalogued against the in-submit design. Recorded here because each is
the reason for a specific property of 4a, and because anyone who proposes
collapsing the split back into the submit path inherits all of them again.

**Alias folds — neutralised by 4a.** The shadow stores raw payload values and
never runs alias normalisation, so none of the following can reach it. It remains
true of the stored rows 4b writes into. A preserved fold must keep its raw alias
keys (`:666-673`) or
"the heal floor is burned on the first partial resubmit and the double count
re-cements permanently". Safe rule: **a client with a fold floor is never
eligible for this heal.**

That rule is now doing more work than originally intended, because the fold path
had a live aliasing defect underneath it. The normalized view copied the models
*map* but left the model *value objects* shared with `rawExistingBreakdown`; an
in-place `+=` merge then mutated those stored objects, and the writeback
persisted the mutated values as the supposedly-raw alias keys. Each submit
re-folded already-folded values, so the nested `models` map compounded without
bound. Client-level `tokens` escaped because the spread copied a scalar, which
is why day totals and the leaderboard stayed correct while per-model views
drifted.

**Fixed in `cdfd1d0c` (#989)** by `cloneClientBreakdownForFold`
(`route.ts:77`, applied at `:122`), which gives the normalized view outright
ownership at the point the sharing is created — covering `models` and
`provenance`, the only two non-scalar fields on `ClientBreakdownData`.

That changes the standing of the exclusion rule rather than removing it. It was
a deferral of an active corruption; it is now an ordinary scope boundary.
Fold-affected rows are no longer degrading while they wait, so excluding them
from the heal costs nothing beyond the heal itself.

**Backfill coexistence — survives the split.** 4b writes into the same
`source_breakdown` the merge path owns, so this constraint is unchanged and must
be honoured by the reconciliation job. `origin: "backfill"` is stamped per client inside
`source_breakdown` (`:623-629`) and carried through merges by
`deriveClientBreakdownProvenance` (`helpers.ts:113-126`). A CLI scan cannot see
imported history, so the rewrite must **preserve `backfill`-tagged entries**. A
payload whose own `provenance.origin` is `backfill` never heals.

**Day-level active time — neutralised by 4a, and the single strongest argument
for the split.** The shadow has no monotonic guard, so there is no arm to bypass.
Under the in-submit design this would have shipped as a silent no-op: it is
ratcheted at TWO enforcement points, and the second one cancels the heal. `mergeActiveTimeMs` (`:206-213`) is a
`Math.max` in the JS merge, and `:735` mirrors it in SQL:

```sql
active_time_ms = GREATEST(daily_breakdown.active_time_ms, EXCLUDED.active_time_ms),
```

whose own comment says the arm "must not be a hole in the monotonic guard the
in-memory merge path applies." A heal writes a *lower* value. Routed through
that `ON CONFLICT` arm, `GREATEST(stale_high, healed_low)` returns the stale
value — **the heal reports success and changes nothing.** No error, no warning;
the post-rewrite assertion above is the only thing that would catch it. Both
enforcement points must be bypassed, not just the JS one.

**Transaction — changed shape under the split.** 4a's shadow write joins the
existing submit transaction, which is the whole of its footprint there. 4b runs
outside the request path and takes its own transaction per batch, so it must
re-acquire the same row lock to avoid racing a live submit for that user. Note
the lock is already stronger than it looks: `:331-342` takes
`.for('update')` on the `submissions` row, which is unique per user
(`submissions_user_id_unique`), so every submit for one user is serialized —
including across devices. A second device cannot commit between the merge's read
and its write, so the gate is never computed against stale data. An audit pass
initially concluded otherwise; the lock is what makes it safe.

## Phase 5 — Declare

Requires a CLI release. Adds `scanScope { parserVersions }` to
`TsTokenContributionData` (`crates/tokscale-cli/src/main.rs:4327`) and extends
`SubmissionProvenanceSchema` (`validation/submission.ts:205`, already optional and
excluded from `generateSubmissionHash`). A client's token decrease is accepted
when its parser version changed and defended when it did not — separating #961's
legitimate re-attribution from a parser regression.

`meta.version` is the CLI version, not a per-client `parser_version`.

## Phase 6 — Compensate (conditional)

Only for devices permanently blocked after a genuine deletion. Adds
`tzOffsetMinutes`; accepts a decrease on day `d` when the declared TZ differs,
`d±1` rose by approximately what `d` fell, and the direction matches. Largely
obviated by Phase 3 for anyone who upgrades. Build only if the census shows the
population is real.

## Behavior for every user state

P2 fixes ranked totals; P3 removes the cause; P4 fixes day-level rows.

| # | User state | P2 | P4 | Outcome |
|---|---|---|---|---|
| 1 | Never submitted | n/a | n/a | Plain insert. |
| 2 | Healthy, stable TZ | correct | no-op | Payload equals stored. |
| 3 | Healthy, multi-device | correct, additive | per-device | `UNIQUE(submission_id, submitted_device_id, date)` scopes every write. |
| 4 | **TZ-inflated, sessions intact** | **fixed to within the boundary leak** | **healed** | P3 stops it recurring. |
| 5 | TZ-inflated, multi-device | fixed | per-device | Each device heals independently. |
| 6 | **Deleted sessions (`d9df8c9c`)** | high-water held | blocked | Protected exactly as today. |
| 6b | Sessions moved where the collector does not scan | held | blocked, temporarily | Self-resolves once support lands. |
| 6c | **Deleted sessions, then kept working** | earlier buckets hold, later buckets grow | later buckets heal | Bucket width bounds how long new work stays invisible. Narrower is better here. |
| 7 | Deleted sessions *and* changed TZ | held | blocked | No loss, no healing. Phase 6. |
| 8 | Retired device | contributes its peak | never runs | Pre-existing, not worsened. |
| 9 | No high-water yet | falls back to stored | blocked | One-submit warm-up. A missing baseline must not be read as `0`. |
| 10 | Legacy device-less CLI | max, not sum, across machines | n/a | Pre-#517 behavior, now visible rather than hidden. |
| 11 | `--client codex` submitter | correct for codex | codex heals | Other clients absent from `submittedClients`, untouched. |
| 12 | `--since` submitter | correct | blocked | Fails range coverage; heals on the next full scan. |
| 13 | Backfill user | **additive** via `origin` | excluded | The `origin` key stops import and CLI overwriting each other. |
| 14 | #961 partial `session_model_usage` | correct | that client blocked | Others still heal. Phase 5. |
| 15 | Parser regression | held | blocked | Correctly defended. |
| 16 | Client with an active alias fold | correct | excluded | Fold heal runs first. |
| 17 | Hidden / moderated user | orthogonal | orthogonal | `leaderboardHidden` affects ranking only. |
| 18 | Alternating TZ daily | fixed to within the leak | heals each scan | P3 eliminates it at the source. |

## Known holes

- **Phase 2 bounds inflation, it does not eliminate it.** The boundary leak
  survives until Phase 3 pins the key or Phase 4 repairs the rows.
- **Filtered *all-time* leaderboards still over-count, independently of all of
  this.** The period boards were fixed in `cb65bbbf` (#988) and `e80b44a3`
  (#991); both now sum only the matching slice through
  `scopeBreakdownToDirectives` (`lib/leaderboard/sourceBreakdown.ts`). The
  all-time path was not, and cannot be repaired the same way: it filters per
  user via `sources_used` (`getLeaderboard.ts:359`) and then sums whole
  `submissions.totalTokens` values, and those columns are bare arrays with no
  per-element token attribution. Scoping it needs a join down to
  `daily_breakdown.source_breakdown`. Listed so nobody validates Phase 2 against
  a filtered all-time board and concludes the derivation is wrong.
- **Swallowing survives inside one bucket.** Bounded by the chosen width; zero
  only after Phase 3 permits daily buckets.
- **`inputTokens` / `outputTokens` stay inflated** until Phase 4. No
  payload-level invariant exists for them.
- **#961 is not healed until Phase 5.**
- **A Phase 4 block means "the token total dropped", not "the user deleted
  something."** Genuine deletion is permanent (6, 7); a collector lagging a
  client's new session location is temporary (6b) —
  [#779](https://github.com/junhoyeo/tokscale/issues/779) is the worked example,
  Codex `archived_sessions` scanned today (`scanner.rs:1389-1395`) after a
  ten-day report-to-fix window. The census must report these separately.
- **A user who stopped using a client** keeps its stale rows: absent from
  `submittedClients`, never rewritten, never zeroed.
- **Cost is recomputed at current pricing** on rewrite, so historical costs shift
  if pricing changed. Tokens are exact; cost is not.
- **Stale comment.** `schema.ts` describes `dailyBreakdown.timestampMs` as "the
  earliest message in this **UTC** day bucket". The bucket is local
  (`sessions/mod.rs:443-444`). Worth correcting — a wrong comment about which
  timezone a bucket uses is precisely the trap this whole issue came from.

## Decision needed

The census settled some of this. Revised standing:

**Ship Phase 3 (pin the bucket key).** 197 of 650 measurable devices show
inflation that scan windowing does not explain, so the mechanism is active and
still producing new damage every time someone rescans from another zone. Phase 3
is CLI-only, has no server dependency, and is the sole change that removes the
cause rather than cleaning up after it. Nothing else here is worth doing first.

**Ship Phase 1 (populate only).** Its justification is now stronger than when it
was written, not weaker. The active-time census answered incidence but not
magnitude, because `total_active_time_ms` only exists on recent CLIs — the 650
visible devices hold ~0.17% of site-wide tokens, so the accounts that actually
matter are invisible to it. Phase 1 writes a per-device **token** high-water on
every submit, which is the only way to measure that population. It reads
nothing, so it cannot regress anything.

**Phases 2, 4, 5 and 6 stay gated**, now on Phase 1's token census specifically
rather than on "a census" generally. Do not build them against the active-time
numbers above; those are an incidence signal on an unrepresentative slice, not a
magnitude estimate.

**Do not size the work from the worst ratios.** The 2300x device is a
partial-scan artifact. Four devices are cleanly artifact, eleven ambiguous.

Still open: whether Phase 2 ships broadly or behind a per-user allowlist
validated against one known inflated account first. That question does not need
answering until Phase 1 reports.

## How to re-derive

| Claim | Command |
|---|---|
| Session metrics already avoid `SUM(daily)`, and why | `sed -n '791,819p' packages/frontend/src/app/api/submit/route.ts` |
| `totalTokens` still uses `SUM(daily)` | `rg -n 'totalTokens' packages/frontend/src/app/api/submit/route.ts` — `:779`, written `:862` |
| Leaderboard reads that column | `rg -n 'submissions.totalTokens' packages/frontend/src/lib/leaderboard/getLeaderboard.ts` |
| Contribution dates are local, not UTC | `sed -n '443,445p' crates/tokscale-core/src/sessions/mod.rs` |
| Device-less submits share a legacy key | `sed -n '182,196p' packages/frontend/src/app/api/submit/route.ts` |
| Guard is per-client; fold heal-floor precedent | `sed -n '154,238p' packages/frontend/src/lib/db/helpers.ts` |
| Only 11 of 45 parsers set `duration_ms` | `rg -l 'duration_ms:\s*Some\|duration_ms =' crates/tokscale-core/src/sessions/ \| wc -l`; `ls crates/tokscale-core/src/sessions/*.rs \| wc -l` |
| Only payload days are visited | `sed -n '632,700p' packages/frontend/src/app/api/submit/route.ts` |
| No delete path exists | `rg -n '\.delete\(' packages/frontend/src/app/api/submit/route.ts` — expect no `dailyBreakdown` hit |
| `activeDays` ignores zero rows | `sed -n '785p' packages/frontend/src/app/api/submit/route.ts` |
| Fold writeback restores raw alias keys | `sed -n '660,674p' packages/frontend/src/app/api/submit/route.ts` |
| Backfill origin is per-client | `sed -n '620,630p' packages/frontend/src/app/api/submit/route.ts` |
| `submittedClients` is the scope set | `sed -n '302,310p' packages/frontend/src/app/api/submit/route.ts` |
| Why the guard exists | `git log -1 d9df8c9c` |
