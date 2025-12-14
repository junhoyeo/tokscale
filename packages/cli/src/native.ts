/**
 * Native module loader for Rust core
 * 
 * Exposes all Rust functions with proper TypeScript types.
 */

import type { PricingEntry } from "./pricing.js";
import type {
  TokenContributionData,
  GraphOptions as TSGraphOptions,
  SourceType,
} from "./graph-types.js";

// =============================================================================
// Types matching Rust exports
// =============================================================================

interface NativeGraphOptions {
  homeDir?: string;
  sources?: string[];
  since?: string;
  until?: string;
  year?: string;
  threads?: number;
}

interface NativeScanStats {
  opencodeFiles: number;
  claudeFiles: number;
  codexFiles: number;
  geminiFiles: number;
  totalFiles: number;
}

interface NativeTokenBreakdown {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
}

interface NativeDailyTotals {
  tokens: number;
  cost: number;
  messages: number;
}

interface NativeSourceContribution {
  source: string;
  modelId: string;
  providerId: string;
  tokens: NativeTokenBreakdown;
  cost: number;
  messages: number;
}

interface NativeDailyContribution {
  date: string;
  totals: NativeDailyTotals;
  intensity: number;
  tokenBreakdown: NativeTokenBreakdown;
  sources: NativeSourceContribution[];
}

interface NativeYearSummary {
  year: string;
  totalTokens: number;
  totalCost: number;
  rangeStart: string;
  rangeEnd: string;
}

interface NativeDataSummary {
  totalTokens: number;
  totalCost: number;
  totalDays: number;
  activeDays: number;
  averagePerDay: number;
  maxCostInSingleDay: number;
  sources: string[];
  models: string[];
}

interface NativeGraphMeta {
  generatedAt: string;
  version: string;
  dateRangeStart: string;
  dateRangeEnd: string;
  processingTimeMs: number;
}

interface NativeGraphResult {
  meta: NativeGraphMeta;
  summary: NativeDataSummary;
  years: NativeYearSummary[];
  contributions: NativeDailyContribution[];
}

// Types for pricing-aware APIs
interface NativePricingEntry {
  modelId: string;
  pricing: {
    inputCostPerToken: number;
    outputCostPerToken: number;
    cacheReadInputTokenCost?: number;
    cacheCreationInputTokenCost?: number;
  };
}

interface NativeReportOptions {
  homeDir?: string;
  sources?: string[];
  pricing: NativePricingEntry[];
  since?: string;
  until?: string;
  year?: string;
}

interface NativeModelUsage {
  source: string;
  model: string;
  provider: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messageCount: number;
  cost: number;
}

interface NativeModelReport {
  entries: NativeModelUsage[];
  totalInput: number;
  totalOutput: number;
  totalCacheRead: number;
  totalCacheWrite: number;
  totalMessages: number;
  totalCost: number;
  processingTimeMs: number;
}

interface NativeMonthlyUsage {
  month: string;
  models: string[];
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  messageCount: number;
  cost: number;
}

interface NativeMonthlyReport {
  entries: NativeMonthlyUsage[];
  totalCost: number;
  processingTimeMs: number;
}

// Types for two-phase processing (parallel optimization)
interface NativeParsedMessage {
  source: string;
  modelId: string;
  providerId: string;
  timestamp: number;
  date: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  sessionId: string;
}

interface NativeParsedMessages {
  messages: NativeParsedMessage[];
  opencodeCount: number;
  claudeCount: number;
  codexCount: number;
  geminiCount: number;
  processingTimeMs: number;
}

interface NativeLocalParseOptions {
  homeDir?: string;
  sources?: string[];
  since?: string;
  until?: string;
  year?: string;
}

interface NativeFinalizeReportOptions {
  homeDir?: string;
  localMessages: NativeParsedMessages;
  pricing: NativePricingEntry[];
  includeCursor: boolean;
  since?: string;
  until?: string;
  year?: string;
}

interface NativeCore {
  version(): string;
  healthCheck(): string;
  generateGraph(options: NativeGraphOptions): NativeGraphResult;
  generateGraphWithPricing(options: NativeReportOptions): NativeGraphResult;
  scanSessions(homeDir?: string, sources?: string[]): NativeScanStats;
  getModelReport(options: NativeReportOptions): NativeModelReport;
  getMonthlyReport(options: NativeReportOptions): NativeMonthlyReport;
  // Two-phase processing (parallel optimization)
  parseLocalSources(options: NativeLocalParseOptions): NativeParsedMessages;
  finalizeReport(options: NativeFinalizeReportOptions): NativeModelReport;
  finalizeMonthlyReport(options: NativeFinalizeReportOptions): NativeMonthlyReport;
  finalizeGraph(options: NativeFinalizeReportOptions): NativeGraphResult;
}

// =============================================================================
// Module loading
// =============================================================================

let nativeCore: NativeCore | null = null;
let loadError: Error | null = null;

