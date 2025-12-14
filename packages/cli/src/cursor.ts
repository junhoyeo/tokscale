/**
 * Cursor IDE API Client
 * Fetches usage data from Cursor's dashboard API via CSV export
 *
 * API Endpoint: https://cursor.com/api/dashboard/export-usage-events-csv?strategy=tokens
 * Authentication: WorkosCursorSessionToken cookie
 *
 * CSV Format:
 * Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost,Cost to you
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import { parse as parseCsv } from "csv-parse/sync";

// ============================================================================
// Types
// ============================================================================

export interface CursorCredentials {
  sessionToken: string;
  userId?: string;
  createdAt: string;
  expiresAt?: string;
}

export interface CursorUsageRow {
  date: string; // YYYY-MM-DD
  timestamp: number; // Unix milliseconds
  model: string;
  inputWithCacheWrite: number;
  inputWithoutCacheWrite: number;
  cacheRead: number;
  outputTokens: number;
  totalTokens: number;
  apiCost: number; // in USD
  costToYou: number; // in USD
}

export interface CursorUsageData {
  source: "cursor";
  model: string;
  providerId: string;
  messageCount: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  cost: number;
}

export interface CursorMessageWithTimestamp {
  source: "cursor";
  model: string;
  providerId: string;
  timestamp: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  cost: number;
}

// ============================================================================
// Credential Management
// ============================================================================

const OLD_CONFIG_DIR = path.join(os.homedir(), ".tokscale");
const CONFIG_DIR = path.join(os.homedir(), ".config", "tokscale");
const OLD_CURSOR_CREDENTIALS_FILE = path.join(OLD_CONFIG_DIR, "cursor-credentials.json");
const CURSOR_CREDENTIALS_FILE = path.join(CONFIG_DIR, "cursor-credentials.json");

function ensureConfigDir(): void {
  if (!fs.existsSync(CONFIG_DIR)) {
    fs.mkdirSync(CONFIG_DIR, { recursive: true, mode: 0o700 });
  }
}

/**
 * Migrate Cursor credentials and cache from old path to new XDG path
 */
function migrateCursorFromOldPath(): void {
  try {
    // Migrate cursor credentials
    if (!fs.existsSync(CURSOR_CREDENTIALS_FILE) && fs.existsSync(OLD_CURSOR_CREDENTIALS_FILE)) {
      ensureConfigDir();
      fs.copyFileSync(OLD_CURSOR_CREDENTIALS_FILE, CURSOR_CREDENTIALS_FILE);
      fs.chmodSync(CURSOR_CREDENTIALS_FILE, 0o600);
      fs.unlinkSync(OLD_CURSOR_CREDENTIALS_FILE);
    }

    // Migrate cache directory (handled after CURSOR_CACHE_DIR is defined)
    // Cache migration happens in migrateCursorCacheFromOldPath()

    // Try to remove old config directory if empty
    try {
      fs.rmdirSync(OLD_CONFIG_DIR);
    } catch {
      // Directory not empty - ignore
    }
  } catch {
    // Migration failed - continue with normal operation
  }
}

export function saveCursorCredentials(credentials: CursorCredentials): void {
  ensureConfigDir();
  fs.writeFileSync(CURSOR_CREDENTIALS_FILE, JSON.stringify(credentials, null, 2), {
    encoding: "utf-8",
    mode: 0o600,
  });
}

export function loadCursorCredentials(): CursorCredentials | null {
  migrateCursorFromOldPath();
  try {
    if (!fs.existsSync(CURSOR_CREDENTIALS_FILE)) {
      return null;
    }
    const data = fs.readFileSync(CURSOR_CREDENTIALS_FILE, "utf-8");
    const parsed = JSON.parse(data);

    if (!parsed.sessionToken) {
      return null;
    }

    return parsed as CursorCredentials;
  } catch {
    return null;
  }
}

