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

Consumed by two tests in `crates/tokscale-core/src/message_cache.rs`.
`dsh_predecessor_caches_are_rejected_and_rebuilt_by_the_production_scan` seeds
this root's transcripts into a cache written under parser identity 4 — the one
4.15.0 cached under — and runs the production scan over it. The scan has to
reject those rows, re-cache them under the running identity, and report
`current`; serving them reports `predecessors.4`, and the test asserts the two
figures differ so a baseline that stopped being able to fail says so.
`dsh_cache_rows_are_served_while_the_parser_identity_matches` runs the same
root the other way: a cache written under the *running* identity must be
served, which is what separates that rejection from a cache that discards
every entry and reparses. This fixture reports a single model, so what it can
see is a change that moves a token total; `dsh-served-model` covers changes
that move only attribution.

The transcripts and their totals are built by `build_fixture.py` in
[token-accounting-conformance/tokscale-dsh-seq-key-check](https://github.com/lizhuojunx86/token-accounting-conformance/tree/main/tokscale-dsh-seq-key-check),
which derives them by arithmetic before writing anything; they are not tuned
to any binary. `expected.json`'s layout is this repo's — the tests read
`current` and `predecessors`, and a per-model split the generator does not
emit.

`predecessors` is a frozen record of what a published binary reported, and
its key is pinned in the test rather than derived from the running parser
version. Bumping DSH's parser identity again means adding the new
predecessor's figure here; rewriting `current` to match changed behaviour
without a bump is re-recording a baseline, not updating a snapshot.
