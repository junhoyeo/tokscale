# Shelley client integration plan

Plan for adding [Shelley](https://shelley.dev) as a tokscale client.

- Branch: `feat/shelley-client`
- Client id: `shelley`
- Status: implemented on this branch — session parsing and registration, the guard against
  schemas the parser cannot read, and the split of a message's direct and indirect halves.
  Where this document and the code disagree, the code is the reference.
- Source of truth for the data model: `/home/exedev/code/shelley/db/ANALYTICS.md` and
  `/home/exedev/code/shelley/db/analytics-views.sql` in the Shelley repo
  (clone at `/home/exedev/code/shelley`, HEAD `5828ea0e`).

## 1. What Shelley stores

Shelley is a Go coding agent. There is no instance-wide usage endpoint, so usage is read
with SQL from `shelley.db`. Two tables carry everything:

- `conversations` — one row per chat session: `conversation_id`, `slug`, `parent_conversation_id`,
  `user_initiated`, `cwd`, `model`, `archived`, `is_draft`, `current_generation`.
- `messages` — append-only, one row per message: `message_id`, `conversation_id`, `sequence_id`,
  `generation`, `type`, `created_at`, `model_name`, `llm_api_url`, `forked_from_message_id`,
  `usage_data`, `other_usage_data`.

Token accounting lives in two JSON columns:

- `usage_data` — one object per agent turn, serialized from `llm.Usage`
  (`llm/llm.go:738`): `input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`,
  `output_tokens`, `cost_usd`, `model`, `url`, `start_time`, `end_time`.
- `other_usage_data` — a JSON **array** of indirect calls (slug generation, compaction,
  LLM-backed tools), each a `PurposedUsage` = the same fields plus `purpose`
  (`llm/llm.go:756`).

Both must be read to see every call.

The columns this parser reads arrive late in Shelley's migration history: `model_name` in 031,
`forked_from_message_id` in 032, `other_usage_data` in 036. That is why a query that fails to
prepare is treated as a schema era rather than a lost read (§6).

## 2. Verified against a live database

Read-only probe of `file:/home/exedev/.config/shelley/shelley.db?mode=ro` (db 462,848 B,
`-wal` 1,413,192 B, `-shm` 32,768 B, `MAX(migration_number) = 48`):

| fact | value |
| --- | --- |
| conversations | 4 (3 roots + 1 subagent) |
| messages | 50 |
| `type = 'agent'` rows with `usage_data` | 17 |
| rows with `other_usage_data` | 3 (all `purpose = slug`) |
| rows with `forked_from_message_id` | 0 |
| `SUM(cost_usd)` over all agent rows | `0.0` |
| distinct `llm_api_url` hosts | 1 — `llm.int.exe.xyz` |

Conversation tree with usage:

| conversation_id | slug | parent | user_initiated | agent rows | input tok | output tok |
| --- | --- | --- | --- | --- | --- | --- |
| cC5RHC5 | test-sum | cN7VONC | 0 | 2 | 8,792 | 57 |
| cRBCPUG | shelley-message-metadata-database | (root) | 1 | 7 | 33,620 | 861 |
| cI3GFJW | concept-of-god | (root) | 1 | 2 | 2,048 | 817 |
| cN7VONC | addition-calculation | (root) | 1 | 6 | 20,326 | 154 |

Five findings that shape the design, each observed rather than assumed:

1. **`usage_data` is non-NULL on rows that are not agent turns.** All 23 non-agent rows
   carrying it (`user` 13, `modelchange` 6, `system` 3, `error` 1) are all-zero. One agent row
   is all-zero too — the subagent's second row (`ab656dc1`, `model_name` NULL,
   `excluded_from_context = 1`). The all-zero filter is load-bearing inside `type = 'agent'`,
   not just outside it.
2. **The main database file's mtime cannot drive cache invalidation.** At probe time the
   `.db` was last modified at 14:07 while `-wal` was 14:25, and rows created at 14:25 were
   readable through a read-only connection. A fingerprint over the main file alone would
   serve stale results for as long as the writer does not checkpoint.
3. **`messages.model_name` and `usage_data.model` agree** (both the wire id, e.g.
   `accounts/fireworks/models/deepseek-v4p1-flash`). `conversations.model` is a *different*
   vocabulary (`deepseek-v4.1-flash-fireworks`) and is mutable — `cN7VONC` read `grok-4.6`
   earlier in the same session and `deepseek-v4.1-flash-fireworks` later. Per-call model
   attribution must come from the message, never the conversation.