export function clearCursorCredentials(): boolean {
  try {
    if (fs.existsSync(CURSOR_CREDENTIALS_FILE)) {
      fs.unlinkSync(CURSOR_CREDENTIALS_FILE);
      return true;
    }
    return false;
  } catch {
    return false;
  }
}

export function isCursorLoggedIn(): boolean {
  return loadCursorCredentials() !== null;
}

// ============================================================================
// API Client
// ============================================================================

const CURSOR_API_BASE = "https://cursor.com";
const USAGE_CSV_ENDPOINT = `${CURSOR_API_BASE}/api/dashboard/export-usage-events-csv?strategy=tokens`;
const USAGE_SUMMARY_ENDPOINT = `${CURSOR_API_BASE}/api/usage-summary`;

/**
 * Build HTTP headers for Cursor API requests
 */
function buildCursorHeaders(sessionToken: string): Record<string, string> {
  return {
    Accept: "*/*",
    "Accept-Language": "en-US,en;q=0.9",
    Cookie: `WorkosCursorSessionToken=${sessionToken}`,
    Referer: "https://www.cursor.com/settings",
    "User-Agent":
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
  };
}

/**
 * Validate Cursor session token by hitting the usage-summary endpoint
 */
export async function validateCursorSession(
  sessionToken: string
): Promise<{ valid: boolean; membershipType?: string; error?: string }> {
  try {
    const response = await fetch(USAGE_SUMMARY_ENDPOINT, {
      method: "GET",
      headers: buildCursorHeaders(sessionToken),
    });

    if (response.status === 401 || response.status === 403) {
      return { valid: false, error: "Session token expired or invalid" };
    }

    if (!response.ok) {
      return { valid: false, error: `API returned status ${response.status}` };
    }

    const data = await response.json();

    // Check for required fields that indicate valid auth
    if (data.billingCycleStart && data.billingCycleEnd) {
      return { valid: true, membershipType: data.membershipType };
    }

    return { valid: false, error: "Invalid response format" };
  } catch (error) {
    return { valid: false, error: (error as Error).message };
  }
}

/**
 * Fetch usage CSV from Cursor API
 */
export async function fetchCursorUsageCsv(sessionToken: string): Promise<string> {
  const response = await fetch(USAGE_CSV_ENDPOINT, {
    method: "GET",
    headers: buildCursorHeaders(sessionToken),
  });

  if (response.status === 401 || response.status === 403) {
    throw new Error("Cursor session expired. Please run 'tokscale cursor login' to re-authenticate.");
  }

  if (!response.ok) {
    throw new Error(`Cursor API returned status ${response.status}`);
  }

  const text = await response.text();

  // Validate it's actually CSV (handle both old and new formats)
  // Old: "Date,Model,..."
  // New: "Date,Kind,Model,..."
  if (!text.startsWith("Date,")) {
    throw new Error("Invalid response from Cursor API - expected CSV format");
  }

  return text;
}

// ============================================================================
// CSV Parsing
// ============================================================================

/**
 * Parse cost string (e.g., "$0.50" or "0.50") to number
 */
function parseCost(costStr: string): number {
  if (!costStr) return 0;
  const cleaned = costStr.replace(/[$,]/g, "").trim();
  const value = parseFloat(cleaned);
  return isNaN(value) ? 0 : value;
}

/**
 * Infer provider from model name
 */
function inferProvider(model: string): string {
  const lowerModel = model.toLowerCase();

  if (lowerModel.includes("claude") || lowerModel.includes("sonnet") || lowerModel.includes("opus") || lowerModel.includes("haiku")) {
    return "anthropic";
  }
  if (lowerModel.includes("gpt") || lowerModel.includes("o1") || lowerModel.includes("o3")) {
    return "openai";
  }
  if (lowerModel.includes("gemini")) {
    return "google";
  }
  if (lowerModel.includes("deepseek")) {
    return "deepseek";
  }
  if (lowerModel.includes("llama") || lowerModel.includes("mixtral")) {
    return "meta";
  }

  return "cursor"; // Default provider
}

