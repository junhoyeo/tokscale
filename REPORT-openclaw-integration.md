# How Tokscale Integrates OpenClaw: A Stateful JSONL Parser for a Four-Name Client

> **Date**: March 3, 2026
> **Sources**: Git history, GitHub releases/PRs, codebase analysis

---

## Table of Contents

- [Executive Summary](#executive-summary)
- [1. What is OpenClaw?](#1-what-is-openclaw)
- [2. How OpenClaw Was Added](#2-how-openclaw-was-added)
- [3. Data Format Deep Dive](#3-data-format-deep-dive)
  - [3.1 Event Types](#31-event-types)
  - [3.2 Token Fields](#32-token-fields)
  - [3.3 Legacy Index Format](#33-legacy-index-format)
- [4. Parser Architecture](#4-parser-architecture)
  - [4.1 Entry Points](#41-entry-points)
  - [4.2 Stateful Parsing Loop](#42-stateful-parsing-loop)
  - [4.3 Session ID Resolution](#43-session-id-resolution)
  - [4.4 Timestamp Fallback](#44-timestamp-fallback)
- [5. Client Registry Entry](#5-client-registry-entry)
- [6. Scanner: Four Paths for One Client](#6-scanner-four-paths-for-one-client)
  - [6.1 The Rebrand History](#61-the-rebrand-history)
  - [6.2 How It Works in Code](#62-how-it-works-in-code)
- [7. Integration Surface Across the Codebase](#7-integration-surface-across-the-codebase)
- [8. How OpenClaw Differs from Other Clients](#8-how-openclaw-differs-from-other-clients)
  - [8.1 Stateful vs Self-Contained](#81-stateful-vs-self-contained)
  - [8.2 Comparison Table](#82-comparison-table)
  - [8.3 Key Architectural Points](#83-key-architectural-points)
- [9. Test Coverage](#9-test-coverage)
- [10. References](#10-references)

---

## Executive Summary

[OpenClaw](https://openclaw.ai/) is a terminal-based AI coding agent that has undergone four name changes: **Clawd → Moltbot → Moldbot → OpenClaw**. Tokscale has supported it since [v1.2.0](https://github.com/junhoyeo/tokscale/releases/tag/v1.2.0) (Jan 30, 2026), added via [PR #139](https://github.com/junhoyeo/tokscale/pull/139) and announced with the 🦞 lobster emoji.

OpenClaw is architecturally unique among tokscale's 14 supported clients for two reasons:

1. **Stateful parsing** — It is the only client that uses `model_change` events to set context for subsequent messages. All other clients embed model metadata directly in each message.
2. **Four scan paths** — Tokscale scans `~/.openclaw/`, `~/.clawdbot/`, `~/.moltbot/`, and `~/.moldbot/` to unify usage history across all rebrands. No other client requires more than two scan paths.

The parser implementation lives in a single 362-line Rust file ([`openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs)) with 8 unit tests covering model changes, user message filtering, missing model context, session index parsing, and filename-derived session IDs.

---

## 1. What is OpenClaw?

OpenClaw is a terminal-based AI coding agent — similar in concept to Claude Code, Codex CLI, or Gemini CLI, but developed independently. It stores session data as JSONL files under `~/.openclaw/agents/`.

The project has been renamed several times:

| Name | Directory | Era |
|------|-----------|-----|
| **Clawd** | `~/.clawdbot/` | Original |
| **Moltbot** | `~/.moltbot/` | First rebrand |
| **Moldbot** | `~/.moldbot/` | Second rebrand |
| **OpenClaw** | `~/.openclaw/` | Current (since at least Jan 2026) |

Each rename changed the tool's data directory, but the JSONL format remained the same. Users who installed and used the tool under any of these names have session files in the corresponding directory.

---

## 2. How OpenClaw Was Added

OpenClaw support landed in tokscale v1.2.0 as a flagship release:

| Detail | Value |
|--------|-------|
| **PR** | [#139](https://github.com/junhoyeo/tokscale/pull/139) |
| **Version** | v1.2.0 |
| **Release date** | January 30, 2026 |
| **Release** | [v1.2.0](https://github.com/junhoyeo/tokscale/releases/tag/v1.2.0) |
| **Announcement** | 🦞 `tokscale@v1.2.0` is here! (Now supports [OpenClaw](https://github.com/openclaw/openclaw)) |

The v1.2.0 release title included the 🦞 lobster emoji, signaling it as a major client addition. The legacy path support (Clawd/Moltbot/Moldbot) was introduced in commit [`6659dde`](https://github.com/junhoyeo/tokscale/commit/6659dde).

---

## 3. Data Format Deep Dive

### 3.1 Event Types

OpenClaw sessions are JSONL files (one JSON object per line) with a **stateful event stream** model. There are two event types:

**`model_change`** — Sets the active model and provider for subsequent messages:

```jsonl
{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
```

**`message`** — Records an interaction with token usage:

```jsonl
{"type":"message","message":{"role":"assistant","usage":{"input":1660,"output":55,"cacheRead":108928,"cost":{"total":0.02}},"timestamp":1769753935279}}
```

Only messages with `role: "assistant"` are counted. User messages are skipped entirely.

The statefulness is the defining characteristic: a `model_change` event sets context that applies to **all subsequent `message` events** until the next `model_change`. This means:

```jsonl
{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
{"type":"message","message":{"role":"assistant","usage":{"input":100,"output":50},"timestamp":1700000000000}}
{"type":"message","message":{"role":"assistant","usage":{"input":200,"output":100},"timestamp":1700000001000}}
{"type":"model_change","provider":"anthropic","modelId":"claude-3.5-sonnet"}
{"type":"message","message":{"role":"assistant","usage":{"input":300,"output":150},"timestamp":1700000002000}}
```

Results in:
- Message 1 → `gpt-5.2` via `openai-codex` (100 in / 50 out)
- Message 2 → `gpt-5.2` via `openai-codex` (200 in / 100 out)
- Message 3 → `claude-3.5-sonnet` via `anthropic` (300 in / 150 out)

If a `message` event appears before any `model_change`, it is **silently dropped** (no model context to assign). This is tested in [`test_parse_openclaw_session_no_model_change`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs).

### 3.2 Token Fields

| Tokscale Field | OpenClaw JSON Key | Notes |
|----------------|-------------------|-------|
| `input` | `usage.input` | Input tokens consumed |
| `output` | `usage.output` | Output tokens generated |
| `cache_read` | `usage.cacheRead` | Cached input tokens read |
| `cache_write` | `usage.cacheWrite` | Tokens written to cache |
| `reasoning` | N/A | Hardcoded to `0` — OpenClaw format does not include reasoning tokens |
| `cost` | `usage.cost.total` | Present in file but **recalculated** by tokscale via LiteLLM for cross-client consistency |

All token values default to `0` if missing and are clamped to non-negative via `.max(0)`:

```rust
// crates/tokscale-core/src/sessions/openclaw.rs, lines 197-202
TokenBreakdown {
    input: usage.input.unwrap_or(0).max(0),
    output: usage.output.unwrap_or(0).max(0),
    cache_read: usage.cache_read.unwrap_or(0).max(0),
    cache_write: usage.cache_write.unwrap_or(0).max(0),
    reasoning: 0,
}
```

The `totalTokens` field is deserialized but unused (`#[allow(dead_code)]`).

### 3.3 Legacy Index Format

In addition to direct JSONL transcript parsing, the parser supports a **`sessions.json` index file** — a JSON map of session entries:

```json
{
  "agent:main:main": {
    "sessionId": "abc-123",
    "sessionFile": "/absolute/path/to/session.jsonl"
  },
  "agent:feature:branch": {
    "sessionId": "def-456",
    "sessionFile": "relative-session.jsonl"
  }
}
```

The `sessionFile` field can be:
- **Absolute path** — used as-is
- **Relative path** — resolved relative to the directory containing `sessions.json`
- **Missing/empty** — falls back to `{sessionId}.jsonl` in the same directory

This is handled by `resolve_session_path()`:

```rust
// crates/tokscale-core/src/sessions/openclaw.rs, lines 102-119
fn resolve_session_path(index_dir: &Path, entry: &SessionEntry) -> PathBuf {
    match entry.session_file.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(session_file) => {
            let path = Path::new(session_file);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                index_dir.join(path)
            }
        }
        None => index_dir.join(format!("{}.jsonl", entry.session_id)),
    }
}
```

---

## 4. Parser Architecture

### 4.1 Entry Points

The parser exposes two public functions:

| Function | Input | Use Case |
|----------|-------|----------|
| `parse_openclaw_transcript(path)` | Direct `.jsonl` file path | Primary path — scanner finds `*.jsonl` files |
| `parse_openclaw_index(path)` | `sessions.json` index file | Legacy compatibility — resolves referenced session files |

Both ultimately call the internal `parse_openclaw_session(path, session_id)`.

### 4.2 Stateful Parsing Loop

The core parser ([`parse_openclaw_session`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs#L121-L212)) maintains two pieces of mutable state:

```rust
let mut current_model: Option<String> = None;
let mut current_provider: Option<String> = None;
```

It reads the file line-by-line via `BufReader` and dispatches on `entry_type`:

```rust
match entry.entry_type.as_str() {
    "model_change" => {
        // Update current_model and current_provider
        if let Some(model) = entry.model_id {
            current_model = Some(model);
        }
        if let Some(provider) = entry.provider {
            current_provider = Some(provider);
        }
    }
    "message" => {
        // Only process assistant messages with usage data and a known model
        // Skip if: role != "assistant", no usage, or no model_change seen yet
    }
    _ => {} // Unknown types silently ignored
}
```

The parser uses `simd_json` for high-performance deserialization, reusing a byte buffer across lines:

```rust
let mut buffer = Vec::with_capacity(4096);
// ...
buffer.clear();
buffer.extend_from_slice(trimmed.as_bytes());
let entry: OpenClawEntry = match simd_json::from_slice(&mut buffer) { ... };
```

### 4.3 Session ID Resolution

- **Direct transcript**: Session ID is derived from the filename stem (e.g., `my-session-123.jsonl` → `"my-session-123"`)
- **Index-based**: Session ID comes from the `sessionId` field in `sessions.json`

### 4.4 Timestamp Fallback

If a message lacks a `timestamp` field, the parser falls back to the file's modification time:

```rust
let file_mtime_ms = std::fs::metadata(session_path)
    .and_then(|m| m.modified())
    .ok()
    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    .map(|d| d.as_millis() as i64)
    .unwrap_or(0);

// Later, during message processing:
let timestamp = msg.timestamp.unwrap_or(file_mtime_ms);
```

This is unique to OpenClaw — other clients either always have timestamps or use other fallback strategies.

---

## 5. Client Registry Entry

OpenClaw is registered as `ClientId::OpenClaw = 7` in the centralized `define_clients!` macro:

```rust
// crates/tokscale-core/src/clients.rs, lines 167-174
OpenClaw = 7 => {
    id: "openclaw",
    root: PathRoot::Home,
    relative: ".openclaw/agents",
    pattern: "*.jsonl",
    headless: false,
    parse_local: true
},
```

Key properties:
- **`root: PathRoot::Home`** — Base directory is `$HOME`
- **`relative: ".openclaw/agents"`** — Primary scan directory
- **`pattern: "*.jsonl"`** — Matches all JSONL files recursively
- **`headless: false`** — No headless/CI mode support
- **`parse_local: true`** — Parser handles local file parsing (vs. API-synced like Cursor)

---

## 6. Scanner: Four Paths for One Client

### 6.1 The Rebrand History

OpenClaw has the most scan paths of any client in tokscale — **4 directories**, all scanned as `ClientId::OpenClaw`:

```
~/.openclaw/agents/**/*.jsonl     ← current (OpenClaw)
~/.clawdbot/agents/**/*.jsonl     ← legacy (Clawd)
~/.moltbot/agents/**/*.jsonl      ← legacy (Moltbot)
~/.moldbot/agents/**/*.jsonl      ← legacy (Moldbot)
```

For comparison:
- Most clients have **1** scan path
- Codex has **3** (sessions + archived_sessions + headless)
- RooCode and KiloCode have **2** each (local + vscode-server)
- OpenClaw has **4** — the most of any client

All four paths produce files attributed to `ClientId::OpenClaw`. A user who started with Clawd, upgraded through Moltbot and Moldbot, and now uses OpenClaw will see their entire usage history unified under a single "OpenClaw" client in the TUI.

### 6.2 How It Works in Code

From [`crates/tokscale-core/src/scanner.rs` lines 226–256](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/scanner.rs#L226-L256):

```rust
if enabled.contains(&ClientId::OpenClaw) {
    // OpenClaw transcripts: ~/.openclaw/agents/**/*.jsonl
    let openclaw_path = ClientId::OpenClaw.data().resolve_path(home_dir);
    tasks.push((ClientId::OpenClaw, openclaw_path, ClientId::OpenClaw.data().pattern));

    // Legacy paths (Clawd -> Moltbot -> OpenClaw rebrand history)
    let clawdbot_path = format!("{}/.clawdbot/agents", home_dir);
    tasks.push((ClientId::OpenClaw, clawdbot_path, ClientId::OpenClaw.data().pattern));

    let moltbot_path = format!("{}/.moltbot/agents", home_dir);
    tasks.push((ClientId::OpenClaw, moltbot_path, ClientId::OpenClaw.data().pattern));

    let moldbot_path = format!("{}/.moldbot/agents", home_dir);
    tasks.push((ClientId::OpenClaw, moldbot_path, ClientId::OpenClaw.data().pattern));
}
```

All four scan tasks use the same pattern (`*.jsonl`) and the same client ID (`ClientId::OpenClaw`). The scans run in parallel via Rayon (`into_par_iter`), and results are merged into a single `Vec<PathBuf>` in the `ScanResult`.

---

## 7. Integration Surface Across the Codebase

OpenClaw is referenced in **10 files** across the Rust core, CLI, and frontend:

### Core (Rust)

| File | Role | Key Detail |
|------|------|------------|
| [`crates/tokscale-core/src/sessions/openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs) | Parser | 362 lines, 8 tests, stateful `model_change` processing |
| [`crates/tokscale-core/src/clients.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/clients.rs) | Registry | `ClientId::OpenClaw = 7`, path `~/.openclaw/agents`, pattern `*.jsonl` |
| [`crates/tokscale-core/src/scanner.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/scanner.rs) | Scanner | 4 scan paths (lines 226–256), parallel Rayon execution |

### CLI (Rust)

| File | Role | Key Detail |
|------|------|------------|
| [`crates/tokscale-cli/src/main.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-cli/src/main.rs) | CLI flags | `--openclaw` filter flag for all report commands |
| [`crates/tokscale-cli/src/tui/client_ui.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-cli/src/tui/client_ui.rs) | TUI display | Display name `"OpenClaw"`, hotkey `'8'` |
| [`crates/tokscale-cli/src/commands/wrapped.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-cli/src/commands/wrapped.rs) | Wrapped image | Logo URL for year-in-review image generation |

### Frontend (TypeScript)

| File | Role | Key Detail |
|------|------|------------|
| [`packages/frontend/src/lib/types.ts`](https://github.com/junhoyeo/tokscale/blob/main/packages/frontend/src/lib/types.ts) | Type definitions | `ClientType` union includes `"openclaw"` |
| [`packages/frontend/src/lib/constants.ts`](https://github.com/junhoyeo/tokscale/blob/main/packages/frontend/src/lib/constants.ts) | UI constants | Display name, logo path, color `#EF4444` (red) |
| [`packages/frontend/src/lib/validation/submission.ts`](https://github.com/junhoyeo/tokscale/blob/main/packages/frontend/src/lib/validation/submission.ts) | Validation | Valid client allowlist for leaderboard submissions |

---

## 8. How OpenClaw Differs from Other Clients

### 8.1 Stateful vs Self-Contained

The fundamental architectural distinction is **how model identity is tracked**:

- **All other JSONL clients** (Claude Code, Codex, Pi, Kimi, Qwen): Each message carries its own `model` field. The parser can process any line independently.
- **OpenClaw**: Model identity is set by `model_change` events and persists across subsequent `message` events. The parser must process lines sequentially and maintain state.

This means OpenClaw's parser **cannot** be parallelized at the line level (though file-level parallelism via Rayon is used). It's a stream parser, not a map-reduce parser.

### 8.2 Comparison Table

| Aspect | OpenClaw | Claude Code | Codex CLI | Gemini CLI |
|--------|----------|-------------|-----------|------------|
| **Format** | JSONL with stateful `model_change` events | JSONL with per-message `model` field | JSONL with `token_count` events | JSON with message arrays |
| **Model tracking** | Stateful — `model_change` sets context for subsequent messages | Per-message `model` field | Per-event model field | Per-message `model` field |
| **Cost in file** | `usage.cost.total` (recalculated by tokscale) | No | No | No |
| **Cache tokens** | `cacheRead`, `cacheWrite` | `cache_read_input_tokens` | No | `cached` |
| **Reasoning tokens** | No | No | No | `thoughts` |
| **Legacy paths** | 3 legacy paths (Clawd, Moltbot, Moldbot) | None | `archived_sessions/` | None |
| **Headless support** | No | No | Yes | No |
| **Provider field** | Explicit (from `model_change`) | Inferred | Inferred | Implicit |
| **Dedup mechanism** | None | `dedup_key` | None | None |
| **Session discovery** | Filename stem or `sessions.json` index | Directory-based | Directory-based | Project hash |
| **Timestamp fallback** | File mtime (if `timestamp` missing) | None needed | None needed | None needed |

### 8.3 Key Architectural Points

1. **Stateful parsing** — OpenClaw is the only client that uses a stateful stream. If a `message` event arrives before any `model_change`, it's silently dropped (no model to assign). This is by design — the parser is conservative and never guesses.

2. **No deduplication** — Unlike Claude Code (which uses `dedup_key` to avoid counting the same usage twice) and OpenCode (which deduplicates via SQLite), OpenClaw has no built-in dedup mechanism. The parser trusts that each JSONL line represents a unique event.

3. **Cost recalculation** — OpenClaw is the only client that embeds cost in its session files (`usage.cost.total`). However, tokscale reads this value and then **recalculates** it via LiteLLM pricing to maintain cross-client consistency. The file-embedded cost serves as a sanity check reference but is not used in the final report.

4. **No agent tracking** — The `agent` field is always `None` for OpenClaw messages. The JSONL format includes an `id` field per entry, but this is not a persistent agent identity — it's a line identifier. The parser does not extract or use it.

5. **Provider explicitness** — OpenClaw is one of the few clients with an explicit `provider` field (set via `model_change`). Most other clients either infer the provider from the model name or don't track it at all. If no provider is set, the parser defaults to `"unknown"`.

---

## 9. Test Coverage

The parser has 8 unit tests in [`openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs#L214-L362):

| Test | What It Validates |
|------|-------------------|
| `test_parse_openclaw_session_with_model_change` | Basic flow: `model_change` → `message` → correct model, provider, tokens, cost |
| `test_parse_openclaw_session_user_messages_ignored` | User role messages are skipped; only assistant counted |
| `test_parse_openclaw_session_no_model_change` | Messages before any `model_change` are dropped (returns empty) |
| `test_parse_openclaw_transcript_derives_session_id_from_filename` | `my-session-123.jsonl` → session ID `"my-session-123"` |
| `test_parse_openclaw_index_absolute_session_file` | Index with absolute `sessionFile` path → resolves correctly |
| `test_parse_openclaw_index_relative_session_file` | Index with relative `sessionFile` → joins with index directory |
| `test_parse_openclaw_index_missing_session_file_fallback` | Index without `sessionFile` → falls back to `{sessionId}.jsonl` |

The scanner also has an OpenClaw-specific test:

| Test (scanner.rs) | What It Validates |
|-------------------|-------------------|
| `test_scan_all_clients_openclaw_jsonl_only` | Scans `~/.openclaw/agents/` and finds only `*.jsonl` files (not `sessions.json`) |

---

## 10. References

### PRs and Commits

| Reference | Description |
|-----------|-------------|
| [PR #139](https://github.com/junhoyeo/tokscale/pull/139) | Original OpenClaw support (v1.2.0) |
| [v1.2.0 Release](https://github.com/junhoyeo/tokscale/releases/tag/v1.2.0) | 🦞 Release announcement |
| [`6659dde`](https://github.com/junhoyeo/tokscale/commit/6659dde) | Legacy path support (Clawd/Moltbot/Moldbot) |
| [PR #237](https://github.com/junhoyeo/tokscale/pull/237) | `define_clients!` macro (centralized client registry) |
| [PR #230](https://github.com/junhoyeo/tokscale/pull/230) | `source` → `client` terminology rename |

### Source Files

| File | Lines | Purpose |
|------|-------|---------|
| [`crates/tokscale-core/src/sessions/openclaw.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/sessions/openclaw.rs) | 362 | Parser implementation + 8 tests |
| [`crates/tokscale-core/src/clients.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/clients.rs) | 392 | Client registry (`ClientId::OpenClaw = 7`) |
| [`crates/tokscale-core/src/scanner.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-core/src/scanner.rs) | 851 | File scanner (4 OpenClaw paths at lines 226–256) |

### External

| Resource | URL |
|----------|-----|
| OpenClaw | https://openclaw.ai/ |
| Tokscale GitHub | https://github.com/junhoyeo/tokscale |
| LiteLLM (pricing source) | https://github.com/BerriAI/litellm |

---

*Generated from codebase analysis, git history, and PR review. All file paths, line numbers, and code snippets verified against the tokscale repository.*