4. **The gateway serves several model families from one host.** `llm.int.exe.xyz` fronted
   `accounts/fireworks/models/*`, `grok-4.6`, `gpt-6-luna` and `claude-haiku-5-5`. The endpoint
   host therefore carries no provider identity and must not be used to derive one.
5. **The subagent's usage is not inside the parent's rows.** The child's call is `db89f20b`
   at 14:25:43 (8,792 / 57); the parent's own turn is `7b620e58` at 14:25:41 (10,039 / 79).
   Summing every conversation counts each call exactly once.

## 3. Decisions

| # | Decision | Rationale |
| --- | --- | --- |
| D1 | Register `~/.config/shelley/shelley.db` as the client root; relocations go through `scanner.extraScanPaths.shelley` / `TOKSCALE_EXTRA_DIRS` | Matches the exe.dev image's systemd unit (`-db /home/exedev/.config/shelley/shelley.db`). Shelley defines no env var of its own, so tokscale must not invent one as the primary mechanism. |
| D2 | A message carrying both a direct call and indirect ones becomes **two** tokscale rows: the direct call under `message_id`, the indirect calls under `<message_id>:indirect`. A message carrying one half stays one row. | The halves are separate calls with their own models and their own cost provenance, and one row can only price them one way: whichever half did not report a cost would be billed at the other half's model rate, and a reported amount would be discarded outright. The direct half keeps the message id, the `is_turn_start` flag and the message count; the indirect half is count-neutral, so one Shelley message never counts twice. Same split `droid` and `copilot_desktop` apply to their own fragments. Nothing double counts either way, because indirect-only rows (slug rows) carry `usage_data IS NULL`. |
| D3 | Use `cost_usd` when `> 0` (`CostSource::ProviderReported`), otherwise price locally (`Estimated`) | 0 of 17 agent rows have cost, but 2 of 3 indirect `slug` calls do (5.6e-05, 4.8e-05). The header is real but unreliable per call. A half is provider-reported only when *every* call in it reported an amount; a partially reported half is priced locally, because the engine prices the half's whole token total and adding a reported sub-amount on top would double count. |
| D4 | Keep the raw wire model id; derive `provider_id` from the model id, never the endpoint host | The host is a gateway shared by four families. See §4 for the one subtlety. |
| D5 | `session_id = conversation_id`, `parent_session_id = parent_conversation_id`, `agent = "Shelley Subagent"` when `user_initiated = 0`, `session_title = slug`, workspace from `cwd` | `parent_conversation_id` is a real populated tree, so this is the OMP/pi shape (`pi.rs:777`, `pi.rs:1091`) plus Codex's Agents-tab bucketing (`codex.rs:39`). |
| D6 | Count every conversation with usage, archived and draft alike | Archiving is a UI action; the tokens were billed. The archived conversation holds 7 of 17 agent rows here. |
| D7 | Prefer `usage_data.start_time` over `messages.created_at` | `created_at` has second granularity and lands at call *end* (observed: `created_at` 14:12:45 vs `start_time` 14:12:42.915). Matches the existing "prefer recorded request start timestamps" precedent. |
| D8 | Exclude rows with `forked_from_message_id IS NOT NULL` rather than deduplicating them | A forked copy repeats the original's usage verbatim; the original still carries the call. Deduplication would need a canonical-row rule that the fork semantics do not provide. |
| D9 | A schema that cannot answer a query is reported, never silently read as empty: a pre-036 database reports its direct usage and logs at `debug`, a pre-032 database warns and yields nothing | Every usage statement carries a `what` label, so a failed prepare is visible instead of looking like an empty table. The indirect query is *allowed* to fail on pre-036, because Shelley recorded no indirect usage there at all. A pre-032 database cannot run the direct query either — the fork filter has no column to read — and counting forked copies would double every call in a forked conversation, so refusing is the honest answer. |
| D10 | The cached lane refuses to cache a scan that did not read the database to the end | `scan_shelley_sqlite` returns a `complete` flag. Caching a subset as a result would replay the shortfall on every later scan until the file happened to change. A database that is not Shelley's, or is older than 036, is complete — there was nothing more to read. |

## 4. The one trap: `provider_id` can silently disable provider-scoped pricing

tokscale already resolves Shelley's Fireworks-style ids, but only when the provider hint does
not contradict them.

