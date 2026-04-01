/**
 * Client-level merge helpers for submission API
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

export interface ClientBreakdownData {
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

export interface SubmissionScopeRow {
  id: string;
  sourceId: string | null;
}

export type SubmissionScopeResolution =
  | {
      kind: "existing";
      submissionId: string;
      upgradeLegacyRow: boolean;
    }
  | {
      kind: "create";
    }
  | {
      kind: "rejectMissingSourceIdentity";
    };

export function resolveSubmissionScope(
  existingRows: SubmissionScopeRow[],
  incomingSourceId: string | null
): SubmissionScopeResolution {
  const exactMatch = incomingSourceId
    ? existingRows.find((row) => row.sourceId === incomingSourceId)
    : undefined;
  if (exactMatch) {
    return {
      kind: "existing",
      submissionId: exactMatch.id,
      upgradeLegacyRow: false,
    };
  }

  const unsourcedRow =
    existingRows.find((row) => row.sourceId == null) ?? null;
  const hasScopedRows = existingRows.some((row) => row.sourceId != null);

  if (incomingSourceId) {
    if (unsourcedRow && !hasScopedRows) {
      return {
        kind: "existing",
        submissionId: unsourcedRow.id,
        upgradeLegacyRow: true,
      };
    }

    return { kind: "create" };
  }

  if (unsourcedRow) {
    return {
      kind: "existing",
      submissionId: unsourcedRow.id,
      upgradeLegacyRow: false,
    };
  }

  if (hasScopedRows) {
    return { kind: "rejectMissingSourceIdentity" };
  }

  return { kind: "create" };
}

export function recalculateDayTotals(
  clientBreakdown: Record<string, ClientBreakdownData>
): DayTotals {
  let tokens = 0;
  let cost = 0;
  let inputTokens = 0;
  let outputTokens = 0;
  let cacheReadTokens = 0;
  let cacheWriteTokens = 0;
  let reasoningTokens = 0;

  for (const client of Object.values(clientBreakdown)) {
    tokens += client.tokens || 0;
    cost += client.cost || 0;
    inputTokens += client.input || 0;
    outputTokens += client.output || 0;
    cacheReadTokens += client.cacheRead || 0;
    cacheWriteTokens += client.cacheWrite || 0;
    reasoningTokens += client.reasoning || 0;
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

export function mergeClientBreakdowns(
  existing: Record<string, ClientBreakdownData> | null | undefined,
  incoming: Record<string, ClientBreakdownData>,
  incomingClients: Set<string>
): Record<string, ClientBreakdownData> {
  const merged: Record<string, ClientBreakdownData> = { ...(existing || {}) };

  for (const clientName of incomingClients) {
    if (incoming[clientName]) {
      merged[clientName] = { ...incoming[clientName] };
    } else {
      delete merged[clientName];
    }
  }

  return merged;
}

export function buildModelBreakdown(
  clientBreakdown: Record<string, ClientBreakdownData>
): Record<string, number> {
  const result: Record<string, number> = {};

  for (const client of Object.values(clientBreakdown)) {
    if (client.models) {
      for (const [modelId, modelData] of Object.entries(client.models)) {
        result[modelId] = (result[modelId] || 0) + modelData.tokens;
      }
    } else if (client.modelId) {
      result[client.modelId] = (result[client.modelId] || 0) + client.tokens;
    }
  }

  return result;
}

export function clientContributionToBreakdownData(
  client_contrib: {
    tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; reasoning?: number };
    cost: number;
    modelId: string;
    messages: number;
  }
): ModelBreakdownData {
  const { input, output, cacheRead, cacheWrite, reasoning = 0 } = client_contrib.tokens;
  return {
    tokens: input + output + cacheRead + cacheWrite + reasoning,
    cost: client_contrib.cost,
    input,
    output,
    cacheRead,
    cacheWrite,
    reasoning,
    messages: client_contrib.messages,
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