try {
  nativeCore = await import("tokscale-core").then((m) => m.default || m);
} catch (e) {
  loadError = e as Error;
}

// =============================================================================
// Public API
// =============================================================================

/**
 * Check if native module is available
 */
export function isNativeAvailable(): boolean {
  return nativeCore !== null;
}

/**
 * Get native module load error (if any)
 */
export function getNativeLoadError(): Error | null {
  return loadError;
}

/**
 * Get native module version
 */
export function getNativeVersion(): string | null {
  return nativeCore?.version() ?? null;
}

/**
 * Scan sessions using native module
 */
export function scanSessionsNative(homeDir?: string, sources?: string[]): NativeScanStats | null {
  if (!nativeCore) {
    return null;
  }
  return nativeCore.scanSessions(homeDir, sources);
}

// =============================================================================
// Graph generation
// =============================================================================

/**
 * Convert TypeScript graph options to native format
 */
function toNativeOptions(options: TSGraphOptions): NativeGraphOptions {
  return {
    homeDir: undefined,
    sources: options.sources,
    since: options.since,
    until: options.until,
    year: options.year,
  };
}

/**
 * Convert native result to TypeScript format
 */
function fromNativeResult(result: NativeGraphResult): TokenContributionData {
  return {
    meta: {
      generatedAt: result.meta.generatedAt,
      version: result.meta.version,
      dateRange: {
        start: result.meta.dateRangeStart,
        end: result.meta.dateRangeEnd,
      },
    },
    summary: {
      totalTokens: result.summary.totalTokens,
      totalCost: result.summary.totalCost,
      totalDays: result.summary.totalDays,
      activeDays: result.summary.activeDays,
      averagePerDay: result.summary.averagePerDay,
      maxCostInSingleDay: result.summary.maxCostInSingleDay,
      sources: result.summary.sources as SourceType[],
      models: result.summary.models,
    },
    years: result.years.map((y) => ({
      year: y.year,
      totalTokens: y.totalTokens,
      totalCost: y.totalCost,
      range: {
        start: y.rangeStart,
        end: y.rangeEnd,
      },
    })),
    contributions: result.contributions.map((c) => ({
      date: c.date,
      totals: {
        tokens: c.totals.tokens,
        cost: c.totals.cost,
        messages: c.totals.messages,
      },
      intensity: c.intensity as 0 | 1 | 2 | 3 | 4,
      tokenBreakdown: {
        input: c.tokenBreakdown.input,
        output: c.tokenBreakdown.output,
        cacheRead: c.tokenBreakdown.cacheRead,
        cacheWrite: c.tokenBreakdown.cacheWrite,
        reasoning: c.tokenBreakdown.reasoning,
      },
      sources: c.sources.map((s) => ({
        source: s.source as SourceType,
        modelId: s.modelId,
        providerId: s.providerId,
        tokens: {
          input: s.tokens.input,
          output: s.tokens.output,
          cacheRead: s.tokens.cacheRead,
          cacheWrite: s.tokens.cacheWrite,
          reasoning: s.tokens.reasoning,
        },
        cost: s.cost,
        messages: s.messages,
      })),
    })),
  };
}

/**
 * Generate graph data using native module (without pricing - uses embedded costs)
 * @deprecated Use generateGraphWithPricing instead
 */
export function generateGraphNative(options: TSGraphOptions = {}): TokenContributionData {
  if (!nativeCore) {
    throw new Error("Native module not available: " + (loadError?.message || "unknown error"));
  }

  const nativeOptions = toNativeOptions(options);
  const result = nativeCore.generateGraph(nativeOptions);
  return fromNativeResult(result);
}



// =============================================================================
// Reports
// =============================================================================

export interface ModelUsage {
  source: string;
  model: string;
  provider: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messageCount: number;
  cost: number;
}

export interface ModelReport {
  entries: ModelUsage[];
  totalInput: number;
  totalOutput: number;
  totalCacheRead: number;
  totalCacheWrite: number;
  totalMessages: number;
  totalCost: number;
  processingTimeMs: number;
}

export interface MonthlyUsage {
  month: string;
  models: string[];
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  messageCount: number;
  cost: number;
}

export interface MonthlyReport {
  entries: MonthlyUsage[];
  totalCost: number;
  processingTimeMs: number;
}

// =============================================================================
// Two-Phase Processing (Parallel Optimization)
// =============================================================================

export interface ParsedMessages {
  messages: Array<{
    source: string;
    modelId: string;
    providerId: string;
    timestamp: number;
    date: string;
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    reasoning: number;
    sessionId: string;
  }>;
  opencodeCount: number;
  claudeCount: number;
  codexCount: number;
  geminiCount: number;
  processingTimeMs: number;
}

export interface LocalParseOptions {
  sources?: SourceType[];
  since?: string;
  until?: string;
  year?: string;
}

export interface FinalizeOptions {
  localMessages: ParsedMessages;
  pricing: PricingEntry[];
  includeCursor: boolean;
  since?: string;
  until?: string;
  year?: string;
}