- `pricing::PricingContext::explicit(model_id, provider_id)` feeds the message's `provider_id`
  into the resolver (`crates/tokscale-core/src/lib.rs:6140`).
- `parse_provider_scoped_model_path` recognises `accounts/<vendor>/(models|routers)/<model>`
  (`crates/tokscale-core/src/pricing/lookup.rs:3929`) and resolves
  `accounts/fireworks/models/deepseek-v4-pro` to the LiteLLM key
  `fireworks_ai/accounts/fireworks/models/deepseek-v4-pro` with
  `ResolutionKind::ProviderScoped` — covered by
  `test_provider_scoped_path_matches_exact_litellm_reseller_key` (`lookup.rs:5281`).
- `provider_hint_matches_scoped_provider` (`lookup.rs:3947`) **rejects** that path when the
  hint's tags do not intersect the scoped vendor's tags, and the three guard sites
  (`lookup.rs:1357`, `1374`, `1407`) then skip provider-scoped resolution entirely.
- `inferred_provider_from_model("accounts/fireworks/models/deepseek-v4p1-flash")` returns the
  model *family*, `deepseek` — not the serving provider. Tags `{deepseek}` vs scoped
  `{fireworks_ai}` do not intersect.

So the naive `resolved_provider(None, model_id, "shelley")` would label these rows `deepseek`
and lose their price. The parser therefore derives `provider_id` in this order, in
`shelley::provider_for_model`:

1. If the model id has the `accounts/<vendor>/(models|routers)/…` shape, use
   `provider_identity::canonical_provider(<vendor>)` — which maps `fireworks` to `fireworks_ai`
   (`provider_identity.rs:62`).
2. Else if the model id is `vendor/model`, canonicalize the leading segment.
3. Else canonicalize `provider_identity::inferred_provider_from_model(model)`. The
   canonicalization is not cosmetic here: inference returns `mistral` for Mistral models while
   the canonical id is `mistralai`, so step 3's result passes through the same canonicalizer
   (`vibe.rs` already does this). If canonicalization fails, the inferred value is used as-is.
4. Else the client id, `shelley` — which prices nothing but labels the row honestly.

Step 1 is load-bearing, and that is confirmed rather than assumed: with the scoped hint
`fireworks_ai` the reseller key resolves, and with the family hint `deepseek` the lookup
returns `None`. Both halves are pinned by
`test_provider_scoped_path_rejects_non_intersecting_family_hint` (`lookup.rs:5360`) and by the
parser's own `provider_for_model_*` unit tests.

The whole rule stays **parser-local**; `provider_identity.rs` is not modified.

## 5. Implementation checklist

Ordered so each step is compile-checked before the next.

