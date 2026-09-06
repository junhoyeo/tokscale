# dsh-served-model

The fixture behind the DSH v3 -> v4 change, where usage moved onto the model
the provider reported serving (`source.replayState.response.responseModel`)
instead of the model the request configured (`source.model`). Two DSH
sessions, three assistant calls, one root:

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
`scripts/check-dsh-cache-migration.sh` compares on it.

`current` is what a served-model parser reports; `predecessors.3` is what the
requested-model parser shipped as 4.14.0 reports.

The same root runs a second time against the last published release, where the
released binary and this build normally share a parser version and the released
rows are therefore served rather than reparsed. That is the only leg in the gate
whose warm scan can disagree with a cold one over a *future* missing bump, and
this is the only fixture whose graded figure an attribution change moves — so
the pair is what stops a served-model change from shipping without one.
