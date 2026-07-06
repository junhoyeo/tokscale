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

## Authoring PR/Issue Content via `gh` CLI

When writing PR bodies, issue bodies, or comments through `gh pr create`, `gh issue comment`, etc.: **prefer `--body-file` with a written-out file over inline `--body` heredocs.**

Why: inline `--body "$(cat <<'EOF' ... EOF)"` patterns lead to recurring mistakes where backticks are written as `` \` `` (incorrectly escaping them inside a single-quoted heredoc where no escaping is needed). The literal `` \` `` then renders in GitHub markdown as backslash + backtick instead of inline code formatting.

**DO:**

- Write the body to `/tmp/pr-body.md` (or similar) with the `Write` tool, then `gh pr create --body-file /tmp/pr-body.md`
- For comment edits: `gh api -X PATCH repos/<owner>/<repo>/issues/comments/<id> -F body=@/tmp/comment.md`

**DON'T:**

- Use `gh pr create --body "$(cat <<'EOF' ... \` ... \` ... EOF)"` — single-quoted heredoc means backticks are already literal; escaping with `` \` `` is wrong and renders incorrectly.
- Use double-quoted heredoc for bodies containing backticks unless you intend command substitution.
- Hard-wrap paragraphs at ~80 columns inside PR/issue/comment bodies. GitHub's markdown collapses single newlines into spaces so the rendered output looks fine, but the **raw markdown view is what reviewers and authors edit in**, and mid-sentence line breaks read as if the prose was chopped. Write each paragraph as one continuous line and let the renderer wrap it. Same rule for blockquotes, list items that span multiple lines, and table cells. Hard wraps are still fine inside fenced code blocks where preserving line layout matters.

This applies to all GitHub-content authoring through the CLI — PR bodies, issue bodies, comments, edits. Commit message bodies should also follow this rule: write prose paragraphs as continuous lines, not hard-wrapped at 80 columns.

## Git Identity & Merge Discipline

- Before any commit, inspect the effective Git identity (`git config user.name` / `user.email`) and remotes. If the identity does not match the contributor or expected automation account for the current branch, stop and ask for confirmation.
- Never commit as worker/agent identities such as `worker1`, `worker2`, `worker3`, or `*@example.invalid`.
- When merging pull requests through `gh`, use squash merge (`gh pr merge --squash ...`) unless the user explicitly requests another merge strategy.
- Before merging, verify the squash commit title is the intended conventional PR title and does not contain worker/agent/internal review jargon.

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

## Migration journal hygiene

Never hand-edit `drizzle/meta/_journal.json` timestamps or sequence numbers. Always run `drizzle-kit generate` to claim a migration slot — the tool assigns the correct monotonic index and timestamp atomically.

Migrations 0010 and 0011 have round-number hand-edited timestamps (`"when": 1780000000000` and `"when": 1780086400000`) as a one-time historical exception made during the 2026-05-25 schema audit. No future migration should follow this pattern; use `drizzle-kit generate` exclusively.


If two branches generate migrations with the same index, resolve the conflict by re-running `drizzle-kit generate` on the branch that was merged later — do not manually renumber files or edit `_journal.json`.

**Never edit the SQL of a migration file after it has been applied to any database.** drizzle stores the SHA256 of the migration content in `drizzle.__drizzle_migrations` on first apply. If the local file content changes (even just a comment), the local hash diverges from the stored hash and drizzle-kit migrate will treat the migration as missing and attempt to re-apply it — which fails on idempotent-unsafe DDL. If you need to document a migration after the fact (lock-window risk, rollback notes, anything), put the commentary in a sidecar `0NNN_*.md` next to the .sql, in `schema.ts`, or in this file — never as comments inside the applied .sql.

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
- `recovery` (optional): Retry an already committed release version. Requires `version` and reuses the current release commit when the manifests already match.

**Stages (sequential):**

| Job | Description |
|-----|-------------|
| `bump-versions` | Reads current version from `packages/cli/package.json`, calculates new version, updates the Rust workspace version plus `Cargo.lock`, the CLI, wrapper, and platform package manifests, then uploads the bumped release files as an artifact |
| `build-cli-binary` | Builds the native Rust binaries defined by the workflow matrix |
| `prepare-release-provenance` | Checks npm auth/release state, then commits and pushes the release provenance files as `chore: bump version to X.Y.Z`. In recovery mode, reuses the already committed release SHA when there are no manifest diffs. |
| `publish-platform-packages` | Publishes platform-specific packages (`@tokscale/cli-darwin-arm64`, etc.) containing native binaries to npm, skipping package versions that already exist only during recovery |
| `publish-cli` | Publishes `@tokscale/cli` to npm (binary dispatcher + optionalDependencies) |
| `publish-alias` | Publishes `tokscale` wrapper package to npm |
| `finalize` | Creates or updates tag `vX.Y.Z` and the GitHub Release after npm publishing succeeds |

**Duration:** ~15-20 minutes end-to-end.

**Package publish chain:** `@tokscale/cli` (with platform packages as optionalDependencies) → `tokscale` (depends on cli). Each waits for the previous to succeed.

### Post-Pipeline: Git Tag & GitHub Release

The CI pipeline creates or updates the git tag and GitHub Release after npm publishing succeeds. After the workflow completes successfully:

1. Verify the `chore: bump version to X.Y.Z` commit was pushed by CI or reused by recovery
2. Verify tag `vX.Y.Z` targets the release provenance commit
3. Verify the GitHub Release exists and follows the release notes style below

### Versioning Conventions

| Bump Type | When to Use | Example |
|-----------|-------------|---------|
| `patch` | Bug fixes, small features, additive parser support | `1.2.0` → `1.2.1` |
| `minor` | New client support, significant features, UI overhauls | `1.1.2` → `1.2.0` |
| `major` | Breaking changes (never used so far) | `1.2.1` → `2.0.0` |

Release version is stored in the Rust workspace and the npm package manifests, and CI updates them together:
- `Cargo.toml` (`[workspace.package].version`) — Rust binary and exported metadata version
- `Cargo.lock` — local workspace package versions for `tokscale-cli` and `tokscale-core`
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
   - For a failed publish retry, set `version` to the already committed release version and enable `recovery`
   - Wait for all stages to complete
5. [ ] Verify `chore: bump version to X.Y.Z` commit was pushed
6. [ ] Verify packages on npm: @tokscale/cli, tokscale
7. [ ] Verify GitHub Release
   - Tag: vX.Y.Z targeting the bump commit
   - Release notes follow the template above
8. [ ] Smoke test: `bunx tokscale@latest --version`
```
