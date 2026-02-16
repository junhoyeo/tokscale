/**
 * Unified message types for session parsers (matches Rust UnifiedMessage)
 */

export interface TokenBreakdown {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
}

export interface UnifiedMessage {
  source: string;
  modelId: string;
  providerId: string;
  sessionId: string;
  timestamp: number;
  date: string;
  tokens: TokenBreakdown;
  cost: number;
  agent?: string;
}

export type SourceType = "opencode" | "claude" | "codex" | "gemini" | "cursor" | "amp" | "droid" | "openclaw" | "pi";

/**
 * Convert Unix milliseconds timestamp to YYYY-MM-DD date string
 */
export function timestampToDate(timestampMs: number): string {
  const date = new Date(timestampMs);
  const year = date.getUTCFullYear();
  const month = String(date.getUTCMonth() + 1).padStart(2, "0");
  const day = String(date.getUTCDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function createUnifiedMessage(
  source: string,
  modelId: string,
  providerId: string,
  sessionId: string,
  timestamp: number,
  tokens: TokenBreakdown,
  cost: number = 0,
  agent?: string
): UnifiedMessage {
  return {
    source,
    modelId,
    providerId,
    sessionId,
    timestamp,
    date: timestampToDate(timestamp),
    tokens,
    cost,
    agent,
  };
}
