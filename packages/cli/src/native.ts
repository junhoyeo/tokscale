/**
 * Native module loader for Rust core
 *
 * Exposes all Rust functions with proper TypeScript types.
 * Native module is REQUIRED - no TypeScript fallback.
 */

import type {
  TokenContributionData,
  SourceType,
} from "./graph-types.js";
import { loadSettings } from "./tui/config/settings.js";

// =============================================================================
// Types matching Rust exports
// =============================================================================

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
  agent?: string;
}

interface NativeParsedMessages {
  messages: NativeParsedMessage[];
  opencodeCount: number;
  claudeCount: number;
  codexCount: number;
  geminiCount: number;
  ampCount: number;
  droidCount: number;
  openclawCount: number;
  piCount: number;
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
  includeCursor: boolean;
  since?: string;
  until?: string;
  year?: string;
}

interface NativeCore {
  version(): string;
  healthCheck(): string;
  parseLocalSources(options: NativeLocalParseOptions): NativeParsedMessages;
  finalizeReport(options: NativeFinalizeReportOptions): NativeModelReport;
  finalizeMonthlyReport(options: NativeFinalizeReportOptions): NativeMonthlyReport;
  finalizeGraph(options: NativeFinalizeReportOptions): NativeGraphResult;
}

// =============================================================================
// Module loading
// =============================================================================

let nativeCore: NativeCore | null = null;

try {
  // Type assertion needed because dynamic import returns module namespace
  // nativeCore.version() is called directly, async functions go through subprocess
  nativeCore = await import("@tokscale/core").then(
    (m) => (m.default || m) as unknown as NativeCore
  );
} catch (e) {
  void e;
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
 * Get native module version
 */
export function getNativeVersion(): string | null {
  return nativeCore?.version() ?? null;
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
    agent?: string;
  }>;
  opencodeCount: number;
  claudeCount: number;
  codexCount: number;
  geminiCount: number;
  ampCount: number;
  droidCount: number;
  openclawCount: number;
  piCount: number;
  processingTimeMs: number;
}

export interface LocalParseOptions {
  homeDir?: string;
  sources?: SourceType[];
  since?: string;
  until?: string;
  year?: string;
}

export interface FinalizeOptions {
  localMessages: ParsedMessages;
  includeCursor: boolean;
  since?: string;
  until?: string;
  year?: string;
}



// =============================================================================
// Async Subprocess Wrappers (Non-blocking for UI)
// =============================================================================

