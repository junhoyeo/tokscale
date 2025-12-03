# Token Tracker - Development Notepad

## Project Overview
CLI tool to track token usage across OpenCode, Claude Code, Codex, and Gemini sessions.

---

## NOTEPAD

[2025-12-02 02:21] - Task 1.1: Create benchmarks/ directory structure

### DISCOVERED ISSUES
- None - clean project structure

### IMPLEMENTATION DECISIONS
- Created `benchmarks/results/` with `.gitkeep` to preserve directory in git
- Added `benchmarks/results/*` and `benchmarks/synthetic-data/` to `.gitignore`
- Results will be JSON files, synthetic data will be generated on-demand

### PROBLEMS FOR NEXT TASKS
- None identified

### VERIFICATION RESULTS
- Directory structure verified: `benchmarks/results/.gitkeep` exists
- .gitignore updated successfully

### LEARNINGS
- Project uses yarn (yarn.lock present)
- Build command: `tsx src/cli.ts` (direct execution, no compile step)
- Current structure is simple - no existing test framework

소요 시간: ~2 minutes

---

[2025-12-02 02:23] - Task 1.2: Implement benchmarks/generate.ts

### DISCOVERED ISSUES
- None

### IMPLEMENTATION DECISIONS
- Created comprehensive synthetic data generator with 4 source types
- Default scale generates ~5,900 messages total:
  - OpenCode: 500 messages (50 sessions × 10 messages)
  - Claude: 2,500 assistant entries (10 projects × 5 files × 100 entries, 50% assistant)
  - Codex: 2,400 token events (30 sessions × 80 events)
  - Gemini: 500 messages (5 projects × 4 sessions × 50 messages, 50% gemini)
- Added --scale flag for CI to run larger benchmarks
- Added --output flag for custom output directory
- Date range: 6 months (2025-06-01 to 2025-12-01)

### PROBLEMS FOR NEXT TASKS
- Synthetic data mimics real format but runner needs to support custom data paths

### VERIFICATION RESULTS
- Ran: `npx tsx benchmarks/generate.ts`
- Output: 601 files generated in benchmarks/synthetic-data/
- Verified file formats match real data structures for all 4 sources

### LEARNINGS
- OpenCode stores individual JSON files per message
- Claude/Codex use JSONL format (one JSON per line)
- Gemini stores entire sessions as single JSON files
- Codex has stateful parsing (turn_context sets model, token_count tracks deltas)

소요 시간: ~5 minutes

---

[2025-12-02 02:25] - Task 1.3: Implement benchmarks/runner.ts

### DISCOVERED ISSUES
- First iteration is always slow (~1100ms) due to module loading
- Need warmup iterations for accurate benchmarks

### IMPLEMENTATION DECISIONS
- Measures wall-clock time using performance.now()
- Measures peak memory using process.memoryUsage().rss
- Supports --synthetic flag for CI (overrides HOME, XDG_DATA_HOME, CODEX_HOME)
- Supports --iterations and --warmup flags for statistical accuracy
- Calculates min, max, median, mean, stdDev for all metrics
- Saves JSON results to benchmarks/results/

### PROBLEMS FOR NEXT TASKS
- Need to add npm scripts for convenience

### VERIFICATION RESULTS
- Ran: `npx tsx benchmarks/runner.ts --synthetic --iterations 3 --warmup 1`
- TypeScript baseline (synthetic data, 5900 messages):
  - Wall-clock: ~79ms median (7.4ms stddev)
  - Peak memory: ~371MB
  - This is our baseline for comparison with Rust

### LEARNINGS
- Warmup is crucial: first iteration includes module loading (~1000ms overhead)
- With warmup, TypeScript processes 5900 messages in ~80ms
- That's ~74,000 messages/second throughput
- Memory usage is high (371MB) - opportunity for Rust optimization

소요 시간: ~5 minutes

---

[2025-12-02 02:26] - Task 1.4: Add npm scripts for benchmarking

### DISCOVERED ISSUES
- None

### IMPLEMENTATION DECISIONS
- Added 5 npm scripts to package.json:
  - `bench:generate`: Generate synthetic benchmark data
  - `bench:ts`: Run TypeScript benchmark with real data
  - `bench:ts:synthetic`: Run TypeScript benchmark with synthetic data (5 iterations, 1 warmup)
  - `bench:rust`: Run Rust benchmark with real data (future)
  - `bench:rust:synthetic`: Run Rust benchmark with synthetic data (future)

### PROBLEMS FOR NEXT TASKS
- None

### VERIFICATION RESULTS
- Ran: `yarn bench:ts:synthetic`
- Output: Benchmark completed successfully, results saved to benchmarks/results/
- Ran: `yarn bench:generate --help`
- Output: Help message displayed correctly

### LEARNINGS
- yarn script arguments pass through correctly
- 5 iterations with 1 warmup is a good default for CI

소요 시간: ~2 minutes

---

[2025-12-02 02:27] - Task 1.5: Run baseline benchmark with real data

### DISCOVERED ISSUES
- None

### IMPLEMENTATION DECISIONS
- Ran benchmark with user's actual session data
- 3 iterations with 1 warmup for consistency

### PROBLEMS FOR NEXT TASKS
- None

### VERIFICATION RESULTS
- Ran: `yarn bench:ts` (real data benchmark)
- TypeScript baseline (REAL data, 18,671 messages):
  - Wall-clock: **1805.63ms median** (75.71ms stddev)
  - Peak memory: **575MB**
  - Sources: Claude, Gemini, OpenCode (19 days of data)
  - Throughput: ~10,340 messages/second

