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
  nativeCore = await import("@0xinevitable/token-tracker-core").then((m) => m.default || m);
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

/**
 * Generate graph data with pricing calculation
 */
export function generateGraphWithPricing(
  options: TSGraphOptions & { pricing: PricingEntry[] }
): TokenContributionData {
  if (!nativeCore) {
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

  const result = nativeCore.generateGraphWithPricing(nativeOptions);
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

export interface ReportOptions {
  sources?: SourceType[];
  pricing: PricingEntry[];
  since?: string;
  until?: string;
  year?: string;
}

/**
 * Get model usage report using native module
 */
export function getModelReportNative(options: ReportOptions): ModelReport {
  if (!nativeCore) {
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

  return nativeCore.getModelReport(nativeOptions);
}

/**
 * Get monthly usage report using native module
 */
export function getMonthlyReportNative(options: ReportOptions): MonthlyReport {
  if (!nativeCore) {
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

  return nativeCore.getMonthlyReport(nativeOptions);
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

/**
 * Parse local sources only (OpenCode, Claude, Codex, Gemini - NO Cursor)
 * This can run in parallel with network operations (Cursor sync, pricing fetch)
 */
export function parseLocalSourcesNative(options: LocalParseOptions): ParsedMessages {
  if (!nativeCore) {
    throw new Error("Native module not available: " + (loadError?.message || "unknown error"));
  }

  const nativeOptions: NativeLocalParseOptions = {
    homeDir: undefined,
    sources: options.sources,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return nativeCore.parseLocalSources(nativeOptions);
}

/**
 * Finalize model report: apply pricing to local messages, add Cursor, aggregate
 */
export function finalizeReportNative(options: FinalizeOptions): ModelReport {
  if (!nativeCore) {
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

  return nativeCore.finalizeReport(nativeOptions);
}

/**
 * Finalize monthly report: apply pricing to local messages, add Cursor, aggregate
 */
export function finalizeMonthlyReportNative(options: FinalizeOptions): MonthlyReport {
  if (!nativeCore) {
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

  return nativeCore.finalizeMonthlyReport(nativeOptions);
}

export function finalizeGraphNative(options: FinalizeOptions): TokenContributionData {
  if (!nativeCore) {
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

  const result = nativeCore.finalizeGraph(nativeOptions);
  return fromNativeResult(result);
}

import { spawn } from "child_process";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

function runInSubprocess<T>(method: string, args: unknown[]): Promise<T> {
  return new Promise((resolve, reject) => {
    const runnerPath = join(__dirname, "native-runner.ts");
    
    const proc = spawn("bun", ["run", runnerPath], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    
    const stdoutChunks: Buffer[] = [];
    const stderrChunks: Buffer[] = [];
    
    proc.stdout.on("data", (data: Buffer) => { stdoutChunks.push(data); });
    proc.stderr.on("data", (data: Buffer) => { stderrChunks.push(data); });
    
    proc.on("close", (code) => {
      const stdout = Buffer.concat(stdoutChunks).toString("utf-8");
      const stderr = Buffer.concat(stderrChunks).toString("utf-8");
      
      if (code !== 0) {
        reject(new Error(`Subprocess failed: ${stderr || `exit code ${code}`}`));
        return;
      }
      try {
        resolve(JSON.parse(stdout) as T);
      } catch (e) {
        reject(new Error(`Failed to parse output: ${(e as Error).message}\nstdout: ${stdout.slice(0, 500)}`));
      }
    });
    
    proc.on("error", reject);
    
    proc.stdin.write(JSON.stringify({ method, args }));
    proc.stdin.end();
  });
}

export async function parseLocalSourcesNativeAsync(options: LocalParseOptions): Promise<ParsedMessages> {
  const nativeOptions: NativeLocalParseOptions = {
    homeDir: undefined,
    sources: options.sources,
    since: options.since,
    until: options.until,
    year: options.year,
  };
  return runInSubprocess<ParsedMessages>("parseLocalSources", [nativeOptions]);
}

export async function finalizeReportNativeAsync(options: FinalizeOptions): Promise<ModelReport> {
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

export async function finalizeMonthlyReportNativeAsync(options: FinalizeOptions): Promise<MonthlyReport> {
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

export async function finalizeGraphNativeAsync(options: FinalizeOptions): Promise<TokenContributionData> {
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
