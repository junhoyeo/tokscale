# AI Agent Guidelines

This document provides context for AI agents working on the Tokscale project.

## Maintaining AGENTS.md Files

When updating AGENTS.md files, follow these principles:

- **No hardcoded counts** — Don't write "10 crates" or "5 modules"; these become outdated instantly
- **No exhaustive lists** — Prefer dynamic commands (`ls crates/`) over maintaining complete lists
- **Document constraints, not descriptions** — Focus on non-obvious behaviors, gotchas, and cross-crate dependencies
- **Use nested AGENTS.md** — Place crate-specific details in `crates/{name}/AGENTS.md`, not here
- **Verify before documenting** — Grep/read the code to confirm claims are accurate
- **Delete outdated info** — Outdated docs are worse than no docs

## Commit Message Convention

```
<type>: <description>

[optional body]
```

### Types

| Type | Description |
|------|-------------|
| `feat` | New feature |
| `fix` | Bug fix |
| `refactor` | Code refactoring (no behavior change) |
| `docs` | Documentation only |
| `test` | Adding or updating tests |
| `chore` | Maintenance tasks |
| `perf` | Performance improvements |

### Examples

```
feat: add session branching with /fork command
fix: handle empty response from provider
refactor: extract streaming logic to separate module
docs: update README with new CLI options
```

### Commit Message & PR Title Rules (CRITICAL)

> These rules apply to **both commit messages AND pull request titles**. PR titles become the squash-merge commit message, so they must follow the same conventions.

**DO:**
- Describe the actual change in plain, technical terms
- Keep commits atomic (one logical change per commit)
- Use the format: `<type>(<scope>): <what changed and why>`

**DON'T:**
- Reference internal review labels (P0, P1, P2, etc.) in commits or PR titles
- Mention "Oracle", "audit", "review findings", "hardening" in commits or PR titles
- Use agent-internal jargon: "wave", "hardening", "compliance", "verification pass"
- Bundle multiple unrelated fixes into one commit
- Use vague messages like "fix issues" or "address feedback"

**Good Examples:**
```
fix(lsp): pass server args to stdio spawn command
fix(lsp): convert 1-indexed input lines to 0-indexed LSP positions
fix(gemini): parse SSE data frames instead of raw JSON lines
fix(orchestrator): route provider tools through approval flow
```

**Bad Examples (NEVER do this):**
```
fix: address P0 issues from Oracle review      ❌
fix(hardening): Oracle Round 4 fixes           ❌
fix: audit findings                            ❌
fix: various improvements                      ❌
fix(tui): harden unreleased changes — P0-P3    ❌  (PR title)
fix: hardening wave 1 compliance fixes         ❌  (PR title)
```

## Agent Command Execution

- When running `tokscale` CLI commands from an automated agent (tests, CI, or tool-driven shells), always pass `--no-spinner` unless spinner behavior is the thing being tested.
- This avoids non-interactive terminal issues and keeps command output stable for assertions and logs.

## Release & Deployment

### Overview

Releases are published to npm via a GitHub Actions `workflow_dispatch` pipeline, followed by a manually created GitHub Release with handwritten notes. There is no staging environment — publishes go directly to npm `latest`.

### Release Pipeline

**Workflow:** `.github/workflows/publish-cli.yml`

**Trigger:** Manual — GitHub Actions UI → "Publish" → "Run workflow"

**Inputs:**
- `bump`: Version bump type — `patch (x.x.X)` | `minor (x.X.0)` | `major (X.0.0)`
- `version` (optional): Override string (e.g., `2.0.0-beta.1`), takes precedence over bump

**Stages (sequential):**

| # | Job | Description |
|---|-----|-------------|
| 1 | `bump-versions` | Reads current version from `packages/cli/package.json`, calculates new version, updates the Rust workspace version plus the CLI, wrapper, and platform package manifests, then uploads the bumped manifests as an artifact |
| 2 | `build-cli-binary` | 8-target parallel native Rust builds (macOS x86/arm64, Linux glibc/musl x86/arm64, Windows x86/arm64) |
| 3 | `publish-platform-packages` | Publishes platform-specific packages (`@tokscale/cli-darwin-arm64`, etc.) containing native binaries to npm |
| 4 | `publish-cli` | Publishes `@tokscale/cli` to npm (binary dispatcher + optionalDependencies) |
| 5 | `publish-alias` | Publishes `tokscale` wrapper package to npm |
| 6 | `finalize` | Commits the bumped release manifests back to repo as `chore: bump version to X.Y.Z` (authored by `github-actions[bot]`) |

**Duration:** ~15-20 minutes end-to-end.

**Package publish chain:** `@tokscale/cli` (with platform packages as optionalDependencies) → `tokscale` (depends on cli). Each waits for the previous to succeed.

### Post-Pipeline: Git Tag & GitHub Release

The CI pipeline does **NOT** create the git tag or GitHub Release. After the workflow completes successfully:

1. Verify the `chore: bump version to X.Y.Z` commit was pushed by CI
2. Create a GitHub Release manually:
   - **Tag:** `vX.Y.Z` (e.g., `v1.2.1`)
   - **Target:** The `chore: bump version to X.Y.Z` commit
   - **Title:** See [Release Notes Style](#release-notes-style) below
   - **Body:** See [Release Notes Template](#release-notes-template) below
3. Publish the release (not as draft, not as prerelease)

### Versioning Conventions

| Bump Type | When to Use | Example |
|-----------|-------------|---------|
| `patch` | Bug fixes, small features, additive parser support | `1.2.0` → `1.2.1` |
| `minor` | New client support, significant features, UI overhauls | `1.1.2` → `1.2.0` |
| `major` | Breaking changes (never used so far) | `1.2.1` → `2.0.0` |

Release version is stored in the Rust workspace and the npm package manifests, and CI updates them together:
- `Cargo.toml` (`[workspace.package].version`) — Rust binary and exported metadata version
- `packages/cli/package.json` — CLI package version and platform optional dependency versions
- Platform packages (`packages/cli-*/package.json`) — native package versions
- `packages/tokscale/package.json` — wrapper version plus `@tokscale/cli` dependency version

### CI-Only Workflow

**`.github/workflows/build-native.yml`** — Runs on PRs touching `crates/tokscale-cli/**`. Builds all 8 native targets to verify compilation. Does not publish.

---

### Release Notes Style

#### Title Conventions

| Release Type | Title Format |
|-------------|--------------|
| Standard patch/minor | `` `tokscale@vX.Y.Z` is here! `` |
| Flagship feature | `` EMOJI `tokscale@vX.Y.Z` is here! (Short subtitle with [link](...)) `` |
| Feature spotlight | Custom banner image replacing the standard hero + call-to-action |

**Examples from past releases:**
- Standard: `` `tokscale@v1.1.2` is here! ``
- Flagship: `` 🦞 `tokscale@v1.2.0` is here! (Now supports [OpenClaw](https://github.com/openclaw/openclaw)) ``
- Spotlight: Custom Wrapped 2025 banner + `` Generate your Wrapped 2025 with `tokscale@v1.0.16` ``

#### Release Notes Template

```markdown
<div align="center">

[![Tokscale](https://github.com/junhoyeo/tokscale/raw/main/.github/assets/hero-v2.png)](https://github.com/junhoyeo/tokscale)

# `tokscale@vX.Y.Z` is here!
</div>

## What's Changed
* scope(area): description by @author in https://github.com/junhoyeo/tokscale/pull/NNN
* scope(area): description by @author in https://github.com/junhoyeo/tokscale/pull/NNN

## New Contributors
* @username made their first contribution in https://github.com/junhoyeo/tokscale/pull/NNN

**Full Changelog**: https://github.com/junhoyeo/tokscale/compare/vPREVIOUS...vNEW
```

#### Style Rules

| Element | Rule |
|---------|------|
| **Header** | Always centered `<div align="center">` with hero banner image linked to the repo |
| **Title** | Backtick-wrapped `tokscale@vX.Y.Z` — package name, not just version |
| **PR list** | `* scope(area): description by @author in URL` — mirrors the PR title exactly as merged |
| **Optional summary** | For releases with many changes or when PR titles alone don't convey impact, add a brief bullet list between the title and "What's Changed" (see v1.0.18 as example) |
| **New Contributors** | Include section when there are first-time contributors |
| **Full Changelog** | Always present at bottom as a GitHub compare link `vPREV...vNEW` |
| **Tone** | Concise. No prose paragraphs. Let the PR list speak for itself. |
| **No draft issues** | Never reference draft release issues (e.g., #121) in the notes |

#### When to Add a Summary Block

Add a short bullet list summary (before "What's Changed") when:
- The release has 4+ PRs spanning different areas
- PR titles alone don't convey the user-facing impact
- A new client/integration is the headline

**Example (v1.0.18):**
```markdown
- Improved model price resolver (Rust)
- Add support for Amp (AmpCode) and Droid (Factory Droid)
- Improved sorting feature on TUI
```

### Deployment Checklist

```
1. [ ] All target PRs merged to main
2. [ ] `cargo test` passes in crates/tokscale-cli
3. [ ] No open blocker bugs (regressions from changes being released)
4. [ ] Run "Publish" workflow via GitHub Actions UI
   - Select bump type (patch/minor/major)
   - Wait for all 6 stages to complete
5. [ ] Verify `chore: bump version to X.Y.Z` commit was pushed
6. [ ] Verify packages on npm: @tokscale/cli, tokscale
7. [ ] Create GitHub Release
   - Tag: vX.Y.Z targeting the bump commit
   - Write release notes following the template above
   - Publish (not draft, not prerelease)
8. [ ] Smoke test: `bunx tokscale@latest --version`
```