/**
 * Parse Cursor usage CSV into structured rows
 */
export function parseCursorCsv(csvText: string): CursorUsageRow[] {
  try {
    const records: Array<Record<string, string>> = parseCsv(csvText, {
      columns: true,
      skip_empty_lines: true,
      trim: true,
      relax_column_count: true,
    });

    return records
      .filter((record) => record["Date"] && record["Model"])
      .map((record) => {
        const dateStr = record["Date"] || "";
        const date = new Date(dateStr);
        const isValidDate = !isNaN(date.getTime());
        const dateOnly = isValidDate
          ? date.toISOString().slice(0, 10)
          : dateStr.length >= 10
            ? dateStr.slice(0, 10)
            : dateStr;

        return {
          date: dateOnly,
          timestamp: isValidDate ? date.getTime() : 0,
          model: (record["Model"] || "").trim(),
          inputWithCacheWrite: parseInt(record["Input (w/ Cache Write)"] || "0", 10),
          inputWithoutCacheWrite: parseInt(record["Input (w/o Cache Write)"] || "0", 10),
          cacheRead: parseInt(record["Cache Read"] || "0", 10),
          outputTokens: parseInt(record["Output Tokens"] || "0", 10),
          totalTokens: parseInt(record["Total Tokens"] || "0", 10),
          apiCost: parseCost(record["Cost"] || record["API Cost"] || "0"),
          costToYou: parseCost(record["Cost to you"] || "0"),
        };
      });
  } catch (error) {
    throw new Error(`Failed to parse Cursor CSV: ${(error as Error).message}`);
  }
}

// ============================================================================
// Data Aggregation (for table display)
// ============================================================================

/**
 * Aggregate Cursor usage by model
 */
export function aggregateCursorByModel(rows: CursorUsageRow[]): CursorUsageData[] {
  const modelMap = new Map<string, CursorUsageData>();

  for (const row of rows) {
    const key = row.model;
    const existing = modelMap.get(key);

    // Cache write = inputWithCacheWrite - inputWithoutCacheWrite (tokens written to cache)
    const cacheWrite = Math.max(0, row.inputWithCacheWrite - row.inputWithoutCacheWrite);
    // Input tokens (without cache) = inputWithoutCacheWrite
    const input = row.inputWithoutCacheWrite;

    if (existing) {
      existing.messageCount += 1;
      existing.input += input;
      existing.output += row.outputTokens;
      existing.cacheRead += row.cacheRead;
      existing.cacheWrite += cacheWrite;
      existing.cost += row.costToYou || row.apiCost;
    } else {
      modelMap.set(key, {
        source: "cursor",
        model: row.model,
        providerId: inferProvider(row.model),
        messageCount: 1,
        input,
        output: row.outputTokens,
        cacheRead: row.cacheRead,
        cacheWrite,
        reasoning: 0, // Cursor doesn't expose reasoning tokens
        cost: row.costToYou || row.apiCost,
      });
    }
  }

  return Array.from(modelMap.values()).sort((a, b) => b.cost - a.cost);
}

// ============================================================================
// Data Conversion (for graph/native module)
// ============================================================================

/**
 * Convert Cursor CSV rows to timestamped messages for graph generation
 */
export function cursorRowsToMessages(rows: CursorUsageRow[]): CursorMessageWithTimestamp[] {
  return rows.map((row) => {
    const cacheWrite = Math.max(0, row.inputWithCacheWrite - row.inputWithoutCacheWrite);
    const input = row.inputWithoutCacheWrite;

    return {
      source: "cursor" as const,
      model: row.model,
      providerId: inferProvider(row.model),
      timestamp: row.timestamp,
      input,
      output: row.outputTokens,
      cacheRead: row.cacheRead,
      cacheWrite,
      reasoning: 0,
      cost: row.costToYou || row.apiCost,
    };
  });
}

