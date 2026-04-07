# Proposal: Claude Code Agent Tracking in the TUI Agents Tab

**Status:** Draft
**Author:** (pending)
**Date:** 2026-04-08
**Scope:** `crates/tokscale-core/src/sessions/claudecode.rs`, Agents tab messaging (see §7 for the full file-touch list — the scanner is intentionally left unchanged because `WalkDir` already picks up sidechain files)

---

## 1. Problem

The TUI **Agents** tab lists per-agent token/cost/message breakdowns for every supported source, but
**Claude Code contributes zero rows to the tab**. When a user runs tokscale against a machine that
uses Claude Code exclusively, the Agents tab shows the "no agent breakdown is available" empty state
even though Claude Code does heavy subagent work via the Task/Agent tool.

This proposal documents the current parser gap, surveys the real on-disk layouts that exist in the
wild, and specifies a minimal, reviewable plan to populate the Agents tab from Claude Code
transcripts — following the same shape as the existing OpenCode / Codex / RooCode providers.

---

## 2. Current State (as of commit `04207c3`)

### 2.1 Agents tab pipeline

```
scanner.rs                 ~/.claude/projects/**/*.jsonl  (recursive WalkDir)
   ↓
sessions/claudecode.rs     parse_claude_file(path) → Vec<UnifiedMessage>
   ↓
lib.rs (dispatch)          crates/tokscale-core/src/lib.rs:661-687
   ↓
tui/data/mod.rs            aggregate_messages() groups by msg.agent  (lines 259-395)
   ↓
tui/app.rs                 get_sorted_agents()                       (lines 874-908)
   ↓
tui/ui/agents.rs           render()                                  (lines 10-189)
```

Key types:

- `UnifiedMessage.agent: Option<String>` — `crates/tokscale-core/src/sessions/mod.rs:26-42`
- `AgentUsage { agent, clients, tokens, cost, message_count }` — `crates/tokscale-cli/src/tui/data/mod.rs:45-52`
- Normalization: `sessions::normalize_agent_name()` and `normalize_opencode_agent_name()`

Providers currently extracting per-message agent names: **OpenCode, RooCode, KiloCode, Codex**.
All of them populate `UnifiedMessage.agent` via `UnifiedMessage::new_with_agent(...)` or equivalent,
then the aggregation layer groups them into the Agents tab. Adding Claude Code follows the same
pattern — **no trait/registry refactor is needed**.

### 2.2 Why Claude Code produces zero agent rows today

`crates/tokscale-core/src/sessions/claudecode.rs` has two independent gaps:

1. **`ClaudeEntry` struct omits all agent fields.** It deserializes only
   `type`, `timestamp`, `message`, `requestId`. There are no fields for
   `isSidechain`, `agentId`, `sessionId`, `cwd` — the exact fields Claude Code
   uses to mark a subagent transcript.
   ```rust
   // claudecode.rs:17-26
   pub struct ClaudeEntry {
       #[serde(rename = "type")]
       pub entry_type: String,
       pub timestamp: Option<String>,
       pub message: Option<ClaudeMessage>,
       #[serde(rename = "requestId")]
       pub request_id: Option<String>,
   }
   ```

2. **`UnifiedMessage` is always built with `new_with_dedup()`**, which sets
   `agent: None`. The parser never calls `new_with_agent()`, so even a sidechain
   line with a known `agentId` and an on-disk meta file would still land in the
   aggregator with `agent = None`.
   ```rust
   // claudecode.rs:151-166
   let mut unified = UnifiedMessage::new_with_dedup(
       "claude", model, "anthropic",
       session_id.clone(), timestamp, TokenBreakdown { ... }, 0.0, dedup_key,
   );
   ```

### 2.3 What the scanner already does