| # | File | Change |
| --- | --- | --- |
| 1 | `crates/tokscale-core/src/clients.rs` | Append `Shelley = 58` to `define_clients!` with all nine fields (`clients.rs:1135`). Bump the `ClientId::COUNT` assertion to 59 (`clients.rs:1260`). |
| 2 | `crates/tokscale-core/src/sessions/mod.rs` | `pub mod shelley;` (`mod.rs:58`). |
| 3 | `crates/tokscale-core/src/sessions/shelley.rs` | New parser (see §6). |
| 4 | `crates/tokscale-core/src/scanner.rs` | Add the `"shelley.db"` arm to `scan_directory` (`scanner.rs:760`). **Mandatory, not conditional**: `scan_directory` is a hard-coded `match` whose fallback is `_ => false` (`scanner.rs:772`), with literal arms for `state.db`, `sessions.db` and `threads.db` and no glob fallback for a bare filename. Without the arm the registered pattern matches nothing — at the default root and under an `extraScanPaths` directory alike. WAL/SHM sidecars are already rejected by the existing sidecar logic (`scanner.rs:755`). |
| 5 | `crates/tokscale-core/src/lib.rs` | Wire both lanes. The streaming lane (`lib.rs:3891`) goes through `load_or_parse_source_with_fingerprint_and_policy` (`lib.rs:1665`) with `SourceFingerprint::check_sqlite_path` and `HistoryRetention::LiveFileOnly`, because it needs the `complete` flag that `load_or_parse_sqlite_source` does not surface. The batch lane (`lib.rs:7324`) is inside `parse_local_clients`, which holds no `SourceMessageCache`: every SQLite client there calls its parser directly and its consumers price through `compute_msg_cost` (`crates/tokscale-cli/src/commands/report.rs:1644`). Shelley follows that precedent. Neither lane is compile-enforced. |
| 6 | `crates/tokscale-core/src/message_cache.rs` | `parser_version` arm, `ClientId::Shelley => 1` (`message_cache.rs:2038`). Compile-enforced. |
| 7 | `crates/tokscale-cli/src/main.rs` | `ClientFilter::Shelley` (`main.rs:1201`) plus its three mappings — `as_str`, `to_client_id`, `from_client_id` (`main.rs:1270`, `1342`, `1410`). |
| 8 | `crates/tokscale-cli/src/tui/client_ui.rs` | `ClientUi { hotkey: 'Z' }` (`client_ui.rs:88`). Lowercase `s` is the global "sources" picker binding (`crates/tokscale-cli/src/tui/app.rs:1007`) and uppercase `S` belongs to an existing client, while every other letter of "Shelley" is taken in both cases (`h`/`H`, `e`/`E`, `l`/`L`, `y`/`Y`), so `Z` is the first free uppercase letter after `T`. |
| 9 | `crates/tokscale-cli/src/tui/ui/widgets.rs` | `get_client_color`: `#1D4ED8` (`widgets.rs:878`). Shelley's own `--primary-dark` (`ui/src/styles.css:73`); its `--primary` `#2563eb` is already taken by `workbuddy`. Falls back to gray if omitted, so this is silent. |
| 10 | `packages/frontend/src/lib/types.ts` | Add `"shelley"` to `SUPPORTED_CLIENT_TYPES` (`types.ts:62`). **This gates `POST /api/submit`** — omitting it makes every submission containing a Shelley row fail in full. |
| 11 | `packages/frontend/src/lib/constants.ts` | Add to `SOURCE_DISPLAY_NAMES` (`"Shelley"`, `constants.ts:90`), `SOURCE_LOGOS` (`:162`), `SOURCE_COLORS` (`#1D4ED8`, `:226`). |
| 12 | `.github/assets/client-shelley.png` + `.github/assets/client-shelley.source.md` | Copy `ui/src/assets/icon-512.png` from the Shelley repo unmodified (44,115 B, SHA256 `d2309c9daa7913f569ba2a4e9c8d556f44e42852585023507ad84f2493628eed`) and record that provenance in the sidecar, following `client-aside.source.md`. |
| 13 | `README.md` + `README.ja.md` + `README.ko.md` + `README.zh-cn.md` | Supported-client row, feature list, source-filter list, and the data-location matrix. |
| 14 | `crates/tokscale-core/tests/shelley.rs` | Integration tests over a temp database (§8). |

Guide drift worth noting while touching step 1: `CONTRIBUTING.md`'s `Pi = 8` example omits the
`display` and `logo` fields the macro now requires, and its `ClientUi { display_name, hotkey }`
shape is stale — `ClientUi` is `{ hotkey: char }` (`client_ui.rs:3`).

## 6. Parser specification

Open the database once with `open_readonly_sqlite_opt` (`sessions/utils.rs:310`) and run four
statements against that connection via `sqlite_for_each_row_on` (`sessions/utils.rs:381`).

**A. Direct usage** — `type = 'agent'`, non-NULL `usage_data`, `forked_from_message_id IS NULL`.
The all-zero filter runs in Rust rather than in SQL, so one definition of "billable" covers
both halves: a call is billable when any of the four token fields or `cost_usd` is `> 0`.
`model_name` is NULL on one observed row, so the model falls back to `usage_data.model`.

**B. Conversation metadata** — `conversation_id`, `slug`, `parent_conversation_id`,
`user_initiated`, `cwd`. Joined in Rust by `conversation_id`; no secrets are read, and
`models.api_key` / `settings.cache_session_master_secret` are never selected. An absent
`user_initiated` counts as user-initiated: only an explicit `0` is Shelley's own subagent
signal.

**C. Indirect usage** — `json_each`-equivalent iteration over `other_usage_data`, same fork
filter, summed per `messages` row. `usage_data` is NULL on those rows, so nothing is counted
twice. A row carrying both halves keeps the direct half and appends rather than overwriting it.

**D. User turns** — `conversation_id`, `sequence_id` for `type = 'user'`, used only to decide
`is_turn_start`.

Field mapping:

| Shelley | `UnifiedMessage` |
| --- | --- |
| `message_id` (indirect half: `message_id:indirect`) | `dedup_key` |
| `conversation_id` | `session_id` |
| `conversations.parent_conversation_id` | `parent_session_id` |
| `conversations.slug` | `session_title` |
| `conversations.cwd` | `workspace_key` / `workspace_label` via `normalize_workspace_key` |
| `"Shelley Subagent"` when `user_initiated = 0` | `agent` |
| `usage_data.start_time`, else `created_at` | `timestamp` (`date` derived by `UnifiedMessage::new`) |
| `end_time - start_time` | `duration_ms` |
| `input_tokens` | `tokens.input` |
| `output_tokens` | `tokens.output` |
| `cache_read_input_tokens` | `tokens.cache_read` |
| `cache_creation_input_tokens` | `tokens.cache_write` |
| — | `tokens.reasoning` = 0, `tokens.cache_write_1h` = 0 |
| `cost_usd` when every call in the half reports one, else the pricing engine | `cost` + `cost_source` |
| §4 rule | `provider_id` |
| wire model id | `model_id` |

`TokenBreakdown` is the same four-bucket shape Shelley's Anthropic-style usage has, so the
mapping is direct — `claudecode.rs:1095` maps the identical field set the same way.

Three smaller rules:

- **`is_turn_start`** — true for the lowest-`sequence_id` billable row in its own gap between
  consecutive `user` rows, per conversation. A row that follows no `user` row at all is not a
  turn start: the rule is "the first assistant response after a user turn". Only the primary
  half of a split message can carry it.
- **`message_count`** — the number of calls the half sums (so 1 for a single direct call, N for
  a half summing N entries); 0 for an appended indirect fragment, so a message that splits into
  two rows still counts once.
- **Output determinism** — `HashMap` iteration order is not stable, so message ids are sorted
  before emission and two scans of an unchanged database produce identical vectors.

### Known limitation

