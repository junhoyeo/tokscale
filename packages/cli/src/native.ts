/**
 * Native module loader for Rust core
 *
 * Exposes all Rust functions with proper TypeScript types.
 * Falls back to TypeScript implementations when native module is unavailable.
 */

import type { PricingEntry } from "./pricing.js";
import type {
  TokenContributionData,
  GraphOptions as TSGraphOptions,
  SourceType,
} from "./graph-types.js";
import {
  parseLocalSources as parseLocalSourcesTS,
  type ParsedMessages as TSParsedMessages,
  type UnifiedMessage,
} from "./sessions/index.js";
import {
  generateModelReport as generateModelReportTS,
  generateMonthlyReport as generateMonthlyReportTS,
  generateGraphData as generateGraphDataTS,
} from "./sessions/reports.js";
import { readCursorMessagesFromCache } from "./cursor.js";

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
  agent?: string;
}

interface NativeAgentUsage {
  agent: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messageCount: number;
  cost: number;
}

interface NativeAgentReport {
  entries: NativeAgentUsage[];
  totalInput: number;
  totalOutput: number;
  totalCacheRead: number;
  totalCacheWrite: number;
  totalMessages: number;
  totalCost: number;
  processingTimeMs: number;
}

interface NativeFinalizeAgentOptions {
  homeDir?: string;
  localMessages: NativeParsedMessages;
  pricing: NativePricingEntry[];
  since?: string;
  until?: string;
  year?: string;
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
  getAgentReport(options: NativeReportOptions): NativeAgentReport;
  parseLocalSources(options: NativeLocalParseOptions): NativeParsedMessages;
  finalizeReport(options: NativeFinalizeReportOptions): NativeModelReport;
  finalizeMonthlyReport(options: NativeFinalizeReportOptions): NativeMonthlyReport;
  finalizeGraph(options: NativeFinalizeReportOptions): NativeGraphResult;
  finalizeAgentReport(options: NativeFinalizeAgentOptions): NativeAgentReport;
}

// =============================================================================
// Module loading
// =============================================================================

let nativeCore: NativeCore | null = null;
let loadError: Error | null = null;