import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { writeFileSync, readFileSync, unlinkSync, mkdirSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { randomUUID } from "node:crypto";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const SIGKILL_GRACE_MS = 500;

function getNativeTimeoutMs(): number {
  const settings = loadSettings();
  return process.env.TOKSCALE_NATIVE_TIMEOUT_MS
    ? parseInt(process.env.TOKSCALE_NATIVE_TIMEOUT_MS, 10)
    : (settings.nativeTimeoutMs ?? 300_000);
}

interface BunSubprocess {
  stdout: { text: () => Promise<string> };
  stderr: { text: () => Promise<string> };
  exited: Promise<number>;
  signalCode: string | null;
  killed: boolean;
  kill: (signal?: string) => void;
}

interface BunSpawnOptions {
  stdout: "pipe" | "ignore";
  stderr: "pipe" | "ignore";
}

interface BunGlobalType {
  spawn: (cmd: string[], opts: BunSpawnOptions) => BunSubprocess;
}

function safeKill(proc: unknown, signal?: string): void {
  try {
    (proc as { kill: (signal?: string) => void }).kill(signal);
  } catch {}
}

async function runInSubprocess<T>(method: string, args: unknown[]): Promise<T> {
  const NATIVE_TIMEOUT_MS = getNativeTimeoutMs();
  const runnerPath = join(__dirname, "native-runner.js");
  const input = JSON.stringify({ method, args });

  const tmpDir = join(tmpdir(), "tokscale");
  mkdirSync(tmpDir, { recursive: true });
  const id = randomUUID();
  const inputFile = join(tmpDir, `input-${id}.json`);
  const outputFile = join(tmpDir, `output-${id}.json`);

  writeFileSync(inputFile, input, "utf-8");

  const BunGlobal = (globalThis as Record<string, unknown>).Bun as BunGlobalType;

  let proc: BunSubprocess;
  try {
    proc = BunGlobal.spawn([process.execPath, runnerPath, inputFile, outputFile], {
      stdout: "ignore",
      stderr: "pipe",
    });
  } catch (e) {
    try { unlinkSync(inputFile); } catch {}
    throw new Error(`Failed to spawn subprocess: ${(e as Error).message}`);
  }

  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  let sigkillId: ReturnType<typeof setTimeout> | null = null;
  let weInitiatedKill = false;

  const cleanup = async () => {
    if (timeoutId) clearTimeout(timeoutId);
    if (sigkillId) clearTimeout(sigkillId);
    try { unlinkSync(inputFile); } catch {}
    try { unlinkSync(outputFile); } catch {}
  };

  try {
    const timeoutPromise = new Promise<never>((_, reject) => {
      timeoutId = setTimeout(() => {
        weInitiatedKill = true;
        safeKill(proc, "SIGTERM");
        sigkillId = setTimeout(() => {
          safeKill(proc, "SIGKILL");
          reject(new Error(
            `Subprocess '${method}' timed out after ${NATIVE_TIMEOUT_MS}ms (hard kill)`
          ));
        }, SIGKILL_GRACE_MS);
      }, NATIVE_TIMEOUT_MS);
    });

    const exitCode = await Promise.race([proc.exited, timeoutPromise]);

    if (timeoutId) clearTimeout(timeoutId);

    if (weInitiatedKill || proc.signalCode) {
      throw new Error(
        `Subprocess '${method}' was killed (signal: ${proc.signalCode || "SIGTERM"})`
      );
    }

    if (exitCode !== 0) {
      const stderr = await proc.stderr.text();
      let errorMsg = stderr || `Process exited with code ${exitCode}`;
      try {
        const parsed = JSON.parse(stderr);
        if (parsed.error) errorMsg = parsed.error;
      } catch {}
      throw new Error(`Subprocess '${method}' failed: ${errorMsg}`);
    }

    if (!existsSync(outputFile)) {
      throw new Error(`Subprocess '${method}' did not produce output file`);
    }

    try {
      const output = readFileSync(outputFile, "utf-8");
      return JSON.parse(output) as T;
    } catch (e) {
      throw new Error(
        `Failed to parse subprocess output: ${(e as Error).message}`
      );
    }
  } finally {
    await cleanup();
  }
}

export async function parseLocalSourcesAsync(options: LocalParseOptions): Promise<ParsedMessages> {
  if (!isNativeAvailable()) {
    throw new Error("Native module required. Run: bun run build:core");
  }

  const nativeOptions: NativeLocalParseOptions = {
    homeDir: options.homeDir,
    sources: options.sources,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return runInSubprocess<ParsedMessages>("parseLocalSources", [nativeOptions]);
}

export async function finalizeReportAsync(options: FinalizeOptions): Promise<ModelReport> {
  if (!isNativeAvailable()) {
    throw new Error("Native module required. Run: bun run build:core");
  }

  const nativeOptions: NativeFinalizeReportOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    includeCursor: options.includeCursor,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return runInSubprocess<ModelReport>("finalizeReport", [nativeOptions]);
}

export async function finalizeMonthlyReportAsync(options: FinalizeOptions): Promise<MonthlyReport> {
  if (!isNativeAvailable()) {
    throw new Error("Native module required. Run: bun run build:core");
  }

  const nativeOptions: NativeFinalizeReportOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    includeCursor: options.includeCursor,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return runInSubprocess<MonthlyReport>("finalizeMonthlyReport", [nativeOptions]);
}

export async function finalizeGraphAsync(options: FinalizeOptions): Promise<TokenContributionData> {
  if (!isNativeAvailable()) {
    throw new Error("Native module required. Run: bun run build:core");
  }

  const nativeOptions: NativeFinalizeReportOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    includeCursor: options.includeCursor,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  const result = await runInSubprocess<NativeGraphResult>("finalizeGraph", [nativeOptions]);
  return fromNativeResult(result);
}

export interface ReportAndGraph {
  report: ModelReport;
  graph: TokenContributionData;
}

interface NativeReportAndGraph {
  report: NativeModelReport;
  graph: NativeGraphResult;
}

export async function finalizeReportAndGraphAsync(options: FinalizeOptions): Promise<ReportAndGraph> {
  if (!isNativeAvailable()) {
    throw new Error("Native module required. Run: bun run build:core");
  }

  const nativeOptions: NativeFinalizeReportOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    includeCursor: options.includeCursor,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  const result = await runInSubprocess<NativeReportAndGraph>("finalizeReportAndGraph", [nativeOptions]);
  return {
    report: result.report,
    graph: fromNativeResult(result.graph),
  };
}
