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

`expected.json` carries both outcomes. `correct` is what a `compactionId`-keyed
parser reports (both legs right); `seq_keyed` is what a `seq`-keyed one reports
(leg B right, one of leg A's two calls dropped, 3,415 tokens).

Consumed by `scripts/check-dsh-cache-migration.sh`, which runs the last
published release and the current build over this root, cold and warm, and
fails when a cache the previous release wrote is served by this build rather
than reparsed.

Built by `build_fixture.py` in
[token-accounting-conformance/tokscale-dsh-seq-key-check](https://github.com/lizhuojunx86/token-accounting-conformance/tree/main/tokscale-dsh-seq-key-check),
which derives the expected totals by arithmetic before writing anything.
Copied verbatim; the numbers are not tuned to any binary.