The scanner at `crates/tokscale-core/src/scanner.rs:102-176` uses `WalkDir::new(root)` and filters
by `*.jsonl` suffix only. That means **subagent files in both known layouts are already being
discovered and fed into `parse_claude_file`** — they are just parsed as if they were regular
assistant sessions, producing token totals that inflate the "claude" client bucket under a
synthetic `session_id` equal to the file stem (e.g. `agent-a48be13e92de1397f`).

Consequences of the current behavior:

- Token totals for Claude Code are **approximately correct** (sidechain API calls are billed
  separately, so parsing sidechain files captures real usage).
- **Session counts are inflated** — each subagent file counts as its own "session".
- **Workspace attribution is correct** — `claude_workspace_from_path()` walks path components
  looking for `.claude/projects/<key>/...`, which still resolves correctly for deeply nested paths.
- **Agent attribution is missing entirely** — the Agents tab is empty for Claude Code users.

---

## 3. Claude Code's On-Disk Reality (verified against `~/.claude/projects/`)

On this machine, both historical and current layouts coexist. Verified counts:

| Layout | Path shape | File count | Meta sidecar? |
|---|---|---|---|
| **Flat (legacy, ≤ ~2.0.x)** | `~/.claude/projects/<encoded-cwd>/agent-<id>.jsonl` | 224 | **No** |
| **Nested (current, ≥ 2.1.x)** | `~/.claude/projects/<encoded-cwd>/<parent-session-uuid>/subagents/agent-<id>.jsonl` | 986 | 591 sidecars (~60%) |

Main sessions (non-sidechain) always live at `~/.claude/projects/<encoded-cwd>/<session-uuid>.jsonl`
in both layouts.

### 3.1 Transcript line schema

Every sidechain line carries (verified against real files):

```jsonc
{
  "parentUuid": "4dbd5659-caf5-4c09-8f86-b0acdc4f5ab4",   // null = first line in the sidechain
  "isSidechain": true,                                    // KEY: distinguishes subagent from main
  "agentId": "a48be13e92de1397f",                         // stable ID within its parent session
  "sessionId": "37b34ca6-c5d7-47ec-bacd-0ae542bc315d",    // KEY: parent session UUID
  "cwd": "/Users/junhoyeo/wrks-sisyphus",                 // workspace path
  "gitBranch": "main",
  "version": "2.1.81",
  "type": "assistant",                                    // or "user"
  "uuid": "b18be654-6773-4324-b8b7-03f5f7e68e93",
  "timestamp": "2026-03-23T03:05:55.664Z",
  "requestId": "req_011CZfU8yGP1k2P72riCg1qS",
  "message": {
    "id": "msg_01HLLR8zNXaMCJbm1M72Zxda",
    "role": "assistant",
    "model": "claude-opus-4-6",
    "usage": {
      "input_tokens": 2,
      "output_tokens": 5805,
      "cache_read_input_tokens": 50526,
      "cache_creation_input_tokens": 4619,
      "cache_creation": { "ephemeral_5m_input_tokens": 4619, "ephemeral_1h_input_tokens": 0 }
    }
  }
}
```

Main session lines are identical in shape but with `isSidechain: false` (or field absent) and no
`agentId`.

### 3.2 Meta sidecar

For the nested layout, Claude Code writes a one-line JSON file next to each sidechain transcript:

```
~/.claude/projects/-Users-junhoyeo-wrks-sisyphus/37b34ca6.../subagents/
├── agent-a48be13e92de1397f.jsonl
└── agent-a48be13e92de1397f.meta.json
```

Meta contents (real example):

```json
{"agentType":"explore","description":"Explore session creation UI"}
```

`agentType` is the **subagent_type** — the canonical name we want in the Agents tab.

### 3.3 Fallback signal: the parent session's tool_use entries

When the meta sidecar is missing (legacy flat files and the ~40% of nested files without a meta),
the subagent_type can still be recovered. Every sidechain file carries its parent's `sessionId` on
every line. The parent session's main JSONL contains an assistant message whose `content[]`
includes a `tool_use` block with:

```jsonc
{ "type": "tool_use", "name": "Agent", "input": { "subagent_type": "explore", "prompt": "..." } }
```

A `grep` across a real parent session file confirmed `subagent_type` values appear in `tool_use`
inputs even when the top-level tool name is `"Agent"` (not `"Task"` — the internal name has
changed in recent versions). A robust implementation should match **tool_use blocks whose input
has a `subagent_type` field**, regardless of the outer tool name.

### 3.4 Token accounting rule

Sidechain API calls are billed as independent requests by the Anthropic API; their usage is recorded
on the sidechain transcript, **not** rolled up into the parent session's messages. This means:

- Summing usage across `<main>.jsonl` + `<main>/subagents/*.jsonl` gives the correct grand total.
- Not reading sidechains would **undercount** Claude Code usage.

The current parser already reads sidechain files (because `WalkDir` is recursive), so grand totals
are already approximately right — this proposal does not change total accounting, only attribution.

---

## 4. Proposal

### 4.1 Design principles

1. **Stay in the existing provider shape.** No new trait, no new registry. Only touch
   `sessions/claudecode.rs`, its tests, and the empty-state message in `ui/agents.rs`.
2. **Preserve token totals.** Current totals are approximately correct; do not regress them.
3. **Correct `session_id`** for sidechain lines so the TUI session count is no longer inflated.
4. **Two-tier agent lookup** (meta sidecar → parent tool_use inference) to handle both layouts
   without requiring users to re-run Claude Code.
5. **Do not break caching.** The existing parser participates in `load_or_parse_source` caching
   at `lib.rs:661-687`. New inputs (meta sidecars) must be tracked so the cache invalidates correctly.

### 4.2 Parser changes

**File:** `crates/tokscale-core/src/sessions/claudecode.rs`

Extend `ClaudeEntry` to capture the linkage fields:

```rust
#[derive(Debug, Deserialize)]
pub struct ClaudeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub message: Option<ClaudeMessage>,
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    // NEW
    #[serde(rename = "isSidechain", default)]
    pub is_sidechain: bool,
    #[serde(rename = "agentId")]
    pub agent_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}
```

In `parse_claude_file`:

1. **Detect sidechain files.** A file is a sidechain when any line has `is_sidechain: true`
   (first-line sniff is sufficient; all lines in a subagent file are marked).
2. **For sidechain files, derive session_id from the transcript**, not the filename:
   ```rust
   let session_id = entry.session_id
       .clone()
       .unwrap_or_else(|| file_stem_fallback());
   ```
   This fixes the inflated session count.
3. **Resolve the agent name** via the cascade:
   ```
   ┌─ Tier 1: sibling meta.json  (agent-<id>.meta.json → agentType)
   ├─ Tier 2: parent session tool_use lookup (see §4.3)
   └─ Tier 3: literal "claude-code-subagent" (keeps the row visible)
   ```
4. **Emit agent-bearing messages** with `UnifiedMessage::new_with_agent(...)` instead of
   `new_with_dedup(...)`. Apply `sessions::normalize_agent_name()` to match the normalization
   every other provider uses.

Pseudocode for the cascade (called once per file, not per line):

```rust
fn resolve_subagent_name(
    path: &Path,
    agent_id: &str,
    parent_session_id: &str,
    parent_index: &ParentSessionIndex, // built lazily, see §4.3
) -> Option<String> {
    // Tier 1: sidecar meta.json
    let meta_path = path.with_extension("meta.json");
    if let Ok(text) = std::fs::read_to_string(&meta_path) {
        if let Ok(meta) = serde_json::from_str::<AgentMetaFile>(&text) {
            return Some(meta.agent_type);
        }
    }

    // Tier 2: parent session tool_use lookup
    if let Some(name) = parent_index.lookup(parent_session_id, agent_id) {
        return Some(name);
    }

    // Tier 3: generic fallback (still visible in Agents tab)
    Some("claude-code-subagent".to_string())
}
```

