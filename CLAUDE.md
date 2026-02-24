# CLAUDE.md — AI Coding Assistant Guide for tokscale

This is a **Rust + TypeScript monorepo** using [Bun](https://bun.sh/) as the runtime and package manager. The project is a high-performance CLI tool for tracking AI coding assistant token usage.

---

## Project Structure

```
tokscale/
├── package.json                  # Monorepo root (Bun workspaces)
├── packages/
│   ├── core/                     # Rust native module (napi-rs)
│   │   ├── Cargo.toml
│   │   ├── build.rs
│   │   ├── src/
│   │   │   ├── lib.rs            # NAPI exports & main entry point
│   │   │   ├── scanner.rs        # Parallel file discovery (walkdir + rayon)
│   │   │   ├── parser.rs         # SIMD JSON parsing (simd-json)
│   │   │   ├── aggregator.rs     # Parallel aggregation (rayon)
│   │   │   ├── pricing.rs        # Cost calculation logic
│   │   │   └── sessions/         # Per-platform parsers
│   │   │       ├── mod.rs
│   │   │       ├── claudecode.rs
│   │   │       ├── codex.rs
│   │   │       ├── cursor.rs
│   │   │       ├── gemini.rs
│   │   │       └── opencode.rs
│   │   └── __test__/             # Node.js integration tests (ava)
│   │
│   ├── cli/                      # TypeScript CLI (@tokscale/cli)
│   │   ├── bunfig.toml           # Bun config — loads OpenTUI Solid.js plugin
│   │   ├── tsconfig.json         # ES2022 + NodeNext + strict
│   │   └── src/
│   │       ├── cli.ts            # Commander.js entry point
│   │       ├── native.ts         # Native module loader (TS fallback on failure)
│   │       ├── native-runner.ts  # Subprocess bridge to Rust binary
│   │       ├── pricing.ts        # LiteLLM pricing fetcher (1hr disk cache)
│   │       ├── graph.ts          # Graph data generation
│   │       ├── cursor.ts         # Cursor IDE API integration
│   │       ├── auth.ts           # Social platform auth
│   │       ├── submit.ts         # Leaderboard submission
│   │       ├── wrapped.ts        # Wrapped 2025 image generation
│   │       ├── table.ts          # Legacy CLI table output
│   │       ├── spinner.ts        # Terminal spinner utility
│   │       ├── tui/              # OpenTUI interactive interface
│   │       │   ├── App.tsx       # Main TUI app (Solid.js JSX)
│   │       │   ├── components/   # TUI components
│   │       │   ├── hooks/        # Data fetching & state
│   │       │   ├── config/       # Themes & settings
│   │       │   └── utils/        # Formatting utilities
│   │       └── sessions/         # TypeScript fallback parsers
│   │           ├── claudecode.ts
│   │           ├── codex.ts
│   │           ├── gemini.ts
│   │           ├── opencode.ts
│   │           ├── reports.ts
│   │           └── types.ts
│   │
│   ├── frontend/                 # Next.js 16 web app (@tokscale/frontend)
│   │   ├── next.config.ts
│   │   ├── drizzle.config.ts
│   │   ├── middleware.ts
│   │   └── src/
│   │       ├── app/              # Next.js App Router
│   │       └── components/       # React components
│   │
│   ├── benchmarks/               # Performance benchmark suite
│   │   ├── runner.ts             # Benchmark harness
│   │   └── generate.ts           # Synthetic data generator
│   │
│   └── tokscale/                 # Alias package (like `swc`) — installs @tokscale/cli
```

---

## Build Commands

### Full Build (from repo root)
```bash
bun install              # Install all dependencies (auto-builds core via postinstall)
bun run build            # Build Rust core + TypeScript CLI
bun run build:core       # Build only the Rust native module (release)
bun run build:cli        # Build only the TypeScript CLI to dist/
```

### Package-Specific Builds
```bash
# Rust core (from packages/core or via root)
bun run --cwd packages/core build          # Release build
bun run --cwd packages/core build:debug    # Debug build (faster compilation)

# TypeScript CLI (from packages/cli or via root)
bun run --cwd packages/cli build           # tsc + bundle
```

### Development Mode (no build required)
```bash
bun run cli              # Run CLI directly from source via Bun
# or equivalently:
bun run --cwd packages/cli dev             # bun src/cli.ts
```

### Frontend
```bash
bun run dev:frontend     # Next.js dev server at http://localhost:3000
# or:
bun run --cwd packages/frontend dev
```

---

## Test Commands

### Rust Tests
```bash
# From packages/core:
bun run test:rust        # cargo test --features noop
bun run test             # Node.js integration tests (ava, __test__/**/*.spec.mjs)
bun run test:all         # Both Rust and Node.js tests

# Direct cargo (when in packages/core):
cargo test --features noop
```

### Benchmarks
```bash
bun run --cwd packages/core bench          # cargo bench --features noop
bun run dev:benchmarks                     # Run TypeScript benchmark harness
```

> **Note**: The `noop` feature flag is required for `cargo test` and `cargo bench` because it disables NAPI bindings that require a Node.js host.

---

## Architecture: Hybrid TypeScript + Rust

Understanding this boundary is critical before making changes.

```
TypeScript (CLI layer)                Rust native module (packages/core)
─────────────────────────             ──────────────────────────────────
• Commander.js CLI parsing            • ALL session file scanning
• Pricing fetch from LiteLLM         • ALL JSON parsing (SIMD)
• Pass pricing data → Rust            • ALL cost calculation
• Format & display results            • ALL aggregation
• OpenTUI TUI rendering               • Parallel execution via rayon
• Cursor API sync                     • ~8–10x faster than TS fallback
```

When the native module is unavailable (e.g., on a fresh clone without `build:core`), `packages/cli/src/native.ts` **automatically falls back** to the TypeScript implementations in `packages/cli/src/sessions/`. Both paths must remain in sync.

### Two-Phase Execution Model

The CLI runs these phases in parallel for maximum performance:
1. **`parse_local_sources`** (Rust) — scans OpenCode, Claude, Codex, Gemini files
2. **Pricing fetch** (TypeScript) — fetches LiteLLM data (cached to `~/.cache/tokscale/pricing.json`, 1hr TTL)
3. **Cursor sync** (TypeScript, optional) — fetches Cursor API data
4. **`finalize_report`** (Rust) — merges all data, applies pricing, aggregates

---

## Code Style Rules

### Rust (`packages/core`)

- **Clippy is enforced**: `#![deny(clippy::all)]` is set in `lib.rs`. All clippy warnings are errors.
- **Error handling**: Use `anyhow` for internal errors, `napi::Result<T>` for NAPI exports. Prefer `?` over `.unwrap()`.
- **Parallelism**: Use `rayon`'s `.par_iter()` / `.par_bridge()` for CPU-bound parallel work. Match the existing pattern in `lib.rs` and `scanner.rs`.
- **Serde**: All types exposed to NAPI must be annotated with `#[napi(object)]` and `#[derive(Debug, Clone)]`. Public fields only.
- **NaN-safe sorting**: Use explicit NaN-handling sort comparators (see `get_model_report` in `lib.rs`) — never use `.unwrap()` on `partial_cmp`.
- **No `unsafe`**: Avoid unsafe Rust unless interfacing with an external C library.
- **Module structure**: Each platform parser lives in its own file under `sessions/`. Add new parsers there, register in `sessions/mod.rs`.
- **Cargo.toml pin policy**: Only pin crate versions when there is a documented reason (e.g., `globset = "=0.4.15"` for edition2024 incompatibility). Comment the reason.

### TypeScript (`packages/cli`, `packages/frontend`)

- **Strict TypeScript**: `"strict": true` is set in all `tsconfig.json` files. Never suppress errors with `as any`, `@ts-ignore`, or `@ts-expect-error`.
- **Runtime**: **Bun only** — do not use Node.js-specific APIs (`fs/promises` is fine, but avoid `require()`, `__dirname`, etc. in new code; use `import.meta.url` patterns).
- **Module system**: `"type": "module"` — use ESM imports (`import`/`export`). No CommonJS.
- **JSX in CLI**: The `cli/` package uses Solid.js JSX via OpenTUI (`"jsxImportSource": "@opentui/solid"`). The `frontend/` package uses React JSX. Do not mix them.
- **File extensions**: Use `.ts` for logic, `.tsx` for JSX components. The TUI components live in `cli/src/tui/`.
- **Formatting**: No formatter is configured — match the surrounding code's indentation (2 spaces), quote style, and trailing comma usage.
- **Frontend linting**: ESLint with `eslint-config-next` — run `bun run lint` in `packages/frontend` before submitting frontend changes.

### General

- **Date format**: Dates are always `YYYY-MM-DD` strings (lexicographically comparable). Never use `Date` objects for date filtering in Rust.
- **Costs**: Always `f64` in Rust, never integers. In TypeScript, costs are also `number`.
- **Source identifiers**: The canonical source strings are `"opencode"`, `"claude"`, `"codex"`, `"gemini"`, `"cursor"` (lowercase). Use these exactly.
- **No secrets in source**: Cursor session tokens and API keys must never be committed. They live in `~/.config/tokscale/`.

---

## Common Pitfalls

### 1. Building the native module is required for full performance
`bun install` runs `bun run build:core` via the `postinstall` script **only if `$VERCEL` is not set**. On CI or fresh clones, run `bun run build:core` explicitly. Without it, the CLI silently falls back to TypeScript — which is ~8x slower but functionally correct.

### 2. Cargo tests require `--features noop`
Running `cargo test` without `--features noop` will fail because the NAPI bindings require a Node.js host to initialize. Always use:
```bash
cargo test --features noop
```

### 3. TUI requires Bun — do not run with Node.js
OpenTUI uses native Zig modules loaded via Bun's native module system. Running `node src/cli.ts` or `ts-node` will fail. Always use `bun src/cli.ts` or `bun run cli`.

### 4. `bunfig.toml` preloads the Solid.js JSX plugin
The file `packages/cli/bunfig.toml` contains `preload = ["@opentui/solid/preload"]`. This is required for Solid.js JSX (`.tsx` files in `cli/src/tui/`) to work at runtime with Bun. If you run `bun` commands from outside `packages/cli`, you may need to specify `--config packages/cli/bunfig.toml`.

### 5. The TypeScript CLI fallback and the Rust implementation must stay in sync
`packages/cli/src/sessions/*.ts` are the TS fallback parsers. `packages/core/src/sessions/*.rs` are the Rust implementations. When the data format of a supported platform changes, **update both**. The integration tests in `packages/core/__test__/` test the Rust path; add corresponding tests for the TS path when adding new parsers.

### 6. Cursor data is network-synced, not file-based
Cursor usage is fetched via the Cursor API and cached locally at `~/.config/tokscale/cursor-cache/*.csv`. It is **not** scanned from application directories like the other sources. The Rust scanner reads the cached CSV files; the TypeScript layer handles the network sync. Do not confuse this with the other sources.

### 7. Pricing data is fetched at runtime, not bundled
`packages/cli/src/pricing.ts` fetches LiteLLM pricing at runtime and caches it to disk (`~/.cache/tokscale/pricing.json`) with a 1-hour TTL. Pricing is then passed **from TypeScript into Rust** via the `PricingEntry[]` / `Vec<PricingEntry>` boundary. Never hardcode prices.

### 8. Gemini token billing differs from other providers
Gemini "thoughts" tokens (reasoning) count as output for billing purposes. See the cost calculation in `lib.rs` (`output + reasoning` is passed to `calculate_cost` for Gemini). Replicate this in the TypeScript fallback if you modify Gemini billing logic.

### 9. Frontend is deployed independently on Vercel
`packages/frontend` is a standalone Next.js 16 app deployed to Vercel. It does **not** depend on the Rust native module. The `VERCEL` environment variable skips `build:core` in the root `postinstall` script specifically for this reason. Do not add native module dependencies to the frontend.

### 10. `globset` is pinned — do not upgrade without checking
`globset = "=0.4.15"` is pinned in `packages/core/Cargo.toml` because newer versions require Rust Edition 2024 which has toolchain constraints. Check the comment in `Cargo.toml` before bumping this dependency.

---

## Key File Locations

| Purpose | Path |
|---|---|
| CLI entry point | `packages/cli/src/cli.ts` |
| NAPI exports (Rust) | `packages/core/src/lib.rs` |
| Native module loader (TS) | `packages/cli/src/native.ts` |
| Pricing fetcher | `packages/cli/src/pricing.ts` |
| TUI app root | `packages/cli/src/tui/App.tsx` |
| Platform parsers (Rust) | `packages/core/src/sessions/` |
| Platform parsers (TS fallback) | `packages/cli/src/sessions/` |
| Integration tests | `packages/core/__test__/` |
| Rust unit tests | Inline in each `.rs` file (`#[cfg(test)]`) |
| Frontend app router | `packages/frontend/src/app/` |
| Benchmark harness | `packages/benchmarks/runner.ts` |

---

## Environment Variables

| Variable | Default | Purpose |
|---|---|---|
| `HOME` | (system) | Base for all session file discovery |
| `XDG_DATA_HOME` | `~/.local/share` | OpenCode session location override |
| `CODEX_HOME` | `~/.codex` | Codex CLI session location override |
| `VERCEL` | unset | Skips `build:core` in postinstall when set |
| `TOKSCALE_NATIVE_TIMEOUT_MS` | `300000` | Max time for Rust subprocess (ms) |
| `TOKSCALE_MAX_OUTPUT_BYTES` | `52428800` | Max output from Rust subprocess (bytes) |
