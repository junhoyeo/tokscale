# dsh-seq-key

The fixture behind [#1187](https://github.com/junhoyeo/tokscale/issues/1187)
and [#1235](https://github.com/junhoyeo/tokscale/pull/1235). Four DSH sessions,
two legs, one root:

- **Leg A** — `session-alpha` and `session-bravo`: two unrelated sessions, no
  `parentSession`, no `seedLength`, whose `compaction/summary` events agree on
  `seq`, `time`, provider, model and every usage bucket, and differ only in
  the two per-call ids, `compactionId` and `sourceCommandId`. Two separately
  billed summarize calls.
- **Leg B** — `session-charlie` and `session-delta`: a parent and a fork whose
  header lost `seedLength`, the child's prefix repeating the parent's summary
  verbatim. One billed call. This is the case the `seq:` fallback exists for
  and it must keep collapsing.

`expected.json` carries both outcomes, per model and in total. `current` is
what a `compactionId`-keyed parser reports (both legs right);
`predecessors.4` is what the `seq`-keyed parser shipped as 4.15.0 reports (leg
B right, one of leg A's two calls dropped, 3,415 tokens).

Consumed by `scripts/check-dsh-cache-migration.sh`, which runs a pinned 4.15.0
and the current build over this root, cold and warm, and fails when a cache
4.15.0 wrote is served by this build rather than reparsed. The same root runs
a second time against the last published release. Only that run's leg A goes
ungraded — a release that moves has no baseline to hold it to. Its warm leg is
still compared against a cold scan, and because the last release and this
build normally share a parser version, that comparison is the one that fails
when the *next* bump is missing: the released rows are served there rather
than reparsed. This fixture reports a single model, so what it can see is a
change that moves a token total; `dsh-served-model` runs the same leg for
changes that move only attribution.

The transcripts and their totals are built by `build_fixture.py` in
[token-accounting-conformance/tokscale-dsh-seq-key-check](https://github.com/lizhuojunx86/token-accounting-conformance/tree/main/tokscale-dsh-seq-key-check),
which derives them by arithmetic before writing anything; they are not tuned
to any binary. `expected.json`'s layout is this repo's — the gate reads
`current` and `predecessors`, and a per-model split the generator does not
emit.