### 4.3 Parent-session tool_use index (optional, Tier 2)

To recover `subagent_type` for files without a meta sidecar, build a per-run cache keyed by
`(parent_session_id, agent_id) → subagent_type`. The cache is populated lazily the first time a
sidechain file in a given project needs Tier 2.

Construction:

1. Given a sidechain file, locate the parent main session file. In both layouts the parent lives
   at `<encoded-cwd>/<parent_session_id>.jsonl`, i.e.
   `path.ancestors()` until we hit `.claude/projects/<key>/<parent>.jsonl`.
2. Scan the parent JSONL line-by-line. For each assistant message, walk
   `message.content[]` and collect any `tool_use` block whose `input.subagent_type` is a string.
3. The linkage from a tool_use to the `agent-<id>.jsonl` file it spawned is **by position**:
   the Nth Task invocation in the parent session corresponds to the Nth `agent-<id>.jsonl` file
   sorted by first-line timestamp. (A cleaner linkage via `agentId` field would be preferred —
   verify during implementation whether recent transcripts write `agentId` into the parent
   tool_use block; if so, use that instead of positional matching.)
4. Cache the resulting map in a short-lived process-local structure so multiple sidechain files
   in the same project only pay for one parent scan.

**Open question for reviewers:** is positional matching acceptable, or should Tier 2 be dropped
in favor of Tier 3's generic label for files without a meta sidecar? Positional matching is
fragile under interleaved/backgrounded subagents. A conservative v1 could ship Tier 1 + Tier 3
only and leave Tier 2 for a follow-up.

### 4.4 Cache invalidation

`load_or_parse_source` at `crates/tokscale-core/src/lib.rs:661-687` keys the cache by file path
and mtime. When a meta sidecar is added later, the sidechain JSONL itself does not change, so the
cache will not invalidate. Two options:

- **A.** Extend the cache key for Claude Code sidechain files to include the sidecar mtime when
  it exists. Minimal blast radius; requires a Claude-specific branch in the cache key builder.
- **B.** Ignore the issue in v1 and document that users must run `tokscale --no-cache` once after
  upgrading Claude Code to pick up newly-written meta files. Simpler, zero invasive change.

Recommendation: **ship B in v1**, revisit A if users complain.

### 4.5 TUI empty-state message

`crates/tokscale-cli/src/tui/ui/agents.rs:191-205` currently shows a generic empty message when the
Agents tab has no rows. Once Claude Code contributes rows, this message is no longer reached for
Claude-only installs. **No code change needed**, but the "only some sources record agent metadata"
wording should be reviewed in the same PR — we will have added the last major source that was
previously silent, and the wording may mislead users who have other quiet providers.

### 4.6 Data model deltas

**None.** `UnifiedMessage.agent: Option<String>` already exists and is the exact field the
aggregation layer groups on. The normalized agent name flows through `aggregate_messages()` in
`crates/tokscale-cli/src/tui/data/mod.rs:350-395` without any changes to the aggregator.

The `clients` column of `AgentUsage` will now include `"claude"` for Claude Code subagent rows.
If an agent name collides across providers (e.g., a hypothetical `explore` agent existing in both
OpenCode and Claude Code), the existing aggregator already merges them into a single row with
both clients listed — this is the same merge behavior RooCode/KiloCode already rely on.

### 4.7 Tests to add

All new tests live in `crates/tokscale-core/src/sessions/claudecode.rs`'s existing `mod tests`.

1. **Nested layout with meta sidecar** — create a temp `<project>/<session>/subagents/agent-X.jsonl`
   + `agent-X.meta.json`, assert parser emits a message with `agent: Some("explore")` and the
   `session_id` from the transcript body (not the filename).