// =============================================================================
// Async Subprocess Wrappers (Non-blocking for UI)
// =============================================================================

import { spawn } from "bun";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const DEFAULT_TIMEOUT_MS = 120_000; // 2 minutes default
const NATIVE_TIMEOUT_MS = parseInt(
  process.env.TOKSCALE_NATIVE_TIMEOUT_MS || String(DEFAULT_TIMEOUT_MS),
  10
);

// Timeout-related exit codes
const TIMEOUT_EXIT_CODES = new Set([
  124,  // GNU timeout default
  143,  // 128 + 15 (SIGTERM)
  137,  // 128 + 9 (SIGKILL - when SIGTERM fails)
]);

async function runInSubprocess<T>(method: string, args: unknown[]): Promise<T> {
  const runnerPath = join(__dirname, "native-runner.ts");
  const input = JSON.stringify({ method, args });

  const proc = spawn({
    cmd: ["bun", "run", runnerPath],
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
    timeout: NATIVE_TIMEOUT_MS,
    killSignal: "SIGTERM",
  });

  // Helper to consume streams (prevents resource leaks)
  const consumeStreams = async (): Promise<{ stdout: string; stderr: string }> => {
    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    return { stdout, stderr };
  };

  try {
    proc.stdin.write(input);
    proc.stdin.end();
  } catch (e) {
    proc.kill("SIGTERM");
    // Must consume streams even after kill to prevent resource leaks
    await consumeStreams().catch(() => {}); // Ignore stream errors after kill
    await proc.exited.catch(() => {}); // Wait for process to exit
    throw new Error(`Failed to write to subprocess stdin: ${(e as Error).message}`);
  }

  const [{ stdout, stderr }, exitCode] = await Promise.all([
    consumeStreams(),
    proc.exited,
  ]);

  // Check for timeout exit codes
  if (TIMEOUT_EXIT_CODES.has(exitCode)) {
    throw new Error(
      `Subprocess '${method}' timed out after ${NATIVE_TIMEOUT_MS}ms (exit code ${exitCode})`
    );
  }

  if (exitCode !== 0) {
    let errorMsg = stderr || `Process exited with code ${exitCode}`;
    try {
      const parsed = JSON.parse(stderr);
      if (parsed.error) errorMsg = parsed.error;
    } catch {}
    throw new Error(`Subprocess '${method}' failed: ${errorMsg}`);
  }

  try {
    return JSON.parse(stdout) as T;
  } catch (e) {
    throw new Error(
      `Failed to parse subprocess output: ${(e as Error).message}\nstdout: ${stdout.slice(0, 500)}`
    );
  }
}

export async function parseLocalSourcesAsync(options: LocalParseOptions): Promise<ParsedMessages> {
  if (!isNativeAvailable()) {
    throw new Error("Native module not available: " + (loadError?.message || "unknown error"));
  }

  const nativeOptions: NativeLocalParseOptions = {
    homeDir: undefined,
    sources: options.sources,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return runInSubprocess<ParsedMessages>("parseLocalSources", [nativeOptions]);
}

export async function finalizeReportAsync(options: FinalizeOptions): Promise<ModelReport> {
  if (!isNativeAvailable()) {
    throw new Error("Native module not available: " + (loadError?.message || "unknown error"));
  }

  const nativeOptions: NativeFinalizeReportOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    pricing: options.pricing,
    includeCursor: options.includeCursor,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return runInSubprocess<ModelReport>("finalizeReport", [nativeOptions]);
}

export async function finalizeMonthlyReportAsync(options: FinalizeOptions): Promise<MonthlyReport> {
  if (!isNativeAvailable()) {
    throw new Error("Native module not available: " + (loadError?.message || "unknown error"));
  }

  const nativeOptions: NativeFinalizeReportOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    pricing: options.pricing,
    includeCursor: options.includeCursor,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return runInSubprocess<MonthlyReport>("finalizeMonthlyReport", [nativeOptions]);
}

export async function finalizeGraphAsync(options: FinalizeOptions): Promise<TokenContributionData> {
  if (!isNativeAvailable()) {
    throw new Error("Native module not available: " + (loadError?.message || "unknown error"));
  }

  const nativeOptions: NativeFinalizeReportOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    pricing: options.pricing,
    includeCursor: options.includeCursor,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  const result = await runInSubprocess<NativeGraphResult>("finalizeGraph", [nativeOptions]);
  return fromNativeResult(result);
}

export async function generateGraphWithPricingAsync(
  options: TSGraphOptions & { pricing: PricingEntry[] }
): Promise<TokenContributionData> {
  if (!isNativeAvailable()) {
    throw new Error("Native module not available: " + (loadError?.message || "unknown error"));
  }

  const nativeOptions: NativeReportOptions = {
    homeDir: undefined,
    sources: options.sources,
    pricing: options.pricing,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  const result = await runInSubprocess<NativeGraphResult>("generateGraphWithPricing", [nativeOptions]);
  return fromNativeResult(result);
}
