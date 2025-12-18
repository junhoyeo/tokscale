<!-- <CENTERED SECTION FOR GITHUB DISPLAY> -->

<div align="center">

[![Tokscale](./.github/assets/hero.png)](https://github.com/junhoyeo/tokscale#tokscale)

</div>

> A high-performance CLI tool and visualization dashboard for tracking AI coding assistant token usage and costs across multiple platforms.

<div align="center">

[![GitHub Release](https://img.shields.io/github/v/release/junhoyeo/tokscale?color=0073FF&labelColor=black&logo=github&style=flat-square)](https://github.com/junhoyeo/tokscale/releases)
[![GitHub Contributors](https://img.shields.io/github/contributors/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/graphs/contributors)
[![GitHub Forks](https://img.shields.io/github/forks/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/network/members)
[![GitHub Stars](https://img.shields.io/github/stars/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/stargazers)
[![GitHub Issues](https://img.shields.io/github/issues/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/issues)
[![License](https://img.shields.io/badge/license-MIT-white?labelColor=black&style=flat-square)](https://github.com/junhoyeo/tokscale/blob/master/LICENSE)

[English](README.md)

</div>

<!-- </CENTERED SECTION FOR GITHUB DISPLAY> -->

## Overview

**Tokscale** helps you monitor and analyze your token consumption from:

| Logo | Client | Data Location | Supported |
|------|----------|---------------|-----------|
| <img width="48px" src=".github/assets/client-opencode.png" alt="OpenCode" /> | [OpenCode](https://github.com/sst/opencode) | `~/.local/share/opencode/storage/message/` | ✅ Yes |
| <img width="48px" src=".github/assets/client-claude.jpg" alt="Claude" /> | [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `~/.claude/projects/` | ✅ Yes |
| <img width="48px" src=".github/assets/client-openai.jpg" alt="Codex" /> | [Codex CLI](https://github.com/openai/codex) | `~/.codex/sessions/` | ✅ Yes |
| <img width="48px" src=".github/assets/client-gemini.png" alt="Gemini" /> | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `~/.gemini/tmp/*/chats/` | ✅ Yes |
| <img width="48px" src=".github/assets/client-cursor.jpg" alt="Cursor" /> | [Cursor IDE](https://cursor.com/) | API sync via `~/.config/tokscale/cursor-cache/` | ✅ Yes |

Get real-time pricing calculations using [🚅 LiteLLM's pricing data](https://github.com/BerriAI/litellm), with support for tiered pricing models and cache token discounts.

## Features

- **Interactive TUI Mode** - Beautiful terminal UI powered by OpenTUI (default mode)
  - 4 interactive views: Overview, Models, Daily, Stats
  - Keyboard & mouse navigation
  - GitHub-style contribution graph with 9 color themes
  - Real-time filtering and sorting
  - Zero flicker rendering (native Zig engine)
- **Multi-platform support** - Track usage across OpenCode, Claude Code, Codex CLI, Cursor IDE, and Gemini CLI
- **Real-time pricing** - Fetches current pricing from LiteLLM with 1-hour disk cache
- **Detailed breakdowns** - Input, output, cache read/write, and reasoning token tracking
- **Native Rust core** - All parsing and aggregation done in Rust for 10x faster processing
- **Web visualization** - Interactive contribution graph with 2D and 3D views
- **Flexible filtering** - Filter by platform, date range, or year
- **Export to JSON** - Generate data for external visualization tools
- **Social Platform** - Share your usage, compete on leaderboards, and view public profiles

## Installation

### Prerequisites

- Node.js 18+
- Bun (for TUI development mode)
- (Optional) Rust toolchain for native module

### Quick Start

```bash
# Install globally via npm
npm install -g tokscale

# Or use the scoped package name (same thing)
npm install -g @tokscale/cli

# Run the CLI
tokscale
```

> **Package Structure**: `tokscale` is an alias package (like [`swc`](https://www.npmjs.com/package/swc)) that installs `@tokscale/cli`. Both commands install the same CLI tool with the native Rust core (`@tokscale/core`) included as a dependency—similar to SWC's package structure.

### Development Setup

For local development or building from source:

```bash
# Clone the repository
git clone https://github.com/junhoyeo/tokscale.git
cd tokscale

# Install Bun (if not already installed)
curl -fsSL https://bun.sh/install | bash

# Install dependencies
bun install

# Run the CLI in development mode
bun run cli
```

> **Note**: `bun run cli` is for local development. When installed globally via npm, the command is `tokscale`. The Usage section below shows the installed binary commands.

### Building the Native Module (Optional)

The native Rust module provides ~10x faster processing through parallel file scanning and SIMD JSON parsing:

```bash
# Build the native core (run from repository root)
bun run build:core

# Verify installation
bun run cli graph --benchmark
```

## Usage

### Basic Commands

```bash
# Launch interactive TUI (default)
tokscale

# Launch TUI with specific tab
tokscale models    # Models tab
tokscale monthly   # Daily view (shows daily breakdown)

# Use legacy CLI table output
tokscale --light
tokscale models --light

# Launch TUI explicitly
tokscale tui

# Export contribution graph data as JSON
tokscale graph --output data.json

# Output data as JSON (for scripting/automation)
tokscale --json                    # Default models view as JSON
tokscale models --json             # Models breakdown as JSON
tokscale monthly --json            # Monthly breakdown as JSON
tokscale models --json > report.json   # Save to file
```

### TUI Features

The interactive TUI mode provides:

- **4 Views**: Overview (chart + top models), Models, Daily, Stats (contribution graph)
- **Keyboard Navigation**:
  - `1-4` or `←/→/Tab`: Switch views
  - `↑/↓`: Navigate lists
  - `c/n/t`: Sort by cost/name/tokens
  - `1-5`: Toggle sources (OpenCode/Claude/Codex/Cursor/Gemini)
  - `p`: Cycle through 9 color themes
  - `r`: Refresh data
  - `e`: Export to JSON
  - `q`: Quit
- **Mouse Support**: Click tabs, buttons, and filters
- **Themes**: Green, Halloween, Teal, Blue, Pink, Purple, Orange, Monochrome, YlGnBu
- **Settings Persistence**: Theme preference saved to `~/.config/tokscale/tui-settings.json`

### Filtering by Platform

```bash
# Show only OpenCode usage
tokscale --opencode

# Show only Claude Code usage
tokscale --claude

# Show only Codex CLI usage
tokscale --codex

# Show only Gemini CLI usage
tokscale --gemini

# Show only Cursor IDE usage (requires `tokscale cursor login` first)
tokscale --cursor

# Combine filters
tokscale --opencode --claude
```

### Date Filtering

Date filters work across all commands that generate reports (`tokscale`, `tokscale models`, `tokscale monthly`, `tokscale graph`):

```bash
# Quick date shortcuts
tokscale --today              # Today only
tokscale --week               # Last 7 days
tokscale --month              # Current calendar month

# Custom date range (inclusive, local timezone)
tokscale --since 2024-01-01 --until 2024-12-31

# Filter by year
tokscale --year 2024

# Combine with other options
tokscale models --week --claude --json
tokscale monthly --month --benchmark
```

> **Note**: Date filters use your local timezone. Both `--since` and `--until` are inclusive.

### Graph Command Options

```bash
# Export graph data to file
tokscale graph --output usage-data.json

# Date filtering (all shortcuts work)
tokscale graph --today
tokscale graph --week
tokscale graph --since 2024-01-01 --until 2024-12-31
tokscale graph --year 2024

# Filter by platform
tokscale graph --opencode --claude

# Show processing time benchmark
tokscale graph --output data.json --benchmark
```

### Benchmark Flag

Show processing time for performance analysis:

```bash
tokscale --benchmark           # Show processing time with default view
tokscale models --benchmark    # Benchmark models report
tokscale monthly --benchmark   # Benchmark monthly report
tokscale graph --benchmark     # Benchmark graph generation
```

### Social Platform Commands

```bash
# Login to Tokscale (opens browser for GitHub auth)
tokscale login

# Check who you're logged in as
tokscale whoami

# Submit your usage data to the leaderboard
tokscale submit

# Submit with filters
tokscale submit --opencode --claude --since 2024-01-01

# Preview what would be submitted (dry run)
tokscale submit --dry-run

# Logout
tokscale logout
```

### Cursor IDE Commands

Cursor IDE requires separate authentication via session token (different from the social platform login):

```bash
# Login to Cursor (requires session token from browser)
tokscale cursor login

# Check Cursor authentication status and session validity
tokscale cursor status

# Logout from Cursor (removes saved credentials)
tokscale cursor logout
```

**Credentials storage**: Cursor session token is saved to `~/.config/tokscale/cursor-credentials.json`. Usage data is cached at `~/.config/tokscale/cursor-cache/`.

**To get your Cursor session token:**
1. Open https://www.cursor.com/settings in your browser
2. Open Developer Tools (F12)
3. **Option A - Network tab**: Make any action on the page, find a request to `cursor.com/api/*`, look in the Request Headers for the `Cookie` header, and copy only the value after `WorkosCursorSessionToken=`
4. **Option B - Application tab**: Go to Application → Cookies → `https://www.cursor.com`, find the `WorkosCursorSessionToken` cookie, and copy its value (not the cookie name)

> ⚠️ **Security Warning**: Treat your session token like a password. Never share it publicly or commit it to version control. The token grants full access to your Cursor account.

### Example Output (`--light` version)

```
  Token Usage Report by Model

┌──────────────────────┬─────────────────┬───────────┬───────────┬─────────────┬────────────┬───────────┬──────────┐
│ Source/Model         │ Models          │     Input │    Output │ Cache Write │ Cache Read │     Total │     Cost │
├──────────────────────┼─────────────────┼───────────┼───────────┼─────────────┼────────────┼───────────┼──────────┤
│ Claude sonnet-4      │ - sonnet-4      │ 2,456,789 │   345,678 │      89,012 │  1,234,567 │ 4,126,046 │   $45.67 │
├──────────────────────┼─────────────────┼───────────┼───────────┼─────────────┼────────────┼───────────┼──────────┤
│ OpenCode opus-4-5    │ - opus-4-5      │   567,890 │   123,456 │      12,345 │    456,789 │ 1,160,480 │   $23.45 │
├──────────────────────┼─────────────────┼───────────┼───────────┼─────────────┼────────────┼───────────┼──────────┤
│ Total                │                 │ 3,024,679 │   469,134 │     101,357 │  1,691,356 │ 5,286,526 │   $69.12 │
└──────────────────────┴─────────────────┴───────────┴───────────┴─────────────┴────────────┴───────────┴──────────┘

  Total: 156 messages, 5,286,526 tokens, $69.12
```

## Architecture

```
tokscale/
├── packages/
│   ├── cli/src/            # TypeScript CLI
│   │   ├── cli.ts          # Commander.js entry point
│   │   ├── tui/            # OpenTUI interactive interface
│   │   │   ├── App.tsx     # Main TUI app (Solid.js)
│   │   │   ├── components/ # TUI components
│   │   │   ├── hooks/      # Data fetching & state
│   │   │   ├── config/     # Themes & settings
│   │   │   └── utils/      # Formatting utilities
│   │   ├── sessions/       # Platform session parsers
│   │   │   ├── claudecode.ts  # Claude Code parser
│   │   │   ├── codex.ts       # Codex CLI parser
│   │   │   ├── gemini.ts      # Gemini CLI parser
│   │   │   └── opencode.ts    # OpenCode parser
│   │   ├── cursor.ts       # Cursor IDE integration
│   │   ├── graph.ts        # Graph data generation
│   │   ├── pricing.ts      # LiteLLM pricing fetcher
│   │   └── native.ts       # Native module loader
│   │
│   ├── core/               # Rust native module (napi-rs)
│   │   ├── src/
│   │   │   ├── lib.rs      # NAPI exports
│   │   │   ├── scanner.rs  # Parallel file discovery
│   │   │   ├── parser.rs   # SIMD JSON parsing
│   │   │   ├── aggregator.rs # Parallel aggregation
│   │   │   ├── pricing.rs  # Cost calculation
│   │   │   └── sessions/   # Platform-specific parsers
│   │   ├── Cargo.toml
│   │   └── package.json
│   │
│   ├── frontend/           # Next.js visualization & social platform
│   │   └── src/
│   │       ├── app/        # Next.js app router
│   │       └── components/ # React components
│   │
│   └── benchmarks/         # Performance benchmarks
│       ├── runner.ts       # Benchmark harness
│       └── generate.ts     # Synthetic data generator
```

### Hybrid TypeScript + Rust Architecture

Tokscale uses a hybrid architecture for optimal performance:

1. **TypeScript Layer**: CLI interface, pricing fetch (with disk cache), output formatting
2. **Rust Native Core**: ALL parsing, cost calculation, and aggregation

```
┌─────────────────────────────────────────────────────────────┐
│                     TypeScript (CLI)                        │
│  • Fetch pricing from LiteLLM (cached to disk, 1hr TTL)     │
│  • Pass pricing data to Rust                                │
│  • Display formatted results                                │
└─────────────────────┬───────────────────────────────────────┘
                      │ pricing entries
                      ▼
┌─────────────────────────────────────────────────────────────┐
│                    Rust Native Core                         │
│  • Parallel file scanning (rayon)                           │
│  • SIMD JSON parsing (simd-json)                            │
│  • Cost calculation with pricing data                       │
│  • Parallel aggregation by model/month/day                  │
└─────────────────────────────────────────────────────────────┘
```

All heavy computation is done in Rust when the native module is available. When the native module is not installed, the CLI automatically falls back to TypeScript implementations for full compatibility (with slower performance).

### Key Technologies

| Layer | Technology | Purpose |
|-------|------------|---------|
| CLI | [Commander.js](https://github.com/tj/commander.js) | Command-line parsing |
| TUI | [OpenTUI](https://github.com/sst/opentui) + [Solid.js](https://www.solidjs.com/) | Interactive terminal UI (zero-flicker rendering) |
| Runtime | [Bun](https://bun.sh/) | Fast JavaScript runtime (required for TUI dev mode) |
| Tables | [cli-table3](https://github.com/cli-table/cli-table3) | Terminal table rendering (legacy CLI) |
| Colors | [picocolors](https://github.com/alexeyraspopov/picocolors) | Terminal colors |
| Native | [napi-rs](https://napi.rs/) | Node.js bindings for Rust |
| Parallelism | [Rayon](https://github.com/rayon-rs/rayon) | Data parallelism in Rust |
| JSON | [simd-json](https://github.com/simd-lite/simd-json) | SIMD-accelerated parsing |
| Frontend | [Next.js 16](https://nextjs.org/) | React framework |
| 3D Viz | [obelisk.js](https://github.com/nicklockwood/obelisk.js) | Isometric 3D rendering |

## Performance

The native Rust module provides significant performance improvements:

| Operation | TypeScript | Rust Native | Speedup |
|-----------|------------|-------------|---------|
| File Discovery | ~500ms | ~50ms | **10x** |
| JSON Parsing | ~800ms | ~100ms | **8x** |
| Aggregation | ~200ms | ~25ms | **8x** |
| **Total** | **~1.5s** | **~175ms** | **~8.5x** |

*Benchmarks for ~1000 session files, 100k messages*

### Memory Optimization

The native module also provides ~45% memory reduction through:

- Streaming JSON parsing (no full file buffering)
- Zero-copy string handling
- Efficient parallel aggregation with map-reduce

### Running Benchmarks

```bash
# Generate synthetic data
cd packages/benchmarks && bun run generate

# Run Rust benchmarks
cd packages/core && bun run bench
```

## Frontend Visualization

The frontend provides a GitHub-style contribution graph visualization:

### Features

- **2D View**: Classic GitHub contribution calendar
- **3D View**: Isometric 3D contribution graph with height based on token usage
- **Multiple color palettes**: GitHub, GitLab, Halloween, Winter, and more
- **3-way theme toggle**: Light / Dark / System (follows OS preference)
- **GitHub Primer design**: Uses GitHub's official color system
- **Interactive tooltips**: Hover for detailed daily breakdowns
- **Day breakdown panel**: Click to see per-source and per-model details
- **Year filtering**: Navigate between years
- **Source filtering**: Filter by platform (OpenCode, Claude, Codex, Cursor, Gemini)
- **Stats panel**: Total cost, tokens, active days, streaks
- **FOUC prevention**: Theme applied before React hydrates (no flash)

### Running the Frontend

```bash
cd packages/frontend
bun install
bun run dev
```

Open [http://localhost:3000](http://localhost:3000) to access the social platform.

## Social Platform

Tokscale includes a social platform where you can share your usage data and compete with other developers.

### Features

- **Leaderboard** - See who's using the most tokens across all platforms
- **User Profiles** - Public profiles with contribution graphs and statistics
- **Period Filtering** - View stats for all time, this month, or this week
- **GitHub Integration** - Login with your GitHub account
- **Local Viewer** - View your data privately without submitting

### Getting Started

1. **Login** - Run `tokscale login` to authenticate via GitHub
2. **Submit** - Run `tokscale submit` to upload your usage data
3. **View** - Visit the web platform to see your profile and the leaderboard

### Data Validation

Submitted data goes through Level 1 validation:
- Mathematical consistency (totals match, no negatives)
- No future dates
- Required fields present
- Duplicate detection

### Self-Hosting

To run your own instance:

1. Set up a PostgreSQL database (Neon, Vercel Postgres, or self-hosted)
2. Configure environment variables:
   ```bash
   DATABASE_URL=postgresql://...
   GITHUB_CLIENT_ID=your_client_id
   GITHUB_CLIENT_SECRET=your_client_secret
   NEXT_PUBLIC_URL=https://your-domain.com
   ```
3. Run database migrations: `cd packages/frontend && npx drizzle-kit push`
4. Deploy to Vercel or your preferred platform

### Generating Data for Frontend

```bash
# Export data for visualization
tokscale graph --output packages/frontend/public/my-data.json
```

## Development

> **Quick setup**: If you just want to get started quickly, see [Development Setup](#development-setup) in the Installation section above.

### Prerequisites

```bash
# Node.js 18+
node --version

# Rust (for native module)
rustc --version
cargo --version
```

### Advanced Development

After following the [Development Setup](#development-setup), you can:

```bash
# Build native module (optional but recommended)
bun run build:core

# Run in development mode (launches TUI)
cd packages/cli && bun src/cli.ts

# Or use legacy CLI mode
cd packages/cli && bun src/cli.ts --light
```

### Project Scripts

| Script | Description |
|--------|-------------|
| `bun run cli` | Run CLI in development mode (TUI with Bun) |
| `bun run build:core` | Build native Rust module (release) |
| `bun run build:cli` | Build CLI TypeScript to dist/ |
| `bun run build` | Build both core and CLI |
| `bun run dev:frontend` | Run frontend development server |

**Package-specific scripts** (from within package directories):
- `packages/cli`: `bun run dev`, `bun run dev:node`, `bun run tui`
- `packages/core`: `bun run build:debug`, `bun run test`, `bun run bench`

**Note**: This project uses **Bun** as the package manager and runtime. TUI requires Bun due to OpenTUI's native modules.

### Testing

```bash
# Test native module (Rust)
cd packages/core
bun run test:rust      # Cargo tests
bun run test           # Node.js integration tests
bun run test:all       # Both
```

### Native Module Development

```bash
cd packages/core

# Build in debug mode (faster compilation)
bun run build:debug

# Build in release mode (optimized)
bun run build

# Run Rust benchmarks
bun run bench
```

## Supported Platforms

### Native Module Targets

| Platform | Architecture | Status |
|----------|--------------|--------|
| macOS | x86_64 | Supported |
| macOS | aarch64 (Apple Silicon) | Supported |
| Linux | x86_64 (glibc) | Supported |
| Linux | aarch64 (glibc) | Supported |
| Linux | x86_64 (musl) | Supported |
| Linux | aarch64 (musl) | Supported |
| Windows | x86_64 | Supported |
| Windows | aarch64 | Supported |

## Session Data Retention

By default, some AI coding assistants automatically delete old session files. To preserve your usage history for accurate tracking, disable or extend the cleanup period.

| Platform | Default | Config File | Setting to Disable | Source |
|----------|---------|-------------|-------------------|--------|
| Claude Code | **⚠️ 30 days** | `~/.claude/settings.json` | `"cleanupPeriodDays": 9999999999` | [Docs](https://docs.anthropic.com/en/docs/claude-code/settings) |
| Gemini CLI | Disabled | `~/.gemini/settings.json` | `"sessionRetention.enabled": false` | [Docs](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md) |
| Codex CLI | Disabled | N/A | No cleanup feature | [#6015](https://github.com/openai/codex/issues/6015) |
| OpenCode | Disabled | N/A | No cleanup feature | [#4980](https://github.com/sst/opencode/issues/4980) |

### Claude Code

**Default**: 30 days cleanup period

Add to `~/.claude/settings.json`:
```json
{
  "cleanupPeriodDays": 9999999999
}
```

> Setting an extremely large value (e.g., `9999999999` days ≈ 27 million years) effectively disables cleanup.

### Gemini CLI

**Default**: Cleanup disabled (sessions persist forever)

If you've enabled cleanup and want to disable it, remove or set `enabled: false` in `~/.gemini/settings.json`:
```json
{
  "general": {
    "sessionRetention": {
      "enabled": false
    }
  }
}
```

Or set an extremely long retention period:
```json
{
  "general": {
    "sessionRetention": {
      "enabled": true,
      "maxAge": "9999999d"
    }
  }
}
```

### Codex CLI

**Default**: No automatic cleanup (sessions persist forever)

Codex CLI does not have built-in session cleanup. Sessions in `~/.codex/sessions/` persist indefinitely.

> **Note**: There's an open feature request for this: [#6015](https://github.com/openai/codex/issues/6015)

### OpenCode

**Default**: No automatic cleanup (sessions persist forever)

OpenCode does not have built-in session cleanup. Sessions in `~/.local/share/opencode/storage/` persist indefinitely.

> **Note**: See [#4980](https://github.com/sst/opencode/issues/4980)

---

## Data Sources

### OpenCode

Location: `~/.local/share/opencode/storage/message/{sessionId}/*.json`

Each message file contains:
```json
{
  "id": "msg_xxx",
  "role": "assistant",
  "modelID": "claude-sonnet-4-20250514",
  "providerID": "anthropic",
  "tokens": {
    "input": 1234,
    "output": 567,
    "reasoning": 0,
    "cache": { "read": 890, "write": 123 }
  },
  "time": { "created": 1699999999999 }
}
```

### Claude Code

Location: `~/.claude/projects/{projectPath}/*.jsonl`

JSONL format with assistant messages containing usage data:
```json
{"type": "assistant", "message": {"model": "claude-sonnet-4-20250514", "usage": {"input_tokens": 1234, "output_tokens": 567, "cache_read_input_tokens": 890}}, "timestamp": "2024-01-01T00:00:00Z"}
```

### Codex CLI

Location: `~/.codex/sessions/*.jsonl`

Event-based format with `token_count` events:
```json
{"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"input_tokens": 1234, "output_tokens": 567}}}}
```

### Gemini CLI

Location: `~/.gemini/tmp/{projectHash}/chats/session-*.json`

Session files containing message arrays:
```json
{
  "sessionId": "xxx",
  "messages": [
    {"type": "gemini", "model": "gemini-2.5-pro", "tokens": {"input": 1234, "output": 567, "cached": 890, "thoughts": 123}}
  ]
}
```

### Cursor IDE

Location: `~/.config/tokscale/cursor-cache/` (synced via Cursor API)

Cursor data is fetched from the Cursor API using your session token and cached locally. Run `tokscale cursor login` to authenticate. See [Cursor IDE Commands](#cursor-ide-commands) for setup instructions.

## Pricing

Tokscale fetches real-time pricing from [LiteLLM's pricing database](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json).

**Caching**: Pricing data is cached to disk at `~/.cache/tokscale/pricing.json` with a 1-hour TTL. This ensures fast startup while keeping pricing data fresh.

Pricing includes:
- Input tokens
- Output tokens
- Cache read tokens (discounted)
- Cache write tokens
- Reasoning tokens (for models like o1)
- Tiered pricing (above 200k tokens)

## Contributing

Contributions are welcome! Please follow these steps:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests (`cd packages/core && bun run test:all`)
5. Commit your changes (`git commit -m 'Add amazing feature'`)
6. Push to the branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

### Development Guidelines

- Follow existing code style
- Add tests for new functionality
- Update documentation as needed
- Keep commits focused and atomic

## Acknowledgments

- [ccusage](https://github.com/ryoppippi/ccusage), [viberank](https://github.com/sculptdotfun/viberank), and [Isometric Contributions](https://github.com/jasonlong/isometric-contributions) for inspiration
- [OpenTUI](https://github.com/sst/opentui) for zero-flicker terminal UI framework
- [Solid.js](https://www.solidjs.com/) for reactive rendering
- [LiteLLM](https://github.com/BerriAI/litellm) for pricing data
- [napi-rs](https://napi.rs/) for Rust/Node.js bindings
- [github-contributions-canvas](https://github.com/sallar/github-contributions-canvas) for 2D graph reference

## License

<p align="center">
  <a href="https://github.com/junhoyeo">
    <img src=".github/assets/labtocat-on-spaceship.png" width="540">
  </a>
</p>

<p align="center">
  <strong>MIT © <a href="https://github.com/junhoyeo">Junho Yeo</a></strong>
</p>

If you find this project intriguing, **please consider starring it ⭐** or [follow me on GitHub](https://github.com/junhoyeo) and join the ride (1.1k+ already aboard). I code around the clock and ship mind-blowing things on a regular basis—your support won't go to waste.
