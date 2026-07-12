# Contributing to Tokscale

Thanks for your interest in improving Tokscale. This guide covers the development workflow and the most common contribution — adding support for a new AI coding client. For commit-message and pull-request conventions, see [`AGENTS.md`](./AGENTS.md).

## Development setup

Tokscale is a Cargo workspace (the `tokscale` CLI and its core library) plus a Next.js web frontend.

Prerequisites: a stable Rust toolchain (`rustup` recommended) and [Bun](https://bun.sh) for the frontend and package scripts.

| Task | Command |
| --- | --- |
| Format check | `cargo fmt --all -- --check` |
| Lint (must pass with **no** warnings) | `cargo clippy --locked --workspace --all-features -- -D warnings` |
| Rust tests | `cargo test --workspace --all-features` |
| Build the CLI | `cargo build --release -p tokscale-cli` |
| Frontend dev server | `bun run dev:frontend` |
| Frontend registry contract | `bun run --cwd packages/frontend test -- __tests__/lib/clientRegistry.test.ts` |
| Frontend type check | `bun run --cwd packages/frontend typecheck` |

CI runs format, Clippy, and Rust tests on Linux, then builds the supported release targets separately. Frontend CI runs Vitest, migration replay, and the type check; it also runs when `crates/tokscale-core/src/clients.rs` or a GitHub CDN asset changes, so cross-registry client checks cannot be skipped by a Rust-only integration PR. A clean local `cargo fmt`, `cargo clippy`, and `cargo test` is the baseline for a reviewable PR.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/tokscale-core` | Client registry, per-client session parsers, scanner, aggregation, and cache |
| `crates/tokscale-cli` | CLI entrypoint, argument parsing, and the terminal UI |
| `packages/frontend` | Next.js web app: public profiles, the submission API, and the frontend client registry |
| `.github/assets` | Client logos, served to the frontend over the GitHub raw CDN |

## Adding a new client integration

A "client" is one AI coding tool whose local session logs Tokscale scans, parses, and reports (Claude Code, Codex, Cursor, and so on). A complete integration spans three areas — the Rust core, the CLI, and the web frontend — and the frontend half is required even though the Rust build passes without it. The single most common mistake is registering a client only in Rust: the CLI then scans and submits usage for it, but the server rejects every submission that includes it. See [Registry enforcement](#registry-enforcement) for why.

Work through the checklist in order.

### 1. Register the client (Rust core)

Add an entry to the `define_clients!` invocation in `crates/tokscale-core/src/clients.rs`. Indices must be sequential — use the next unused number (a compile-time assertion enforces this):

```rust
Pi = 8 => {
    id: "pi",                       // stable id used everywhere: CLI flag, submit payload, frontend
    root: PathRoot::Home,           // base directory the relative path resolves against
    relative: ".pi/agent/sessions", // where the client stores its session logs
    pattern: "*.jsonl",             // glob for session files under that directory
    headless: false,                // supports headless / subprocess capture
    parse_local: true,              // parse files locally (vs. remote-only)
    submit_default: true            // included in `tokscale submit` by default
},
```

The `id` string is the contract that ties every other layer together. Choose it once and reuse it verbatim.

### 2. Write the parser (Rust core)

Add `crates/tokscale-core/src/sessions/<client>.rs` with a function that returns `Vec<UnifiedMessage>`, and register the module in `crates/tokscale-core/src/sessions/mod.rs` with `pub mod <client>;`. Model it on an existing parser of the same shape — JSONL (`pi.rs`), SQLite (`opencode.rs`), or NDJSON (`devin.rs`). Parse defensively: skip malformed rows instead of panicking, and clamp token counts to non-negative values.

### 3. Wire discovery and dispatch (Rust core)

Discover the client's session files in `crates/tokscale-core/src/scanner.rs` (follow the pattern used by a similar client), then dispatch parsing inside `parse_all_messages_*` in `crates/tokscale-core/src/lib.rs`, passing the client identity so cache entries record their owner:

```rust
load_or_parse_source(
    message_cache::CacheIdentity::for_client(ClientId::MyClient),
    path,
    &source_cache,
    pricing,
    sessions::myclient::parse_myclient_file,
);
```

Neither step is compile-enforced: a client that is registered but not wired here builds cleanly and silently produces no data.

### 4. Wire the CLI

In `crates/tokscale-cli/src/main.rs`, add the `ClientFilter` variant and its two mappings — the `id` string and the `ClientId` it resolves to — so `--client <id>` works. In `crates/tokscale-cli/src/tui/client_ui.rs`, add a `ClientUi { display_name, hotkey }` entry with an unused `hotkey`. `CLIENT_UI` is a fixed-size array (`[ClientUi; ClientId::COUNT]`), so the build fails until it has exactly one entry per client.

### 5. Register the client on the web frontend (required)

This is the step that is easy to miss and that breaks real usage. Add the `id` to the frontend registry:

- `packages/frontend/src/lib/types.ts` — add the `id` to `SUPPORTED_CLIENT_TYPES`. **This list gates `POST /api/submit`.** A submission that contains any client id not in this list is rejected in full, so a client missing here cannot submit at all.
- `packages/frontend/src/lib/constants.ts` — add the `id` to all three per-client registries: `SOURCE_DISPLAY_NAMES` (human-readable name), `SOURCE_LOGOS` (logo URL — see step 6), and `SOURCE_COLORS` (a chart color that reads well in both light and dark themes).

Because those three are `Record<ClientType, …>`, `tsc` fails to compile once the id is in `SUPPORTED_CLIENT_TYPES` until each has an entry — so if you start with `types.ts`, the type checker guides you through the rest.

The frontend registry contract test also reads the Rust `define_clients!` list, validates each id through the submit schema, and checks its display name, logo, and color. Frontend CI is explicitly triggered by `clients.rs`, so adding a Rust client without completing this step fails before merge.

### 6. Add the logo asset

Add `.github/assets/client-<id>.png` (or `.jpg`) and reference it from `SOURCE_LOGOS`:

```ts
"<id>": `${GITHUB_CDN_BASE}/client-<id>.png`,
```

`SOURCE_LOGOS` may point at an external URL instead, but any `GITHUB_CDN_BASE` reference must resolve to a file that exists in `.github/assets` on `main`, or the frontend renders a broken image. CLI and desktop variants of the same product may share a single asset (as `antigravity` and `antigravity-cli` do).

The frontend registry contract test checks every GitHub CDN logo against the checked-in asset directory. It is triggered by `.github/assets/**` changes as well as frontend changes.

### 7. Update user-facing documentation

Add the client to the supported-client overview and feature list in `README.md`, plus the data-location matrix when a platform-specific path matters. Mirror those user-facing updates in the maintained localized READMEs (`README.ko.md`, `README.ja.md`, and `README.zh-cn.md`).

### Registry enforcement

Some registries fail the build when they are incomplete; others fail silently at runtime. Know which is which:

| Registry / step | Enforced by | If omitted |
| --- | --- | --- |
| `define_clients!` sequential index | Rust compile-time assertion | Build fails |
| `CLIENT_UI` array | Rust fixed-size array | Build fails |
| `SOURCE_DISPLAY_NAMES` / `SOURCE_LOGOS` / `SOURCE_COLORS` | TypeScript `Record<ClientType>` + frontend registry contract test | Type check or CI test fails |
| **`SUPPORTED_CLIENT_TYPES`** | **Frontend registry contract test, triggered by Rust registry changes** | **CI fails before a server-rejected client ships** |
| Scanner + `lib.rs` dispatch | Nothing | Client is defined but scans and reports no data |
| Logo asset file | Frontend registry contract test, triggered by asset changes | CI fails for a missing GitHub CDN asset |

The scanner/dispatch row is the remaining silent integration gap. Verify it by hand.

### Verifying the integration

1. Run `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-features -- -D warnings`, and `cargo test --workspace --all-features`.
2. Add a parser unit test with a small fixture in the client's real log format.
3. Run `tokscale --no-spinner --client <id>` against real local data and confirm the message and token counts look right.
4. Run `bun run --cwd packages/frontend test -- __tests__/lib/clientRegistry.test.ts` and `bun run --cwd packages/frontend typecheck`.
5. Update the supported-client and data-location documentation, including maintained translations.

## Commit messages and pull requests

Tokscale uses [Conventional Commits](https://www.conventionalcommits.org), and a PR title becomes its squash-merge commit message. The full rules — allowed types, atomic-commit guidance, and title restrictions — live in [`AGENTS.md`](./AGENTS.md). In short: a new client is a `feat` (for example, `feat(clients): add <Name> session parsing`), and titles should describe the change, not internal review or process labels.