2. **Nested layout without meta sidecar, Tier 2 hit** — create a parent `<session>.jsonl` with a
   synthesized tool_use block, assert Tier 2 recovers the subagent_type. (Only if Tier 2 ships.)
3. **Flat legacy layout, no meta, no parent** — assert Tier 3 emits a row labeled
   `claude-code-subagent`.
4. **Token totals preserved** — parse a fixture with main + sidechain, sum input/output/cache
   tokens, assert they match the sum of the underlying transcripts.
5. **Session count** — assert that a parent session + three sidechain files collapse to
   **one** `session_id` group in `UnifiedMessage` output.
6. **isSidechain=false main session regression** — confirm no behavior change for plain main
   sessions (all existing tests in the file should continue to pass unchanged).

### 4.8 Out of scope for v1

Things explicitly deferred:

- **Duration tracking** per subagent invocation. The schema supports it (first-line vs last-line
  timestamp) but the Agents tab doesn't have a duration column today.
- **Success/failure flags.** Same reason.
- **Subagent prompt preview.** Would require a new column and a UX decision.
- **Custom `~/.claude/agents/*.md` definitions** as a source of "known agents". Not needed for
  attribution since the transcript carries the name directly.
- **Hooks-based tracking** (`SubagentStop` etc.). Post-hoc JSONL parsing is sufficient and
  requires no user-side configuration.
- **Deduplication across overlapping parent/sidechain files.** Current dedup key is
  `message.id + requestId`, which is globally unique per API call. Sidechain and main session
  lines will never share a requestId, so no new dedup logic is needed.

---

## 5. Risks & Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Token totals regress because sidechain files are now "double-counted" | Low | They are already counted today; we only change attribution. Add test §4.7(4). |
| Agent name cardinality explodes (every unknown hex id becomes a row) | Medium | Tier 3 collapses all unknowns into a single `claude-code-subagent` row; Tiers 1/2 produce stable names. |
| Parent session lookup is slow on huge projects | Medium | Tier 2 is lazy + cached per run. Worst case: drop Tier 2 and ship Tier 1 + Tier 3. |
| Cache doesn't invalidate when meta sidecars appear later | Low | Documented workaround (`--no-cache`) in v1; formal fix in a follow-up. |
| Positional matching in Tier 2 misattributes under parallel subagents | Medium | Tier 2 is optional for v1; gate on verifying the `agentId` field is present in recent parent tool_use blocks before relying on it. |
| Breaking change to `ClaudeEntry` serde surface for downstream crates | Very low | All new fields are `Option`/`default`; no existing field renamed. |

---

## 6. Migration / Rollout

- No user-facing migration. Upgrading tokscale re-parses Claude Code transcripts from disk.
- First run after upgrade will rebuild the source cache (or users opt in via `--no-cache`).
- No schema change to the serialized usage submission format (if any downstream exists) because
  `UnifiedMessage.agent` is already an `Option<String>` field that other providers populate today.

---

## 7. Files Touched (estimated)

| File | Change |
|---|---|
| `crates/tokscale-core/src/sessions/claudecode.rs` | Main parser extension, sidechain handling, meta lookup, tests |
| `crates/tokscale-core/src/sessions/mod.rs` | Possibly expose a small helper if `normalize_agent_name` needs a Claude-specific variant (likely not) |
| `crates/tokscale-cli/src/tui/ui/agents.rs` | (Optional) update empty-state copy |
| `docs/proposals/claude-code-agents.md` | This document |

No changes anticipated to:

- `crates/tokscale-core/src/scanner.rs` (already picks up sidechain files)
- `crates/tokscale-core/src/lib.rs` (parser dispatch unchanged)
- `crates/tokscale-cli/src/tui/data/mod.rs` (aggregator unchanged)
- `crates/tokscale-cli/src/tui/app.rs` (sorting unchanged)
- `crates/tokscale-core/src/clients.rs` (`ClientId::Claude` already registered)

