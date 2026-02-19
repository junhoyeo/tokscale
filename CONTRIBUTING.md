# Contributing to Tokscale

Welcome to the tokscale community!

We're excited to have you here. Whether you're fixing a bug, adding a new feature, or improving documentation, every contribution helps make tokscale better for the entire AI developer community.

## Quick Links

- **GitHub:** https://github.com/junhoyeo/tokscale
- **Issues:** https://github.com/junhoyeo/tokscale/issues
- **Discussions:** https://github.com/junhoyeo/tokscale/discussions

## How to Contribute

1. **Bugs & small fixes** - Open a PR directly!
2. **New features / architecture changes** - Start a [GitHub Discussion](https://github.com/junhoyeo/tokscale/discussions) first
3. **Questions** - Open a Discussion or Issue

## Before Opening an Issue

Please check these common solutions first:

### Checklist

- [ ] **Check you're on the latest version**: `bunx tokscale@latest --version`
- [ ] **Search existing issues**: Your problem may already have a solution
- [ ] **Try with `--light` flag**: If TUI crashes, try `tokscale --light` for simpler output
- [ ] **Verify AI client data exists**: Check that session files exist in the expected locations

### Required Information for Bug Reports

To help us resolve issues quickly, please include:

| Required | How to Get |
|----------|------------|
| **tokscale version** | `tokscale --version` |
| **Bun version** | `bun --version` |
| **OS + Architecture** | e.g., "macOS arm64", "Ubuntu 22.04 x64", "Windows 11" |
| **Installation method** | `bunx tokscale@latest` or cloned from repo |
| **Exact command run** | The full command that caused the issue |
| **Full error output** | Complete error message and stack trace |

### Example of a Good Bug Report

See [#208](https://github.com/junhoyeo/tokscale/issues/208) for an excellent example that includes:
- Clear title with the error message
- Root cause analysis
- Full environment details
- Steps to reproduce
- Expected vs actual behavior

## Common Issues & FAQ

### "Native module required. Run: bun run build:core"

**Cause**: The pre-built native binary for your platform isn't available.

**Solutions**:
1. Make sure you're using `bunx tokscale@latest` (not an older cached version)
2. If on Linux x64/arm64 or Windows, this should work automatically — please [open an issue](https://github.com/junhoyeo/tokscale/issues/new)
3. If building from source: run `bun run build:core` (requires Rust toolchain)

### Windows: "HOME directory not specified"

**Cause**: Windows environment missing `HOME` or `USERPROFILE` variable.

**Solution**: Run in PowerShell (not cmd.exe) or set the HOME environment variable:
```powershell
$env:HOME = $env:USERPROFILE
bunx tokscale@latest
```

### Model pricing shows $0.00

**Common causes**:
1. **New model not yet in pricing database**: Very recently released models may not have pricing data yet
2. **GitHub Copilot resolution**: Some models incorrectly resolve to `github_copilot/*` entries which have $0 pricing

**Solution**: Check the model pricing directly:
```bash
tokscale pricing "model-name"
```
If the price is wrong, [open an issue](https://github.com/junhoyeo/tokscale/issues/new) with the model name and expected price.

### Submit fails with validation errors

**Common causes**:
1. **Unsupported source**: New AI client not yet supported by the server
2. **Negative values**: Edge case in parsing produced negative token counts
3. **Payload too large**: Very large datasets exceeding limits

**Solutions**:
- Update to latest version: `bunx tokscale@latest submit`
- Try with filters: `tokscale submit --claude --since 2024-01-01`
- Use `--dry-run` to preview: `tokscale submit --dry-run`

### OpenCode usage missing after v1.2+

**Cause**: OpenCode 1.2+ stores sessions in SQLite instead of JSON files.

**Solution**: Update tokscale to v1.2.1+ which reads from both SQLite and legacy JSON.

### `--today` shows wrong date (timezone issue)

**Cause**: Date filtering was using UTC instead of local timezone.

**Solution**: Update to v1.2.2+ which uses local timezone for all date operations.

### TUI shows blank/garbled screen on Windows

**Cause**: Windows Terminal compatibility issues with some rendering modes.

**Solution**: 
- Try `tokscale --light` for table output without TUI
- Use Windows Terminal (not cmd.exe or older PowerShell)

## Development Setup

### Prerequisites

- [Bun](https://bun.sh/) (required - runtime and package manager)
- [Rust](https://rustup.rs/) (required for native module development)

### Getting Started

```bash
# Clone the repository
git clone https://github.com/junhoyeo/tokscale.git
cd tokscale

# Install dependencies
bun install

# Build the native Rust core
bun run build:core

# Run the CLI in development mode
bun run cli
```

### Project Structure

```
tokscale/
├── packages/
│   ├── core/       # Native Rust module (NAPI-RS bindings)
│   ├── cli/        # TypeScript CLI application
│   └── tokscale/   # npm alias package
├── scripts/        # Build and utility scripts
└── .github/        # GitHub workflows and assets
```

### Available Scripts

| Command | Description |
|---------|-------------|
| `bun run cli` | Run CLI in development mode |
| `bun run build` | Build both core and CLI |
| `bun run build:core` | Build native Rust module |
| `bun run build:cli` | Build CLI TypeScript |
| `bun run dev:frontend` | Run frontend dev server |

### Testing

```bash
# Run Rust tests
cd packages/core
cargo test --features noop

# Run Node.js integration tests
bun run test

# Run all tests
bun run test:all

# Run benchmarks
bun run bench
```

## Before You PR

- [ ] Test locally with your changes
- [ ] Run tests: `cd packages/core && bun run test:all`
- [ ] Ensure CI checks pass
- [ ] Keep PRs focused (one thing per PR; do not mix unrelated concerns)
- [ ] Write clear commit messages following [conventional commits](#commit-conventions)
- [ ] Update documentation if needed

## Commit Conventions

We use [Conventional Commits](https://www.conventionalcommits.org/) for clear and consistent commit history.

### Format

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### Types

| Type | Description |
|------|-------------|
| `feat` | New feature |
| `fix` | Bug fix |
| `docs` | Documentation only changes |
| `style` | Code style changes (formatting, semicolons, etc.) |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `perf` | Performance improvement |
| `test` | Adding or correcting tests |
| `chore` | Changes to build process or auxiliary tools |
| `ci` | Changes to CI configuration |

### Examples

```
feat(cli): add support for new AI client
fix(core): handle empty session files gracefully
docs: update installation instructions
chore: bump dependencies
```

## Contributing with AI Coding Agents

**AI-assisted contributions are welcome and encouraged!**

Whether you're using [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Codex CLI](https://github.com/openai/codex), [Gemini CLI](https://github.com/google-gemini/gemini-cli), [OpenCode](https://github.com/sst/opencode), [oh-my-claudecode](https://github.com/anthropics/omc), or any other AI coding assistant, your contributions are valued here.

### Best Practices for AI-Assisted Contributions

1. **Understand the code** - Review what the AI generates and ensure you understand the changes
2. **Test thoroughly** - AI-generated code should be tested just like any other code
3. **Review for quality** - Check for edge cases, error handling, and code style consistency
4. **Be transparent** - Optionally note in your PR if you used AI assistance

### What Makes a Good AI-Assisted PR

- [ ] Code has been reviewed and understood by the contributor
- [ ] Tests are included and passing
- [ ] Code follows existing patterns and conventions
- [ ] Edge cases have been considered
- [ ] No hallucinated APIs or non-existent dependencies

### Tips for AI Agents

If you're an AI coding agent working on this codebase:

1. **Read AGENTS.md** - Contains project-specific context for AI agents
2. **Follow existing patterns** - Check similar files before implementing new features
3. **Run tests** - Always verify changes work with `bun run test:all`
4. **Use conventional commits** - Follow the commit format described above
5. **Keep changes focused** - One logical change per PR

## Code Style

- **TypeScript**: Follow existing patterns in `packages/cli/src/`
- **Rust**: Run `cargo fmt` and `cargo clippy` before committing
- **Keep files focused** - Prefer smaller, single-purpose modules
- **Document public APIs** - Add JSDoc/rustdoc comments for public functions

## Adding Support for New AI Clients

Tokscale supports multiple AI coding assistants. To add support for a new client:

1. Create a new parser in `packages/core/src/parsers/`
2. Follow the existing parser patterns (OpenCode, Claude, Codex, etc.)
3. Add tests in `packages/core/__test__/`
4. Update the README to document the new client
5. Add the client logo to `.github/assets/`

## Reporting Security Issues

For security vulnerabilities, please email the maintainer directly rather than opening a public issue. See the repository's security policy for details.

## License

By contributing to tokscale, you agree that your contributions will be licensed under the MIT License.

---

Thank you for contributing to tokscale! Your efforts help the entire AI developer community track and optimize their token usage.
