# Benchmark Infrastructure

This directory contains tools for measuring and comparing the performance of the Token Tracker CLI.

## Overview

The benchmark infrastructure measures:
- **Wall-clock time** (primary metric): Total execution time
- **Peak memory usage** (secondary metric): Maximum RSS during execution

## Quick Start

```bash
# Generate synthetic benchmark data
yarn bench:generate

# Run benchmark with synthetic data (recommended for CI)
yarn bench:ts:synthetic

# Run benchmark with your real session data
yarn bench:ts

# (Future) Run Rust implementation benchmark
yarn bench:rust:synthetic
```

## Benchmark Tools

### `generate.ts` - Synthetic Data Generator

Generates reproducible test data that mimics real session files.

```bash
# Generate default dataset (~6,000 messages)
yarn bench:generate

# Generate larger dataset (2x scale)
npx tsx benchmarks/generate.ts --scale 2

# Generate to custom directory
npx tsx benchmarks/generate.ts --output /tmp/bench-data
```

**Default data volume:**
| Source   | Files | Messages |
|----------|-------|----------|
| OpenCode | 500   | 500      |
| Claude   | 50    | 2,500    |
| Codex    | 30    | 2,400    |
| Gemini   | 20    | 500      |
| **Total**| 600   | ~5,900   |

### `runner.ts` - Benchmark Runner

Measures performance of the `graph` command.

```bash
# Run with defaults (3 iterations, 1 warmup)
npx tsx benchmarks/runner.ts

# Use synthetic data
npx tsx benchmarks/runner.ts --synthetic

# More iterations for accuracy
npx tsx benchmarks/runner.ts --iterations 10 --warmup 2

# Test Rust implementation (when available)
npx tsx benchmarks/runner.ts --implementation rust
```

**Options:**
| Flag | Description | Default |
|------|-------------|---------|
| `--synthetic` | Use synthetic data | false |
| `--iterations <n>` | Number of benchmark runs | 3 |
| `--warmup <n>` | Warmup iterations (not counted) | 1 |
| `--implementation <ts\|rust>` | Which implementation to test | typescript |
| `--output <dir>` | Directory for results | ./benchmarks/results |

## Baseline Results

Initial TypeScript implementation baseline (as of 2025-12-02):

### Synthetic Data (5,900 messages)

| Metric | Value |
|--------|-------|
| Wall-clock (median) | ~79ms |
| Wall-clock (stddev) | ~7ms |
| Peak memory | ~371MB |
| Throughput | ~74,000 msg/sec |

### Real Data (18,671 messages)

| Metric | Value |
|--------|-------|
| Wall-clock (median) | ~1,806ms |
| Wall-clock (stddev) | ~76ms |
| Peak memory | ~575MB |
| Throughput | ~10,340 msg/sec |

### Target (Rust Implementation)

| Metric | TypeScript | Target (Rust) | Expected Speedup |
|--------|------------|---------------|------------------|
| Real data (18k msgs) | 1,806ms | ~200ms | ~9x |
| Synthetic (6k msgs) | 79ms | ~10ms | ~8x |
| Peak memory | 575MB | ~50MB | ~11x |

## Result Files

Benchmark results are saved to `benchmarks/results/` as JSON:

```
benchmark-{implementation}-{dataSource}-{timestamp}.json
```

Example:
```json
{
  "implementation": "typescript",
  "command": "graph",
  "dataSource": "real",
  "timestamp": "2025-12-02T17:27:52.334Z",
  "summary": {
    "wallClockMs": {
      "min": 1711.54,
      "max": 1896.98,
      "median": 1805.63,
      "mean": 1804.72,
      "stdDev": 75.71
    },
    "peakMemoryMb": {
      "median": 575.0
    }
  },
  "data": {
    "totalMessages": 18671,
    "totalDays": 19,
    "sources": ["claude", "gemini", "opencode"]
  }
}
```

## CI Integration

For CI pipelines, use synthetic data for reproducible benchmarks:

```yaml
# .github/workflows/benchmark.yml
jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - run: yarn install
      - run: yarn bench:generate
      - run: yarn bench:ts:synthetic
      - uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: benchmarks/results/
```

## Adding New Benchmarks

To add a new benchmark scenario:

1. Modify `generate.ts` to include new data patterns
2. Update `runner.ts` to measure new metrics if needed
3. Run benchmarks and update baseline in this README