---

## 8. Open Questions for Reviewers

1. **Tier 2 yes/no?** Ship parent-session tool_use inference in v1, or start with Tier 1 + Tier 3
   only and observe how many Claude Code users actually hit the "unknown" bucket?
2. **Cardinality cap.** Should there be a hard cap on the number of distinct agent names surfaced
   per run (e.g., top 100 by token count, everything else collapsed) to avoid runaway rows from
   hex-id fallbacks?
3. **`"claude-code-subagent"` label wording.** Is there a better term that matches the rest of
   the tokscale UX? ("Claude subagent"? "Task (unknown)"?)
4. **Empty-state rewrite.** With Claude Code filled in, is the "only some sources record agent
   metadata" message still accurate? Which providers remain silent? (Crush, Gemini, OpenClaw,
   Amp, Droid, Pi, Kimi, Qwen, Mux — per the survey in §2.1.)
5. **Meta sidecar as cache key input.** Worth the added complexity in v1, or defer?

---

## 9. Evidence Appendix

### 9.1 Real flat-layout sidechain first line (legacy)

From `~/.claude/projects/-Users-junhoyeo/agent-ac0c74c.jsonl` (Claude Code v2.0.67):

```json
{"parentUuid":null,"isSidechain":true,"userType":"external","cwd":"/Users/junhoyeo","sessionId":"9b23a108-3213-4a2e-83a7-71f23ef8096e","version":"2.0.67","gitBranch":"","agentId":"ac0c74c","type":"user","message":{"role":"user","content":"Warmup"},"uuid":"1e4c2ee2-bd30-4761-951d-54ee7ca955b8","timestamp":"2025-12-28T10:20:03.829Z"}
```

No sibling `.meta.json` exists. Parent main session `9b23a108-...jsonl` exists in the same dir.

### 9.2 Real nested sidechain first line (current)

From `~/.claude/projects/-Users-junhoyeo-wrks-sisyphus/37b34ca6-.../subagents/agent-a48be13e92de1397f.jsonl`
(Claude Code v2.1.81):

```json
{"parentUuid":null,"isSidechain":true,"promptId":"8c91b9f0-...","agentId":"a48be13e92de1397f","type":"user","message":{"role":"user","content":"Find the session creation form..."},"uuid":"4b4afd08-...","timestamp":"2026-03-23T03:05:55.664Z","userType":"external","entrypoint":"cli","cwd":"/Users/junhoyeo/wrks-sisyphus","sessionId":"37b34ca6-c5d7-47ec-bacd-0ae542bc315d","version":"2.1.81","gitBranch":"main","slug":"quiet-exploring-pebble"}
```

Sibling meta:

```json
{"agentType":"explore","description":"Explore session creation UI"}
```

### 9.3 Verified counts on this workstation

```
Flat (direct child of projects):    224
Nested (in subagents subdir):        986
Nested meta.json sidecars:           591
```

~60% of nested sidechain files have a meta sidecar. Flat files have none.

### 9.4 Parent session tool_use grep

```
$ grep -o '"subagent_type":"[a-z][^"]*"' <parent_session>.jsonl | head
"subagent_type":"explore"
"subagent_type":"explore"
"subagent_type":"explore"
```

`grep -c '"name":"Task"'` returned 0 on the same file — the tool is nested under a different
outer name (likely `"Agent"`). The implementation should key off the presence of
`input.subagent_type`, not the tool name.

---

## 10. Summary

Claude Code's subagent telemetry is already on disk, already being walked by the scanner, and
already being parsed into `UnifiedMessage` records — the only missing piece is a four-line
extension of `ClaudeEntry` plus a small agent-name lookup in `parse_claude_file`. The Agents
tab pipeline downstream of the parser requires **no changes**. The primary implementation risk
is the Tier 2 parent-session lookup, which can be deferred to a follow-up without blocking a
useful v1.