### LEARNINGS
- Real data performance comparison:
  - Synthetic: 5,900 messages in 79ms = 74,683 msg/sec
  - Real: 18,671 messages in 1,806ms = 10,340 msg/sec
- Real data is 7x slower per message than synthetic!
- This suggests:
  1. Real files are larger (more content per message)
  2. File I/O is the bottleneck (more files to read)
  3. Significant room for Rust optimization
- Target for Rust implementation: sub-200ms for real data (~8-9x speedup)

소요 시간: ~2 minutes

---

[2025-12-02 02:29] - Task 1.6: Document in benchmarks/README.md

### DISCOVERED ISSUES
- None

### IMPLEMENTATION DECISIONS
- Created comprehensive README with:
  - Quick start commands
  - Tool documentation (generate.ts, runner.ts)
  - Baseline results tables
  - Target performance goals
  - CI integration example
  - Result file format explanation

### PROBLEMS FOR NEXT TASKS
- None - Phase 1 complete!

### VERIFICATION RESULTS
- README.md created at benchmarks/README.md
- Documents all benchmark scripts and usage

### LEARNINGS
- Phase 1 complete - benchmark infrastructure ready
- Baseline established:
  - TypeScript: 1,806ms for 18,671 real messages
  - Target: ~200ms with Rust (9x speedup)

소요 시간: ~2 minutes

---

## PHASE 1 COMPLETE

**Summary:**
- Created benchmark infrastructure with synthetic data generator and runner
- Established TypeScript baselines:
  - Synthetic (5,900 msgs): 79ms median
  - Real (18,671 msgs): 1,806ms median
- Target for Rust: ~8-9x speedup
- Ready to proceed with Phase 2: Rust Project Setup

---

[2025-12-02 08:06] - Phase 2: Rust Project Setup (Tasks 2.1-2.7)

### DISCOVERED ISSUES
- napi-rs CLI `new` command requires interactive input, manual setup was easier
- napi config format changed: `name` → `binaryName`, `triples` → `targets`
- Default `triples.defaults` causes duplicate target errors

### IMPLEMENTATION DECISIONS
- Created manual project structure for better control:
  ```
  core/
  ├── Cargo.toml      (napi 2.x, rayon, simd-json, walkdir, chrono)
  ├── build.rs        (napi-build setup)
  ├── package.json    (npm package config with targets)
  ├── index.js        (platform-aware native binding loader)
  ├── index.d.ts      (TypeScript type definitions)
  └── src/
      ├── lib.rs          (NAPI exports, types)
      ├── scanner.rs      (parallel file discovery)
      ├── parser.rs       (SIMD JSON parsing)
      ├── aggregator.rs   (parallel map-reduce)
      └── sessions/
          ├── mod.rs          (UnifiedMessage type)
          ├── opencode.rs     (OpenCode parser)
          ├── claudecode.rs   (Claude parser)
          ├── codex.rs        (Codex parser - stateful)
          └── gemini.rs       (Gemini parser)
  ```

### PROBLEMS FOR NEXT TASKS
- Many warnings about unused code (expected - not wired together yet)
- Need to add workspace config to root package.json

### VERIFICATION RESULTS
- `cargo check`: All code compiles ✅
- `yarn build:debug`: Native module built (904KB .node file) ✅
- Node.js test:
  ```
  Version: 0.1.0
  Health: token-tracker-core is healthy!
  ```

### LEARNINGS
- napi-rs 2.x works well with Rust 1.92 nightly
- simd-json requires mutable byte slice (`&mut [u8]`)
- NAPI object struct fields use snake_case in Rust, camelCase in JS
- Build time: ~15 seconds for debug, expect longer for release with LTO
- Native module size: 904KB (debug), expect smaller with release + strip

소요 시간: ~15 minutes

---

[2025-12-03] - Primer Design System Integration

### DISCOVERED ISSUES
- `styled-components@5` requires SSR registry for Next.js App Router
- Primer's `ProgressBar.Item` doesn't support `sx` prop (use inline styles)
- Need `react-is` package for styled-components compatibility

### IMPLEMENTATION DECISIONS
- Selective Primer adoption (not full migration) to preserve existing design
- Created provider stack: `StyledComponentsRegistry` → `PrimerProvider`
- Components migrated:
  - **SegmentedControl**: Period filter on leaderboard
  - **Pagination**: Page navigation
  - **Avatar**: User avatars throughout
  - **ActionMenu + ActionList**: User dropdown menu
  - **Label**: Badges for rank, sources, models
- Kept custom components: Contribution graph, stat cards, token breakdown bar

### FILES CREATED
- `frontend/src/lib/providers/StyledComponentsRegistry.tsx` - SSR support
- `frontend/src/lib/providers/PrimerProvider.tsx` - Theme integration
- `frontend/src/lib/providers/Providers.tsx` - Combined provider
- `frontend/src/lib/providers/index.ts` - Exports

### PACKAGES ADDED
```json
{
  "@primer/react": "^38.3.0",
  "@primer/primitives": "^11.3.1",
  "styled-components": "^5.3.11",
  "react-is": "^19.2.0",
  "@types/styled-components": "^5.1.36"
}
```

### VERIFICATION RESULTS
- TypeScript: ✅ No errors
- Build: ✅ All 18 pages generated successfully
- ESLint: Minor warnings (pre-existing, not from Primer)

### LEARNINGS
- Primer's `colorMode` values: `'day' | 'night' | 'auto'`
- Use `preventSSRMismatch` on ThemeProvider for hydration
- `ActionMenu` is self-contained (manages its own open state)
- Primer CSS imports: `@primer/primitives/dist/css/functional/themes/*.css`
- Data visualization colors: `--data-{color}-color-emphasis` CSS variables

소요 시간: ~20 minutes