try {
  nativeCore = await import("@tokscale/core").then((m) => m.default || m);
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

export interface AgentUsage {
  agent: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messageCount: number;
  cost: number;
}

export interface AgentReport {
  entries: AgentUsage[];
  totalInput: number;
  totalOutput: number;
  totalCacheRead: number;
  totalCacheWrite: number;
  totalMessages: number;
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

import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const DEFAULT_TIMEOUT_MS = 300_000;
const NATIVE_TIMEOUT_MS = parseInt(
  process.env.TOKSCALE_NATIVE_TIMEOUT_MS || String(DEFAULT_TIMEOUT_MS),
  10
);

const SIGKILL_GRACE_MS = 500;
const DEFAULT_MAX_OUTPUT_BYTES = 100 * 1024 * 1024;
const MAX_OUTPUT_BYTES = parseInt(
  process.env.TOKSCALE_MAX_OUTPUT_BYTES || String(DEFAULT_MAX_OUTPUT_BYTES),
  10
);

interface BunSubprocess {
  stdin: { write: (data: string) => void; end: () => void };
  stdout: { text: () => Promise<string> };
  stderr: { text: () => Promise<string> };
  exited: Promise<number>;
  signalCode: string | null;
  killed: boolean;
  kill: (signal?: string) => void;
}

interface BunSpawnOptions {
  stdin: string;
  stdout: string;
  stderr: string;
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
  const runnerPath = join(__dirname, "native-runner.js");
  const input = JSON.stringify({ method, args });

  const BunGlobal = (globalThis as Record<string, unknown>).Bun as BunGlobalType;

  let proc: BunSubprocess;
  try {
    proc = BunGlobal.spawn([process.execPath, runnerPath], {
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
    });
  } catch (e) {
    throw new Error(`Failed to spawn subprocess: ${(e as Error).message}`);
  }

  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  let sigkillId: ReturnType<typeof setTimeout> | null = null;
  let weInitiatedKill = false;
  let aborted = false;

  const cleanup = async () => {
    if (timeoutId) clearTimeout(timeoutId);
    if (sigkillId) clearTimeout(sigkillId);
    if (aborted) {
      safeKill(proc, "SIGKILL");
      await proc.exited.catch(() => {});
    }
  };

  const abort = () => {
    aborted = true;
    weInitiatedKill = true;
  };

  try {
    proc.stdin.write(input);
    proc.stdin.end();

    const stdoutChunks: Uint8Array[] = [];
    const stderrChunks: Uint8Array[] = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;

    const readStream = async (
      stream: BunSubprocess["stdout"],
      chunks: Uint8Array[],
      getBytesRef: () => number,
      setBytesRef: (n: number) => void
    ): Promise<string> => {
      const reader = (stream as unknown as ReadableStream<Uint8Array>).getReader();
      try {
        while (!aborted) {
          const { done, value } = await reader.read();
          if (done) break;
          const newTotal = getBytesRef() + value.length;
          if (newTotal > MAX_OUTPUT_BYTES) {
            abort();
            throw new Error(`Output exceeded ${MAX_OUTPUT_BYTES} bytes`);
          }
          setBytesRef(newTotal);
          chunks.push(value);
        }
      } finally {
        await reader.cancel().catch(() => {});
        reader.releaseLock();
      }
      const combined = new Uint8Array(getBytesRef());
      let offset = 0;
      for (const chunk of chunks) {
        combined.set(chunk, offset);
        offset += chunk.length;
      }
      return new TextDecoder().decode(combined);
    };

    const timeoutPromise = new Promise<never>((_, reject) => {
      timeoutId = setTimeout(() => {
        abort();
        safeKill(proc, "SIGTERM");
        sigkillId = setTimeout(() => {
          safeKill(proc, "SIGKILL");
          reject(new Error(
            `Subprocess '${method}' timed out after ${NATIVE_TIMEOUT_MS}ms (hard kill)`
          ));
        }, SIGKILL_GRACE_MS);
      }, NATIVE_TIMEOUT_MS);
    });

    const workPromise = Promise.all([
      readStream(proc.stdout, stdoutChunks, () => stdoutBytes, (n) => { stdoutBytes = n; }),
      readStream(proc.stderr, stderrChunks, () => stderrBytes, (n) => { stderrBytes = n; }),
      proc.exited,
    ]);

    const [stdout, stderr, exitCode] = await Promise.race([workPromise, timeoutPromise]);

    // Note: proc.killed is always true after exit in Bun (even for normal exits), so we only check signalCode
    if (weInitiatedKill || proc.signalCode) {
      throw new Error(
        `Subprocess '${method}' was killed (signal: ${proc.signalCode || "SIGTERM"})`
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
  } finally {
    await cleanup();
  }
}

export async function parseLocalSourcesAsync(options: LocalParseOptions): Promise<ParsedMessages> {
  // Use TypeScript fallback when native module is not available
  if (!isNativeAvailable()) {
    const result = parseLocalSourcesTS({
      sources: options.sources,
      since: options.since,
      until: options.until,
      year: options.year,
    });

    // Convert TypeScript ParsedMessages to native format
    return {
      messages: result.messages.map((msg) => ({
        source: msg.source,
        modelId: msg.modelId,
        providerId: msg.providerId,
        timestamp: msg.timestamp,
        date: msg.date,
        input: msg.tokens.input,
        output: msg.tokens.output,
        cacheRead: msg.tokens.cacheRead,
        cacheWrite: msg.tokens.cacheWrite,
        reasoning: msg.tokens.reasoning,
        sessionId: msg.sessionId,
        agent: msg.agent,
      })),
      opencodeCount: result.opencodeCount,
      claudeCount: result.claudeCount,
      codexCount: result.codexCount,
      geminiCount: result.geminiCount,
      processingTimeMs: result.processingTimeMs,
    };
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

function buildMessagesForFallback(options: FinalizeOptions): UnifiedMessage[] {
  const messages: UnifiedMessage[] = options.localMessages.messages.map((msg) => ({
    source: msg.source,
    modelId: msg.modelId,
    providerId: msg.providerId,
    sessionId: msg.sessionId,
    timestamp: msg.timestamp,
    date: msg.date,
    tokens: {
      input: msg.input,
      output: msg.output,
      cacheRead: msg.cacheRead,
      cacheWrite: msg.cacheWrite,
      reasoning: msg.reasoning,
    },
    cost: 0,
  }));

  if (options.includeCursor) {
    const cursorMessages = readCursorMessagesFromCache();
    for (const cursor of cursorMessages) {
      const inRange =
        (!options.year || cursor.date.startsWith(options.year)) &&
        (!options.since || cursor.date >= options.since) &&
        (!options.until || cursor.date <= options.until);
      if (inRange) {
        messages.push(cursor);
      }
    }
  }

  return messages;
}

export async function finalizeReportAsync(options: FinalizeOptions): Promise<ModelReport> {
  if (!isNativeAvailable()) {
    const startTime = performance.now();
    const messages = buildMessagesForFallback(options);
    return generateModelReportTS(messages, options.pricing, startTime);
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
    const startTime = performance.now();
    const messages = buildMessagesForFallback(options);
    return generateMonthlyReportTS(messages, options.pricing, startTime);
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
    const startTime = performance.now();
    const messages = buildMessagesForFallback(options);
    return generateGraphDataTS(messages, options.pricing, startTime);
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
  // Use TypeScript fallback when native module is not available
  if (!isNativeAvailable()) {
    const startTime = performance.now();

    // Parse local sources using TS fallback
    const parsed = parseLocalSourcesTS({
      sources: options.sources,
      since: options.since,
      until: options.until,
      year: options.year,
    });

    return generateGraphDataTS(parsed.messages, options.pricing, startTime);
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

export interface FinalizeAgentOptions {
  localMessages: ParsedMessages;
  pricing: PricingEntry[];
  since?: string;
  until?: string;
  year?: string;
}

export async function finalizeAgentReportAsync(options: FinalizeAgentOptions): Promise<AgentReport> {
  if (!isNativeAvailable()) {
    const startTime = performance.now();
    const messages = options.localMessages.messages;
    
    const agentMap = new Map<string, AgentUsage>();
    
    for (const msg of messages) {
      if (!msg.agent) continue;
      
      const existing = agentMap.get(msg.agent);
      if (existing) {
        existing.input += msg.input;
        existing.output += msg.output;
        existing.cacheRead += msg.cacheRead;
        existing.cacheWrite += msg.cacheWrite;
        existing.reasoning += msg.reasoning;
        existing.messageCount += 1;
      } else {
        agentMap.set(msg.agent, {
          agent: msg.agent,
          input: msg.input,
          output: msg.output,
          cacheRead: msg.cacheRead,
          cacheWrite: msg.cacheWrite,
          reasoning: msg.reasoning,
          messageCount: 1,
          cost: 0,
        });
      }
    }
    
    const entries = Array.from(agentMap.values()).sort((a, b) => b.messageCount - a.messageCount);
    
    return {
      entries,
      totalInput: entries.reduce((sum, e) => sum + e.input, 0),
      totalOutput: entries.reduce((sum, e) => sum + e.output, 0),
      totalCacheRead: entries.reduce((sum, e) => sum + e.cacheRead, 0),
      totalCacheWrite: entries.reduce((sum, e) => sum + e.cacheWrite, 0),
      totalMessages: entries.reduce((sum, e) => sum + e.messageCount, 0),
      totalCost: entries.reduce((sum, e) => sum + e.cost, 0),
      processingTimeMs: Math.round(performance.now() - startTime),
    };
  }

  const nativeOptions: NativeFinalizeAgentOptions = {
    homeDir: undefined,
    localMessages: options.localMessages,
    pricing: options.pricing,
    since: options.since,
    until: options.until,
    year: options.year,
  };

  return runInSubprocess<AgentReport>("finalizeAgentReport", [nativeOptions]);
}
