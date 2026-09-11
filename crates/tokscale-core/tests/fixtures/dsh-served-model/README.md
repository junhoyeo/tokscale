# dsh-served-model

The fixture behind the DSH v3 -> v4 change, where usage moved onto the model
the provider reported serving (`source.replayState.response.responseModel`)
instead of the model the request configured (`source.model`). Two DSH
sessions, three assistant calls, one project root (both sessions share
`cwd: /fixture`) and two message-tree roots (each session's step-0 message):

- `session-echo` — one call the provider substituted (`fixture-model`
  requested, `fixture-served-model` served) and one it answered as requested,
  so the same transcript straddles the change.
- `session-foxtrot` — a floating request alias (`fixture-alias-model`) that
  resolves to the same concrete `fixture-served-model`, the shape
  `served_model` exists for.

No compaction and no fork: every row carries its own `message.id`, so the key
is `msg:<id>` under every DSH parser version this fixture is used with, and
attribution is the only thing that moves.

**The totals are identical on both sides of the change.** 621 input, 65
output, 6,210 cache read, 15 cache write, 3 messages, whether the calls are
credited to the requested models or the served one — only the split moves.
That is the point: a comparator that checks token buckets alone cannot see
stale model attribution, or the pricing derived from it, survive a migration.
`expected.json` therefore records a per-model breakdown, and
`dsh_predecessor_caches_are_rejected_and_rebuilt_by_the_production_scan`
(in `crates/tokscale-core/src/message_cache.rs`) compares on it.

`current` is what a served-model parser reports; `predecessors.3` is what the
requested-model parser shipped as 4.14.0 reports.

The test seeds this root's transcripts into a cache written under parser
identity 3 — the one 4.14.0 cached under — and then runs the production scan.
Serving those rows reports `predecessors.3`; rejecting them and reparsing
reports `current`. Because only the split moves, this is the fixture that
stops an attribution change from shipping unnoticed, and it keeps doing so for
a *future* change: `current` is frozen here, so a semantics change that does
not bump the parser identity lands as a diff against it rather than moving
both halves of a same-version round trip together.

`predecessors` is a frozen record of what a published binary reported, and its
key is pinned in the test rather than derived from the running parser version.
Rewriting `current` to match changed behaviour without a bump is re-recording
a baseline, not updating a snapshot.
