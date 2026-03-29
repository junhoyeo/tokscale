# Tokscale v2: From TypeScript/NAPI Hybrid to Pure Rust Binary

> **Date**: March 3, 2026
> **Sources**: Git history, GitHub releases/PRs, codebase analysis, npm registry

---

## Table of Contents

- [Executive Summary](#executive-summary)
- [1. Version Timeline](#1-version-timeline)
  - [1.1 Complete Release History](#11-complete-release-history)
  - [1.2 The Ratatui Rewrite Branch](#12-the-ratatui-rewrite-branch)
- [2. Architecture Comparison](#2-architecture-comparison)
  - [2.1 Runtime Model](#21-runtime-model)
  - [2.2 Crate / Package Structure](#22-crate--package-structure)
  - [2.3 TUI Framework](#23-tui-framework)
  - [2.4 Rust-to-JavaScript Interface](#24-rust-to-javascript-interface)
  - [2.5 Data Processing Pipeline](#25-data-processing-pipeline)
  - [2.6 Client Registry Architecture](#26-client-registry-architecture)
  - [2.7 npm Distribution Model](#27-npm-distribution-model)
  - [2.8 CI/CD Pipeline](#28-cicd-pipeline)
  - [2.9 Pricing Engine](#29-pricing-engine)
- [3. Breaking Changes](#3-breaking-changes)
- [4. Performance](#4-performance)
- [5. Client Support Matrix](#5-client-support-matrix)
- [6. Key PRs and References](#6-key-prs-and-references)

---

## Executive Summary

Tokscale v2 is a **complete runtime rewrite** from a hybrid TypeScript/Rust NAPI-RS architecture to a **pure Rust binary**. The transition took ~17 days on a parallel branch (Feb 3--20, 2026), shipped as [PR #150](https://github.com/junhoyeo/tokscale/pull/150) (+33,752 / -7,206 lines across 113 files), and was released on Feb 26, 2026.

| Dimension | v1 (<=1.4.3) | v2 (>=2.0.0) |
|-----------|-------------|-------------|
| **Runtime** | TypeScript CLI + Rust NAPI core (requires Bun) | Pure Rust binary (zero runtime dependency) |
| **TUI** | Solid.js + OpenTUI (Zig engine) | Ratatui + crossterm (pure Rust) |
| **JS Bridge** | NAPI-RS v3 (`.node` binaries) | None -- native binary via `spawnSync` |
| **npm deps** | 13 production dependencies | 0 production dependencies |
| **Windows** | Broken (OpenTUI freeze bug [#143](https://github.com/junhoyeo/tokscale/issues/143)) | Full support |
| **Terminology** | "source" | "client" |
| **Test coverage** | ~22% | ~60%+ |
| **Supported clients** | 10 (at v1.4.3) | 14+ (at v2.0.4) |

---

## 1. Version Timeline

### 1.1 Complete Release History

32 releases across 5 months:

| Era | Versions | Dates | Key Milestones |
|-----|----------|-------|----------------|
| **Genesis** | (pre-tag) | Dec 1--2, 2025 | TypeScript POC: OpenCode, Claude, Codex support |
| **v1.0.x** | v1.0.14 -- v1.0.24 | Dec 19, 2025 -- Jan 13, 2026 | Bun migration, OpenTUI/Solid.js TUI, Gemini/Amp/Droid support, pricing service |
| **v1.1.x** | v1.1.0 -- v1.1.2 | Jan 27--30, 2026 | Headless log aggregation, Pi client support |
| **v1.2.x** | v1.2.0 -- v1.2.10 | Jan 30 -- Feb 18, 2026 | OpenClaw support (v1.2.0), SQLite dedup, timezone fixes, Cursor pricing |
| **v1.3.x** | v1.3.0 | Feb 18, 2026 | Platform-specific npm binary packages (infrastructure for v2) |
| **v1.4.x** | v1.4.0 -- v1.4.3 | Feb 18, 2026 | Kimi CLI support, glibc 2.17 compat, codex archived sessions |
| **v2.0.x** | v2.0.0 -- v2.0.4 | Feb 26 -- Mar 2, 2026 | **Ratatui rewrite**, source->client rename, Qwen/RooCode/Kilo/Synthetic support |

### 1.2 The Ratatui Rewrite Branch

The `feat/ratatui-rewrite` branch ran concurrently with v1 maintenance for 17 days:

| Date | Event | Reference |
|------|-------|-----------|
| Feb 3, 2026 | First Ratatui TUI commit | [`43bc1d0`](https://github.com/junhoyeo/tokscale/commit/43bc1d0) |
| Feb 4, 2026 | Cargo workspace created; tokscale-core extracted as pure Rust lib | [`e78ba99`](https://github.com/junhoyeo/tokscale/commit/e78ba99), [`353a91f`](https://github.com/junhoyeo/tokscale/commit/353a91f) |
| Feb 4, 2026 | Unified Rust CLI binary; legacy `packages/tui` deleted | [`6b7adc7`](https://github.com/junhoyeo/tokscale/commit/6b7adc7), [`8b400a0`](https://github.com/junhoyeo/tokscale/commit/8b400a0) |
| Feb 11, 2026 | Rust CLI feature-complete | [`85c1292`](https://github.com/junhoyeo/tokscale/commit/85c1292) |
| Feb 18--19, 2026 | v1.2.7--v1.4.2 features ported into rewrite branch | [#214](https://github.com/junhoyeo/tokscale/pull/214), [#219](https://github.com/junhoyeo/tokscale/pull/219), [#222](https://github.com/junhoyeo/tokscale/pull/222) |
| **Feb 20, 2026** | **PR #150 merged** -- the defining merge | [#150](https://github.com/junhoyeo/tokscale/pull/150) |
| Feb 26, 2026 | v2.0.0 published to npm | [`5ea0646`](https://github.com/junhoyeo/tokscale/commit/5ea0646) |

**What drove the rewrite:**

1. **Data discrepancy bug** -- The old `packages/tui` had 1,200+ lines of duplicate parser code that caused "62 vs 78 models" discrepancies between TUI and CLI output. The unified Rust binary eliminates this.
2. **Windows Terminal freeze** ([#143](https://github.com/junhoyeo/tokscale/issues/143)) -- OpenTUI's Zig engine froze on Windows Terminal. Ratatui + crossterm works everywhere.
3. **Bun dependency** -- v1 TUI required Bun runtime. v2 binary is self-contained.
4. **Bridge complexity** -- The NAPI bridge required subprocess IPC via tmpfiles (`native-runner.ts`), complex error handling, and separate build pipelines. v2 eliminates all of this.

---

## 2. Architecture Comparison

### 2.1 Runtime Model

**v1: Hybrid TypeScript + Rust subprocess model**

```
User
  └─ bunx tokscale
       └─ packages/cli/src/cli.ts          (TypeScript, commander)
            ├─ native-runner.ts             (subprocess IPC via tmpfile JSON)
            │    └─ require("@tokscale/core")  (.node NAPI binary)
            │         └─ packages/core/src/    (Rust with #[napi] attributes)
            └─ tui/App.tsx                  (Solid.js + OpenTUI Zig engine)
```

- Required **Bun** runtime (OpenTUI used native Zig modules)
- NAPI-RS v3 bridged Rust <-> JavaScript with async function exports
- Two-phase processing: `parseLocalSources()` -> TS orchestration -> `finalizeReport()`
- ~1,900 lines of TypeScript CLI logic (`cli.ts`) orchestrating native calls

**v2: Pure Rust binary with thin JS dispatcher**

```
User
  └─ bunx tokscale
       └─ packages/cli/src/index.ts        (173 lines — platform detection ONLY)
            └─ spawnSync
                 └─ @tokscale/cli-{platform}/bin/tokscale   (native Rust binary)
                      └─ crates/tokscale-core               (pure rlib, no NAPI)
```

- **Zero runtime dependency** -- works with Node, npm, pnpm, yarn, bunx, or direct binary execution
- All logic in Rust -- the JS dispatcher only resolves which platform binary to `spawnSync`
- Single `get_model_report(ReportOptions)` call replaces the two-phase NAPI bridge

**Source references:**
- v1 entry point: `packages/cli/src/cli.ts` (deleted in [`51da717`](https://github.com/junhoyeo/tokscale/commit/51da717))
- v1 NAPI bridge: `packages/cli/src/native.ts` (718 lines, deleted)
- v2 dispatcher: [`packages/cli/src/index.ts`](https://github.com/junhoyeo/tokscale/blob/main/packages/cli/src/index.ts)
- v2 CLI entry: [`crates/tokscale-cli/src/main.rs`](https://github.com/junhoyeo/tokscale/blob/main/crates/tokscale-cli/src/main.rs)

---

### 2.2 Crate / Package Structure

| Component | v1 Location | v2 Location | Change |
|-----------|-------------|-------------|--------|
| Rust core | `packages/core/` (NAPI `cdylib`) | `crates/tokscale-core/` (pure `rlib`) | NAPI removed |
| CLI binary | `packages/cli/src/cli.ts` | `crates/tokscale-cli/` | TS -> Rust |
| TUI | `packages/cli/src/tui/` (Solid.js/TSX) | `crates/tokscale-cli/src/tui/` (Rust) | OpenTUI -> Ratatui |
| Workspace root | None | `Cargo.toml` (workspace) | New |
| npm wrapper | `packages/tokscale/` | `packages/tokscale/` | Unchanged |
| Frontend | `packages/frontend/` | `packages/frontend/` | Unchanged |

**v1 `packages/core/Cargo.toml` dependencies (deleted):**
```toml
napi = "3"
napi-derive = "3"
napi-build = "2"    # build dependency
```

**v2 `crates/tokscale-core/Cargo.toml`** -- no NAPI at all. Pure Rust library consumed directly by `tokscale-cli`.

---

### 2.3 TUI Framework

| Aspect | v1 (OpenTUI + Solid.js) | v2 (Ratatui + crossterm) |
|--------|-------------------------|--------------------------|
| Language | TypeScript/TSX | Rust |
| Renderer | OpenTUI Zig engine (zero-flicker) | Ratatui terminal backend |
| Reactive framework | Solid.js 1.9.9 | None (imperative Rust) |
| Runtime requirement | Bun only | Any (compiled binary) |
| Windows support | Freeze bug ([#143](https://github.com/junhoyeo/tokscale/issues/143)) | Full support |
| Component files | 13 TSX files | 11 Rust modules |
| Data loading | Synchronous NAPI call | Background thread with disk cache + spinner |
| Dialogs | None | Modal dialogs for source/group-by pickers |

**v1 TUI components** (`packages/cli/src/tui/components/`):
```
BarChart.tsx, DailyView.tsx, DateBreakdownPanel.tsx, Footer.tsx,
Header.tsx, Legend.tsx, LoadingSpinner.tsx, ModelRow.tsx,
ModelView.tsx, OverviewView.tsx, StatsView.tsx, TokenBreakdown.tsx
```

**v2 TUI modules** (`crates/tokscale-cli/src/tui/ui/`):
```
bar_chart.rs, daily.rs, dialog/, footer.rs, header.rs,
models.rs, overview.rs, spinner.rs, stats.rs, widgets.rs
```

---

### 2.4 Rust-to-JavaScript Interface

**v1: NAPI-RS v3 -- 7+ async JS-callable functions**

```rust
// packages/core/src/lib.rs (v1)
#[napi]
pub fn version() -> String { ... }

#[napi(object)]
pub struct TokenBreakdown { ... }

#[napi(object)]
pub struct ParsedMessages { ... }
```

Exposed via `.node` binary, loaded with `require("@tokscale/core")` in TypeScript.

**v2: No NAPI -- zero JS-callable functions**

```rust
// crates/tokscale-core/src/lib.rs (v2)
pub struct TokenBreakdown { ... }    // Plain Rust struct, no #[napi]
pub struct ParsedMessages { ... }    // Direct Rust consumption only
```

The Rust binary is invoked via `spawnSync` -- no programmatic JS <-> Rust bridge.

---

### 2.5 Data Processing Pipeline

**v1: Two-phase with JS orchestration**

```
Phase 1: parseLocalSources(options)
  ├─ Rust scans filesystem → finds session files
  ├─ Rust parses JSONL/JSON → extracts messages
  └─ Returns ParsedMessages to TypeScript via NAPI

  TypeScript: merge Cursor CSV data, apply date filters

Phase 2: finalizeReport(messages, pricing, options)
  ├─ Rust receives messages + pricing data from JS
  ├─ Aggregates by model/source
  └─ Returns ModelReport to TypeScript for display
```

**v2: Single-phase, all in Rust**

```
get_model_report(ReportOptions)
  ├─ Rust scans filesystem → finds session files
  ├─ Rust parses all formats (JSONL, JSON, CSV, SQLite)
  ├─ Rust fetches pricing (LiteLLM + OpenRouter, disk-cached)
  ├─ Rust aggregates by model/client/provider
  └─ Returns complete ModelReport directly to TUI/CLI renderer
```

No data crosses the Rust <-> JS boundary. Cursor API sync, pricing fetch, and all processing happen within the Rust binary.

---

### 2.6 Client Registry Architecture

**v1: `SessionType` enum with scattered per-client fields**

```rust
// v1 ScanResult had named fields for each client
pub struct ScanResult {
    pub opencode_files: Vec<PathBuf>,
    pub claude_files: Vec<PathBuf>,
    pub codex_files: Vec<PathBuf>,
    pub gemini_files: Vec<PathBuf>,
    pub amp_files: Vec<PathBuf>,
    // ... one field per client
}
```

Adding a new client required modifying `ScanResult`, `ParsedMessages` (count fields), scanner, parser, CLI flags, and TUI -- across both Rust and TypeScript.

**v2: Centralized `define_clients!` macro with array-indexed `ClientId`**

```rust
// crates/tokscale-core/src/clients.rs (v2)
define_clients!(
    OpenCode = 0 => { id: "opencode", root: PathRoot::XdgData, relative: "opencode/storage/message", pattern: "*.json", ... },
    Claude = 1   => { id: "claude",   root: PathRoot::Home,    relative: ".claude/projects",           pattern: "*.jsonl", ... },
    // ... all clients defined in one macro invocation
    OpenClaw = 7 => { id: "openclaw", root: PathRoot::Home,    relative: ".openclaw/agents",           pattern: "*.jsonl", ... },
);
```

Adding a new client: add one entry to the macro. `ClientCounts` uses a fixed-size array `[i32; ClientId::COUNT]` instead of named fields.

**Terminology change:** `source` -> `client` across the entire codebase ([PR #230](https://github.com/junhoyeo/tokscale/pull/230)).

---

### 2.7 npm Distribution Model

**v1: NAPI binary + TypeScript package**

```
@tokscale/core                    ← NAPI .node binary (Rust)
├── @tokscale/core-darwin-arm64   ← platform-specific .node files
├── @tokscale/core-linux-x64-gnu
└── ...

@tokscale/cli                     ← TypeScript CLI (depends on @tokscale/core)
├── src/cli.ts, native.ts, tui/, ...
├── dependencies: @tokscale/core, commander, solid-js, @opentui/*, picocolors, ...
└── 13 production dependencies

tokscale                          ← alias wrapper
```

**v2: Native binary + thin dispatcher**

```
@tokscale/cli-darwin-arm64        ← native Rust binary (/bin/tokscale)
@tokscale/cli-linux-x64-gnu
@tokscale/cli-win32-x64-msvc
└── ... (8 platform packages)

@tokscale/cli                     ← 173-line JS dispatcher (spawnSync)
├── dependencies: {}              ← ZERO production dependencies
├── optionalDependencies: @tokscale/cli-{platform} × 8
└── src/index.ts (platform detection only)

tokscale                          ← alias wrapper (unchanged)
```

**v1 `@tokscale/cli` dependencies (13):**
```json
{
  "@napi-rs/canvas": "^0.1.68",
  "@opentui/core": "0.1.60",
  "@opentui/solid": "^0.1.60",
  "@resvg/resvg-js": "^2.6.2",
  "@tokscale/core": "1.4.3",
  "cli-table3": "^0.6.5",
  "clipboardy": "^5.0.2",
  "commander": "^14.0.2",
  "csv-parse": "^5.6.0",
  "date-fns": "^4.1.0",
  "picocolors": "^1.1.1",
  "solid-js": "1.9.9",
  "string-width": "^8.1.0"
}
```

**v2 `@tokscale/cli` dependencies: `{}`** (empty)

---

### 2.8 CI/CD Pipeline

| Aspect | v1 | v2 |
|--------|----|----|
| Build tool | `@napi-rs/cli` | `cargo build --release -p tokscale-cli` |
| Cross-compilation | NAPI-RS cross-compile | `cargo-zigbuild` (Zig as linker) |
| Targets | 8 platform `.node` binaries | 8 platform native binaries |
| Publish chain | `@tokscale/core` -> `@tokscale/cli` -> `tokscale` | `@tokscale/cli-{platform}` x8 -> `@tokscale/cli` -> `tokscale` |
| Test infra | Minimal | Comprehensive (`tarpaulin`, lcov, 1,057-line integration test suite) |
| Workflow file | `.github/workflows/publish-cli.yml` | Same file, rewritten for Rust workspace |
| glibc target | Default | `2.17` via `cargo-zigbuild` suffix |

**Reference:** [`.github/workflows/publish-cli.yml`](https://github.com/junhoyeo/tokscale/blob/main/.github/workflows/publish-cli.yml)

---

### 2.9 Pricing Engine

The pricing engine architecture is largely the same between v1 and v2 -- the core logic was already in Rust. Key difference: v2 removed the NAPI boundary so pricing data no longer crosses to JavaScript.

| Component | Location | Function |
|-----------|----------|----------|
| LiteLLM provider | `pricing/litellm.rs` | Fetches [LiteLLM pricing DB](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json) |
| OpenRouter fallback | `pricing/openrouter.rs` | Fallback for models not in LiteLLM |
| Cursor overrides | `pricing/lookup.rs` | Hardcoded pricing for very new models (e.g., `gpt-5.3-codex`) |
| Model aliases | `pricing/aliases.rs` | Maps friendly names -> canonical model IDs |
| Disk cache | `pricing/cache.rs` | 1-hour TTL, stored at `~/.cache/tokscale/pricing-*.json` |

**Lookup strategy (7-step resolution):**
1. Exact match -> 2. Alias resolution -> 3. Tier suffix stripping -> 4. Version normalization -> 5. Provider prefix matching -> 6. Cursor model pricing -> 7. Fuzzy matching

---

## 3. Breaking Changes

| Change | Impact | Migration |
|--------|--------|-----------|
| **`source` -> `client` terminology** ([#230](https://github.com/junhoyeo/tokscale/pull/230)) | API field names, CLI output, JSON export keys renamed | Update any scripts parsing tokscale output |
| **Bun no longer required** | Users on Node/npm/pnpm can now run tokscale | No action needed (positive change) |
| **`@tokscale/core` package removed** ([`3b8e2e0`](https://github.com/junhoyeo/tokscale/commit/3b8e2e0)) | Any direct programmatic imports of `@tokscale/core` break | Use CLI output instead |
| **OpenTUI TUI removed** | Visual appearance changed (Ratatui renders differently) | Keyboard shortcuts preserved; visual adaptation only |
| **Two-phase report API removed** | `parseLocalSources()` + `finalizeReport()` no longer exist | Use CLI commands or `--json` output |
| **Default group-by changed** | TUI defaults to `model` grouping (was `client,model`) | Press `g` in TUI to switch |

---

## 4. Performance

The native Rust core was already present in v1 for parsing. v2's gains come from eliminating the JS <-> Rust bridge overhead:

| Operation | v1 (TS + NAPI) | v2 (Pure Rust) | Source |
|-----------|----------------|----------------|--------|
| File discovery | ~50ms | ~50ms | Same Rust scanner |
| JSON parsing | ~100ms | ~100ms | Same SIMD parser |
| Aggregation | ~25ms | ~25ms | Same Rust aggregator |
| **Bridge overhead** | ~50--100ms | **0ms** | NAPI serialization eliminated |
| **TUI startup** | ~500ms+ (Bun + OpenTUI init) | **~100ms** (native binary + disk cache) | New disk cache at `~/.cache/tokscale/` |
| **Total (1k files)** | ~300--400ms | **~175ms** | README benchmarks |

Memory: v2 achieves ~45% memory reduction through streaming JSON parsing and zero-copy string handling (same as v1 core, but without Node.js memory overhead).

---

## 5. Client Support Matrix

| # | Client | v1 Added In | v2 Status | Data Path | Format |
|---|--------|-------------|-----------|-----------|--------|
| 1 | OpenCode | Genesis (Dec 2025) | Supported | `~/.local/share/opencode/` | SQLite (1.2+) + JSON |
| 2 | Claude Code | Genesis | Supported | `~/.claude/projects/` | JSONL |
| 3 | Codex CLI | Genesis | Supported | `~/.codex/sessions/` | JSONL |
| 4 | Cursor IDE | v1.0.14 | Supported | API sync -> `~/.config/tokscale/cursor-cache/` | CSV |
| 5 | Gemini CLI | v1.0.14 | Supported | `~/.gemini/tmp/*/chats/` | JSON |
| 6 | Amp (AmpCode) | v1.0.18 | Supported | `~/.local/share/amp/threads/` | JSON |
| 7 | Droid (Factory) | v1.0.18 | Supported | `~/.factory/sessions/` | JSON |
| 8 | OpenClaw | v1.2.0 | Supported | `~/.openclaw/agents/` | JSONL |
| 9 | Pi | v1.1.0 | Supported | `~/.pi/agent/sessions/` | JSONL |
| 10 | Kimi CLI | v1.4.3 | Supported | `~/.kimi/sessions/` | JSONL |
| 11 | Qwen CLI | v2.0.3 | Supported | `~/.qwen/projects/` | JSONL |
| 12 | Roo Code | v2.0.3 | Supported | `~/.config/Code/.../roo-cline/tasks/` | JSON |
| 13 | Kilo | v2.0.3 | Supported | `~/.config/Code/.../kilo-code/tasks/` | JSON |
| 14 | Synthetic | v2.0.3 | Supported | Re-attributed from other sources | N/A |

**v1 (at v1.4.3):** 10 clients | **v2 (at v2.0.4):** 14 clients (+Qwen, Roo Code, Kilo, Synthetic)

---

## 6. Key PRs and References

### Architecture / Migration PRs

| PR | Title | Impact |
|----|-------|--------|
| [#150](https://github.com/junhoyeo/tokscale/pull/150) | Ratatui rewrite and Rust workspace migration | **The v2 defining PR** -- +33,752 / -7,206 lines |
| [#230](https://github.com/junhoyeo/tokscale/pull/230) | Rename `source` -> `client` for AI client terminology | Breaking API rename |
| [#237](https://github.com/junhoyeo/tokscale/pull/237) | Centralize client definitions with `define_clients!` macro | New extensible client registry |
| [#239](https://github.com/junhoyeo/tokscale/pull/239) | Deploy v2 to main | v2 deploy to production |

### Key Commits

| Commit | Description |
|--------|-------------|
| [`353a91f`](https://github.com/junhoyeo/tokscale/commit/353a91f) | Extract pure Rust library from NAPI core |
| [`6b7adc7`](https://github.com/junhoyeo/tokscale/commit/6b7adc7) | Add unified Rust CLI with TUI |
| [`8b400a0`](https://github.com/junhoyeo/tokscale/commit/8b400a0) | Remove legacy `packages/tui` |
| [`51da717`](https://github.com/junhoyeo/tokscale/commit/51da717) | Remove dead v1 TypeScript code |
| [`3b8e2e0`](https://github.com/junhoyeo/tokscale/commit/3b8e2e0) | Remove `packages/core` NAPI-RS package |
| [`5ea0646`](https://github.com/junhoyeo/tokscale/commit/5ea0646) | Bump version to 2.0.0 |

### External References

| Resource | URL |
|----------|-----|
| GitHub Repository | https://github.com/junhoyeo/tokscale |
| npm: `@tokscale/cli` | https://www.npmjs.com/package/@tokscale/cli |
| npm: `tokscale` | https://www.npmjs.com/package/tokscale |
| v2.0.0 Release | https://github.com/junhoyeo/tokscale/releases/tag/v2.0.0 |
| LiteLLM Pricing DB | https://github.com/BerriAI/litellm |
| Ratatui | https://ratatui.rs/ |
| OpenTUI (v1, deprecated) | https://github.com/sst/opentui |

---

*Generated from codebase analysis, git history (32 tags, 250+ commits), 32 GitHub releases, and PR review.*
