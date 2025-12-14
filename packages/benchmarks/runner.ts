#!/usr/bin/env npx tsx
/**
 * Benchmark Runner for Token Tracker CLI
 * 
 * Measures performance of the graph command with:
 * - Wall-clock time (primary metric)
 * - Peak memory usage (secondary metric)
 * 
 * Supports both real user data and synthetic benchmark data.
 * 
 * Usage:
 *   npx tsx benchmarks/runner.ts                    # Run with real data
 *   npx tsx benchmarks/runner.ts --synthetic        # Run with synthetic data
 *   npx tsx benchmarks/runner.ts --iterations 5     # Run 5 iterations
 *   npx tsx benchmarks/runner.ts --implementation rust  # Test Rust impl (future)
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import { performance } from "node:perf_hooks";

// =============================================================================
// Types
// =============================================================================

interface BenchmarkConfig {
  implementation: "typescript" | "rust";
  useSyntheticData: boolean;
  syntheticDataPath: string;
  iterations: number;
  warmupIterations: number;
  outputDir: string;
}

interface IterationResult {
  iteration: number;
  wallClockMs: number;
  peakMemoryMb: number;
  heapUsedMb: number;
}

interface BenchmarkResult {
  implementation: "typescript" | "rust";
  command: "graph";
  dataSource: "real" | "synthetic";
  timestamp: string;
  system: {
    platform: string;
    arch: string;
    cpus: number;
    totalMemoryGb: number;
    nodeVersion: string;
  };
  config: {
    iterations: number;
    warmupIterations: number;
  };
  iterations: IterationResult[];
  summary: {
    wallClockMs: {
      min: number;
      max: number;
      median: number;
      mean: number;
      stdDev: number;
    };
    peakMemoryMb: {
      min: number;
      max: number;
      median: number;
      mean: number;
    };
  };
  data: {
    totalFiles: number;
    totalMessages: number;
    totalDays: number;
    sources: string[];
  };
}

// =============================================================================
// Configuration
// =============================================================================

const DEFAULT_CONFIG: BenchmarkConfig = {
  implementation: "typescript",
  useSyntheticData: false,
  syntheticDataPath: "./benchmarks/synthetic-data",
  iterations: 3,
  warmupIterations: 1,
  outputDir: "./benchmarks/results",
};

function parseArgs(): Partial<BenchmarkConfig> {
  const args = process.argv.slice(2);
  const config: Partial<BenchmarkConfig> = {};
  
  for (let i = 0; i < args.length; i++) {
    switch (args[i]) {
      case "--synthetic":
        config.useSyntheticData = true;
        break;
      case "--iterations":
        config.iterations = parseInt(args[++i], 10);
        break;
      case "--warmup":
        config.warmupIterations = parseInt(args[++i], 10);
        break;
      case "--implementation":
        config.implementation = args[++i] as "typescript" | "rust";
        break;
      case "--output":
        config.outputDir = args[++i];
        break;
      case "--help":
        console.log(`
Benchmark Runner for Token Tracker CLI

Usage: npx tsx benchmarks/runner.ts [options]

Options:
  --synthetic          Use synthetic benchmark data instead of real data
  --iterations <n>     Number of benchmark iterations (default: 3)
  --warmup <n>         Number of warmup iterations (default: 1)
  --implementation <t> Implementation to test: typescript | rust (default: typescript)
  --output <dir>       Output directory for results (default: ./benchmarks/results)
  --help               Show this help message

Examples:
  npx tsx benchmarks/runner.ts                        # Benchmark real data
  npx tsx benchmarks/runner.ts --synthetic            # Benchmark synthetic data
  npx tsx benchmarks/runner.ts --iterations 10        # 10 iterations
  npx tsx benchmarks/runner.ts --implementation rust  # Test Rust implementation
`);
        process.exit(0);
    }
  }
  
  return config;
}

// =============================================================================
// Data Path Override for Synthetic Data
// =============================================================================

function setupSyntheticDataPaths(syntheticPath: string): void {
  // Override environment variables to point to synthetic data
  const absPath = path.resolve(syntheticPath);
  
  // Override XDG_DATA_HOME for OpenCode
  process.env.XDG_DATA_HOME = path.join(absPath, ".local/share");
  
  // Override HOME for Claude, Codex, Gemini
  process.env.HOME = absPath;
  
  // Also set CODEX_HOME explicitly
  process.env.CODEX_HOME = path.join(absPath, ".codex");
}

// =============================================================================
// Graph Data Generation (imported from main codebase)
// =============================================================================

async function runGraphGenerationTS(): Promise<{
  data: import("../src/graph-types.js").TokenContributionData;
  files: number;
  messages: number;
}> {
  // Dynamic import to pick up environment variable changes
  const { generateGraphData } = await import("../src/graph.js");
  
  const data = await generateGraphData({});
  
  // Count files and messages
  let totalMessages = 0;
  for (const contrib of data.contributions) {
    totalMessages += contrib.totals.messages;
  }
  
  return {
    data,
    files: 0, // We'll count files separately
    messages: totalMessages,
  };
}

async function runGraphGenerationRust(syntheticPath?: string): Promise<{
  data: import("../src/graph-types.js").TokenContributionData;
  files: number;
  messages: number;
}> {
  // Dynamic import native module
  const { 
    parseLocalSourcesNative, 
    finalizeGraphNative, 
    isNativeAvailable 
  } = await import("../src/native.js");
  
  if (!isNativeAvailable()) {
    throw new Error("Native Rust module not available. Run 'yarn build:core' first.");
  }
  
  // Two-phase approach: parse local files, then finalize with empty pricing (no network)
  // This benchmarks pure file parsing + aggregation performance without network latency
  const localMessages = parseLocalSourcesNative({
    sources: ['opencode', 'claude', 'codex', 'gemini'], // No cursor - it's network-synced
  });
  
  const data = finalizeGraphNative({
    localMessages,
    pricing: [], // Empty pricing for benchmark - no network fetch
    includeCursor: false, // Skip cursor - no credentials in benchmark
  });
  
  // Count files and messages
  let totalMessages = 0;
  for (const contrib of data.contributions) {
    totalMessages += contrib.totals.messages;
  }
  
  return {
    data,
    files: 0,
    messages: totalMessages,
  };
}

async function runGraphGeneration(
  implementation: "typescript" | "rust",
  syntheticPath?: string
): Promise<{
  data: import("../src/graph-types.js").TokenContributionData;
  files: number;
  messages: number;
}> {
  if (implementation === "rust") {
    return runGraphGenerationRust(syntheticPath);
  }
  return runGraphGenerationTS();
}

// =============================================================================
// Memory Measurement
// =============================================================================

function getMemoryUsage(): { rss: number; heapUsed: number } {
  const mem = process.memoryUsage();
  return {
    rss: mem.rss / (1024 * 1024), // Convert to MB
    heapUsed: mem.heapUsed / (1024 * 1024),
  };
}

// =============================================================================
// Statistics
// =============================================================================

function calculateStats(values: number[]): {
  min: number;
  max: number;
  median: number;
  mean: number;
  stdDev: number;
} {
  const sorted = [...values].sort((a, b) => a - b);
  const n = sorted.length;
  
  const min = sorted[0];
  const max = sorted[n - 1];
  const median = n % 2 === 0
    ? (sorted[n / 2 - 1] + sorted[n / 2]) / 2
    : sorted[Math.floor(n / 2)];
  const mean = values.reduce((a, b) => a + b, 0) / n;
  
  const variance = values.reduce((sum, val) => sum + Math.pow(val - mean, 2), 0) / n;
  const stdDev = Math.sqrt(variance);
  
  return { min, max, median, mean, stdDev };
}

// =============================================================================
// Benchmark Execution
// =============================================================================

async function runBenchmark(config: BenchmarkConfig): Promise<BenchmarkResult> {
  console.log("\n📊 Token Tracker Benchmark Runner\n");
  console.log(`  Implementation: ${config.implementation}`);
  console.log(`  Data source: ${config.useSyntheticData ? "synthetic" : "real"}`);
  console.log(`  Iterations: ${config.iterations} (+ ${config.warmupIterations} warmup)`);
  console.log();
  
  // Setup synthetic data paths if needed
  if (config.useSyntheticData) {
    const absPath = path.resolve(config.syntheticDataPath);
    if (!fs.existsSync(absPath)) {
      console.error(`❌ Synthetic data not found at: ${absPath}`);
      console.error(`   Run: npx tsx benchmarks/generate.ts`);
      process.exit(1);
    }
    setupSyntheticDataPaths(config.syntheticDataPath);
    console.log(`  Synthetic data: ${absPath}`);
    console.log();
  }
  
  // Warmup iterations
  if (config.warmupIterations > 0) {
    console.log(`🔥 Warming up (${config.warmupIterations} iteration${config.warmupIterations > 1 ? "s" : ""})...`);
    for (let i = 0; i < config.warmupIterations; i++) {
      await runGraphGeneration(
        config.implementation,
        config.useSyntheticData ? config.syntheticDataPath : undefined
      );
    }
    console.log();
  }
  
  // Force garbage collection before benchmarking (if available)
  if (global.gc) {
    global.gc();
  }
  
  // Run benchmark iterations
  const iterations: IterationResult[] = [];
  let lastData: import("../src/graph-types.js").TokenContributionData | null = null;
  let totalMessages = 0;
  
  console.log(`⏱️  Running ${config.iterations} benchmark iteration${config.iterations > 1 ? "s" : ""}...`);
  
  for (let i = 0; i < config.iterations; i++) {
    // Force GC between iterations if available
    if (global.gc) {
      global.gc();
    }
    
    const memBefore = getMemoryUsage();
    const startTime = performance.now();
    
    const result = await runGraphGeneration(
      config.implementation,
      config.useSyntheticData ? config.syntheticDataPath : undefined
    );
    
    const endTime = performance.now();
    const memAfter = getMemoryUsage();
    
    const wallClockMs = endTime - startTime;
    const peakMemoryMb = Math.max(memBefore.rss, memAfter.rss);
    const heapUsedMb = memAfter.heapUsed;
    
    iterations.push({
      iteration: i + 1,
      wallClockMs,
      peakMemoryMb,
      heapUsedMb,
    });
    
    lastData = result.data;
    totalMessages = result.messages;
    
    console.log(`    Iteration ${i + 1}: ${wallClockMs.toFixed(2)}ms, ${peakMemoryMb.toFixed(1)}MB`);
  }
  
  console.log();
  
  // Calculate statistics
  const wallClockStats = calculateStats(iterations.map(i => i.wallClockMs));
  const memoryStats = calculateStats(iterations.map(i => i.peakMemoryMb));
  
  // Build result
  const result: BenchmarkResult = {
    implementation: config.implementation,
    command: "graph",
    dataSource: config.useSyntheticData ? "synthetic" : "real",
    timestamp: new Date().toISOString(),
    system: {
      platform: os.platform(),
      arch: os.arch(),
      cpus: os.cpus().length,
      totalMemoryGb: Math.round(os.totalmem() / (1024 * 1024 * 1024) * 10) / 10,
      nodeVersion: process.version,
    },
    config: {
      iterations: config.iterations,
      warmupIterations: config.warmupIterations,
    },
    iterations,
    summary: {
      wallClockMs: wallClockStats,
      peakMemoryMb: {
        min: memoryStats.min,
        max: memoryStats.max,
        median: memoryStats.median,
        mean: memoryStats.mean,
      },
    },
    data: {
      totalFiles: 0, // TODO: count files
      totalMessages,
      totalDays: lastData?.contributions.length ?? 0,
      sources: lastData?.summary.sources ?? [],
    },
  };
  
  return result;
}

// =============================================================================
// Output
// =============================================================================

function printSummary(result: BenchmarkResult): void {
  console.log("📈 Benchmark Results Summary\n");
  
  console.log("  ┌─────────────────────────────────────────────┐");
  console.log(`  │ Implementation: ${result.implementation.padEnd(27)} │`);
  console.log(`  │ Data Source: ${result.dataSource.padEnd(30)} │`);
  console.log("  ├─────────────────────────────────────────────┤");
  console.log(`  │ Wall Clock Time (median): ${result.summary.wallClockMs.median.toFixed(2).padStart(10)}ms │`);
  console.log(`  │ Wall Clock Time (mean):   ${result.summary.wallClockMs.mean.toFixed(2).padStart(10)}ms │`);
  console.log(`  │ Wall Clock Time (stddev): ${result.summary.wallClockMs.stdDev.toFixed(2).padStart(10)}ms │`);
  console.log("  ├─────────────────────────────────────────────┤");
  console.log(`  │ Peak Memory (median):     ${result.summary.peakMemoryMb.median.toFixed(1).padStart(10)}MB │`);
  console.log("  ├─────────────────────────────────────────────┤");
  console.log(`  │ Messages Processed:       ${String(result.data.totalMessages).padStart(14)} │`);
  console.log(`  │ Days with Data:           ${String(result.data.totalDays).padStart(14)} │`);
  console.log(`  │ Sources:                  ${result.data.sources.join(", ").padStart(14)} │`);
  console.log("  └─────────────────────────────────────────────┘");
  console.log();
}

function saveResults(result: BenchmarkResult, outputDir: string): string {
  if (!fs.existsSync(outputDir)) {
    fs.mkdirSync(outputDir, { recursive: true });
  }
  
  const filename = `benchmark-${result.implementation}-${result.dataSource}-${Date.now()}.json`;
  const filepath = path.join(outputDir, filename);
  
  fs.writeFileSync(filepath, JSON.stringify(result, null, 2));
  
  return filepath;
}

// =============================================================================
// Main
// =============================================================================

async function main() {
  const userConfig = parseArgs();
  const config: BenchmarkConfig = { ...DEFAULT_CONFIG, ...userConfig };
  
  try {
    const result = await runBenchmark(config);
    
    printSummary(result);
    
    const savedPath = saveResults(result, config.outputDir);
    console.log(`💾 Results saved to: ${savedPath}\n`);
    
  } catch (error) {
    console.error("❌ Benchmark failed:", error);
    process.exit(1);
  }
}

main();