// ============================================================================
// High-Level API
// ============================================================================

/**
 * Fetch and parse Cursor usage data
 * Requires valid credentials to be stored
 */
export async function readCursorUsage(): Promise<{
  rows: CursorUsageRow[];
  byModel: CursorUsageData[];
  messages: CursorMessageWithTimestamp[];
}> {
  const credentials = loadCursorCredentials();
  if (!credentials) {
    throw new Error("Cursor not authenticated. Run 'tokscale cursor login' first.");
  }

  const csvText = await fetchCursorUsageCsv(credentials.sessionToken);
  const rows = parseCursorCsv(csvText);
  const byModel = aggregateCursorByModel(rows);
  const messages = cursorRowsToMessages(rows);

  return { rows, byModel, messages };
}

/**
 * Get Cursor credentials file path (for debugging)
 */
export function getCursorCredentialsPath(): string {
  return CURSOR_CREDENTIALS_FILE;
}

// ============================================================================
// Cache Management (for Rust integration)
// ============================================================================

const OLD_CURSOR_CACHE_DIR = path.join(os.homedir(), ".tokscale", "cursor-cache");
const CURSOR_CACHE_DIR = path.join(CONFIG_DIR, "cursor-cache");
const CURSOR_CACHE_FILE = path.join(CURSOR_CACHE_DIR, "usage.csv");

function ensureCacheDir(): void {
  if (!fs.existsSync(CURSOR_CACHE_DIR)) {
    fs.mkdirSync(CURSOR_CACHE_DIR, { recursive: true, mode: 0o700 });
  }
}

/**
 * Migrate cursor cache from old path to new XDG path
 */
function migrateCursorCacheFromOldPath(): void {
  try {
    if (!fs.existsSync(CURSOR_CACHE_DIR) && fs.existsSync(OLD_CURSOR_CACHE_DIR)) {
      ensureCacheDir();
      fs.cpSync(OLD_CURSOR_CACHE_DIR, CURSOR_CACHE_DIR, { recursive: true });
      fs.rmSync(OLD_CURSOR_CACHE_DIR, { recursive: true });
    }

    // Try to remove old config directory if empty
    try {
      fs.rmdirSync(OLD_CONFIG_DIR);
    } catch {
      // Directory not empty - ignore
    }
  } catch {
    // Migration failed - continue with normal operation
  }
}

/**
 * Sync Cursor usage data from API to local cache
 * This downloads the CSV and saves it for the Rust module to parse
 */
export async function syncCursorCache(): Promise<{ synced: boolean; rows: number; error?: string }> {
  migrateCursorCacheFromOldPath();
  const credentials = loadCursorCredentials();
  if (!credentials) {
    return { synced: false, rows: 0, error: "Not authenticated" };
  }

  try {
    const csvText = await fetchCursorUsageCsv(credentials.sessionToken);
    ensureCacheDir();
    fs.writeFileSync(CURSOR_CACHE_FILE, csvText, { encoding: "utf-8", mode: 0o600 });

    // Count rows for feedback
    const rows = parseCursorCsv(csvText);
    return { synced: true, rows: rows.length };
  } catch (error) {
    return { synced: false, rows: 0, error: (error as Error).message };
  }
}

/**
 * Get the cache file path
 */
export function getCursorCachePath(): string {
  return CURSOR_CACHE_FILE;
}

/**
 * Check if cache exists and when it was last updated
 */
export function getCursorCacheStatus(): { exists: boolean; lastModified?: Date; path: string } {
  const exists = fs.existsSync(CURSOR_CACHE_FILE);
  let lastModified: Date | undefined;

  if (exists) {
    try {
      const stats = fs.statSync(CURSOR_CACHE_FILE);
      lastModified = stats.mtime;
    } catch {
      // Ignore stat errors
    }
  }

  return { exists, lastModified, path: CURSOR_CACHE_FILE };
}
