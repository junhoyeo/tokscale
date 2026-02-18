/**
 * Source-level merge helpers for submission API
 */

export interface ModelBreakdownData {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
}

export interface SourceBreakdownData {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
  models: Record<string, ModelBreakdownData>;
  /** @deprecated Legacy field for backward compat - use models instead */
  modelId?: string;
}

export interface DayTotals {
  tokens: number;
  cost: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  reasoningTokens: number;
}

export function recalculateDayTotals(
  sourceBreakdown: Record<string, SourceBreakdownData>
): DayTotals {
  let tokens = 0;
  let cost = 0;
  let inputTokens = 0;
  let outputTokens = 0;
  let cacheReadTokens = 0;
  let cacheWriteTokens = 0;
  let reasoningTokens = 0;

  for (const source of Object.values(sourceBreakdown)) {
    tokens += source.tokens || 0;
    cost += source.cost || 0;
    inputTokens += source.input || 0;
    outputTokens += source.output || 0;
    cacheReadTokens += source.cacheRead || 0;
    cacheWriteTokens += source.cacheWrite || 0;
    reasoningTokens += source.reasoning || 0;
  }

  return {
    tokens,
    cost,
    inputTokens,
    outputTokens,
    cacheReadTokens,
    cacheWriteTokens,
    reasoningTokens,
  };
}

export function mergeSourceBreakdowns(
  existing: Record<string, SourceBreakdownData> | null | undefined,
  incoming: Record<string, SourceBreakdownData>,
  incomingSources: Set<string>
): Record<string, SourceBreakdownData> {
  const merged: Record<string, SourceBreakdownData> = { ...(existing || {}) };

  for (const sourceName of incomingSources) {
    if (incoming[sourceName]) {
      merged[sourceName] = { ...incoming[sourceName] };
    } else {
      delete merged[sourceName];
    }
  }

  return merged;
}

export function buildModelBreakdown(
  sourceBreakdown: Record<string, SourceBreakdownData>
): Record<string, number> {
  const result: Record<string, number> = {};

  for (const source of Object.values(sourceBreakdown)) {
    if (source.models) {
      for (const [modelId, modelData] of Object.entries(source.models)) {
        result[modelId] = (result[modelId] || 0) + modelData.tokens;
      }
    } else if (source.modelId) {
      result[source.modelId] = (result[source.modelId] || 0) + source.tokens;
    }
  }

  return result;
}

export function sourceContributionToBreakdownData(
  source: {
    tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; reasoning?: number };
    cost: number;
    modelId: string;
    messages: number;
  }
): ModelBreakdownData {
  const { input, output, cacheRead, cacheWrite, reasoning = 0 } = source.tokens;
  return {
    tokens: input + output + cacheRead + cacheWrite + reasoning,
    cost: source.cost,
    input,
    output,
    cacheRead,
    cacheWrite,
    reasoning,
    messages: source.messages,
  };
}

/**
 * Merge two nullable timestamps, keeping the earliest non-null value.
 * Used by both submit and profile aggregation to maintain consistent merge semantics.
 */
export function mergeTimestampMs(
  existing: number | null | undefined,
  incoming: number | null | undefined,
): number | null {
  if (incoming != null && existing != null) return Math.min(existing, incoming);
  return incoming ?? existing ?? null;
}