Within one half, `other_usage_data` may still name different models, and summing the entries
leaves a single `model_id` for the total. The collapse is resolved as: the half's own model
first (the row's `model_name` for a direct half), then the first entry's. Every row observed on
a real instance carried exactly one entry, so the disagreeing case is only exercised by a
fixture, and the collapse is documented in the parser header.

### Schema eras

`scan_shelley_sqlite` reports whether the database was read to the end alongside the messages,
and each query carries a `what` label so a prepare failure is distinguishable from an empty
table:

| schema | behaviour |
| --- | --- |
| ≥ 036 | Both halves read. |
| 032–035 | No `other_usage_data` column. Shelley recorded no indirect usage in this era, so the indirect query is allowed to fail to prepare; direct usage is still reported and a `debug` line records why. The read is complete. |
| < 032 | No `forked_from_message_id` column, so the direct query cannot answer its own fork filter. Warns and yields nothing rather than counting forked copies twice. The read is **not** complete. |
| not Shelley's database | No `conversations` table. Nothing to read, so the scan is complete and silent — a probe miss, not a fault. |

## 7. Discovery and caching

**Discovery.** `ClientDef` gets `root: PathRoot::Home`,
`relative: ".config/shelley/shelley.db"`, `pattern: "shelley.db"`, `headless: false`,
`parse_local: true`, `submit_default: true`. `PathRoot::Home` rather than `PathRoot::Config`
because the database is not a config-directory file. Shelley hardcodes `~/.config/shelley` for
config only — `AGENTS.md`, skills, hooks and the client socket — while the database comes from
`-db`, which resolves its default against the *server process's* working directory. The
registered path is therefore the layout service deployments use, `~/.config/shelley/shelley.db`
(the exe.dev image's unit points `-db` there), and a database elsewhere is reached by pointing
`scanner.extraScanPaths.shelley` (or `TOKSCALE_EXTRA_DIRS`) at the containing directory, which
the existing extra-path scan then walks for the registered pattern — provided the `"shelley.db"`
arm from §5 step 4 is present, since a file-shaped pattern has no glob fallback.

**Caching.** The fingerprint comes from `SourceFingerprint::check_sqlite_path`
(`message_cache.rs:506`) — the `.db` plus its `-wal` sidecar. This is required, not optional:
finding 2 in §2 shows the main file's mtime stayed 18 minutes stale while fresh rows were being
committed. Read-only open replays the WAL, so committed-but-uncheckpointed turns are visible
(`cherrystudio.rs:648` documents the same behaviour for Cherry Studio). A scan whose `complete`
flag is false is never written to the cache (D10).

## 8. Test plan

Fixture: build a temp SQLite database from the real DDL subset (`conversations`, `messages`,
`migrations`), then assert. Cases in `crates/tokscale-core/tests/shelley.rs`:

1. Direct agent turn → one message, correct four token buckets, `session_id`, `session_title`.
2. All-zero `usage_data` on `user`, `system`, `modelchange`, `error` rows → no messages.
3. All-zero `usage_data` on an `agent` row → no message (the `ab656dc1` case).
4. `forked_from_message_id IS NOT NULL` with duplicated usage → excluded.
5. `other_usage_data` with one entry → summed into the affiliated row, `cost_source` set from
   the entry's `cost_usd`.
6. `other_usage_data` with two entries naming different models → the documented collapse.
7. Subagent conversation (`user_initiated = 0`, `parent_conversation_id` set) →
   `parent_session_id` and `agent = "Shelley Subagent"`.
8. `model_name` NULL with `usage_data.model` set → model still attributed.
9. Provider scoping (§4): `accounts/fireworks/models/<id>` resolves under the scoped vendor
   hint, and the family-inference hint does not.
10. Cost: `cost_usd > 0` → `ProviderReported`; `cost_usd = 0` → `Estimated` from the engine.
11. Cache: touch only the `-wal` file and assert the fingerprint invalidates.
12. Archived and draft conversations are both counted.
13. Discovery: the default root is found with no extra scan paths, and a relocated database is
    found through `extraScanPaths`.
14. A foreign database named `shelley.db` yields nothing rather than erroring.
15. A pre-036 schema still reports the direct calls; a pre-032 schema reports nothing rather
    than counting forked copies twice.
16. A message carrying both call groups becomes two rows, and a partially reported message
    prices each half on its own.

The provider-scoping half of case 9 also lives in the pricing crate, because the trap is
`lookup`'s and not the parser's:
`test_provider_scoped_path_rejects_non_intersecting_family_hint` (`lookup.rs:5360`).
`message_cache.rs`'s `test_shelley_parser_version` (`message_cache.rs:8404`) pins the cache
version arm.

## 9. Verification gates

```
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-features -- -D warnings
cargo test --workspace --all-features
bun run --cwd packages/frontend test -- __tests__/lib/clientRegistry.test.ts
bun run --cwd packages/frontend typecheck
```

Plus a live run against the real database — every command with `--no-spinner`:

```
tokscale --no-spinner --client shelley
```

Expect 19 rows and 175,201 tokens from the instance probed in §2. The 17 `type = 'agent'` rows
include one all-zero row (`ab656dc1`), so 16 are billable; plus 3 indirect-only `slug` rows = 19.
Verified against SQL: 16 billable agent rows, and 174,763 agent tokens + 438 indirect tokens =
175,201. The frontend registry contract test reads `define_clients!` directly and is triggered
by `clients.rs`, so a Rust-only registration fails it.

## 10. Out of scope

- `live_calls` / `live_call_transcripts` (migration 047+) — voice calls carry no token usage.
- `messages_fts` text search and `llm_data.Content[]` parsing — the enums are documented in
  §5 of `ANALYTICS.md` if a later feature needs them.
- Following a `-db` path that points outside the home root without an extra scan path.
- Shelley's bundled models.dev catalog: tokscale prices from its own datasets.

## 11. Resolved questions

| question | answer |
| --- | --- |
| How is the database found? | `~/.config/shelley/shelley.db`, relocations via extra scan paths (D1, §7). |
| Are indirect calls counted? | Yes — summed within their own half, as a separate tokscale row when the message also carries a direct call (D2). |
| Which cost source? | Provider-reported when every call in the half reports an amount, else estimated (D3). |
| Which model string? | The raw wire id from the message, with a scoped-vendor provider hint (D4, §4). |
| Subagent sessions? | Separate session, `parent_session_id` set, `agent` bucket label (D5). |
| Archived and draft conversations? | Counted (D6). |
| Forked copies? | Excluded by `forked_from_message_id`, never deduplicated (D8). |
| A database too old to answer a query? | Reported as a schema era, never read as empty; pre-032 yields nothing and is not cached (D9, D10). |
| Mixed-model `other_usage_data`? | Collapsed to the half's own model, else the first entry's; documented in the parser header (§6). |
| Does §4 need a shared helper in `provider_identity.rs`? | No — the rule is parser-local and that file is untouched (§4, §5). |
| Where does this plan live? | `docs/shelley-client-plan.md` on `feat/shelley-client`. |

Nothing is open. Every question this design raised has a decided answer, and the two that were
left to implementation time — the mixed-model collapse and the helper placement — were settled
in the code rather than deferred.
