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

export interface ReplayWindow {
  start: string;
  end: string;
}

export interface ExistingReplayDay {
  id: string;
  date: string;
  timestampMs: number | null;
  sourceBreakdown: Record<string, ClientBreakdownData> | null | undefined;
}

export interface IncomingReplayDay {
  date: string;
  timestampMs: number | null | undefined;
  sourceBreakdown: Record<string, ClientBreakdownData>;
}

export interface ReplayInsertDay {
  submissionId: string;
  date: string;
  tokens: number;
  cost: string;
  inputTokens: number;
  outputTokens: number;
  timestampMs: number | null;
  sourceBreakdown: Record<string, ClientBreakdownData>;
  modelBreakdown: Record<string, number>;
}

export interface ReplayUpdateDay {
  id: string;
  date: string;
  tokens: number;
  cost: string;
  inputTokens: number;
  outputTokens: number;
  timestampMs: number | null;
  sourceBreakdown: Record<string, ClientBreakdownData>;
  modelBreakdown: Record<string, number>;
}

export interface ReplayDeleteDay {
  id: string;
  date: string;
}

export interface PlannedReplayMutations {
  inserts: ReplayInsertDay[];
  updates: ReplayUpdateDay[];
  deletes: ReplayDeleteDay[];
}

export interface PlanSubmittedReplayArgs {
  existingDays: ExistingReplayDay[];
  incomingDays: IncomingReplayDay[];
  submittedClients: Set<string>;
  replayWindow: ReplayWindow;
  submissionId: string;
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

function isDateWithinReplayWindow(date: string, replayWindow: ReplayWindow): boolean {
  return date >= replayWindow.start && date <= replayWindow.end;
}

export function planSubmittedReplayMutations({
  existingDays,
  incomingDays,
  submittedClients,
  replayWindow,
  submissionId,
}: PlanSubmittedReplayArgs): PlannedReplayMutations {
  const existingByDate = new Map(existingDays.map((day) => [day.date, day]));
  const incomingByDate = new Map(incomingDays.map((day) => [day.date, day]));
  const replayDates = new Set(incomingDays.map((day) => day.date));

  for (const existingDay of existingDays) {
    if (isDateWithinReplayWindow(existingDay.date, replayWindow)) {
      replayDates.add(existingDay.date);
    }
  }

  const sortedReplayDates = Array.from(replayDates).sort((left, right) =>
    left.localeCompare(right)
  );

  const inserts: ReplayInsertDay[] = [];
  const updates: ReplayUpdateDay[] = [];
  const deletes: ReplayDeleteDay[] = [];

  for (const replayDate of sortedReplayDates) {
    const existingDay = existingByDate.get(replayDate);
    const incomingDay = incomingByDate.get(replayDate);

    const mergedClientBreakdown = existingDay
      ? mergeClientBreakdowns(
          existingDay.sourceBreakdown,
          incomingDay?.sourceBreakdown ?? {},
          submittedClients
        )
      : incomingDay?.sourceBreakdown ?? {};

    if (Object.keys(mergedClientBreakdown).length === 0) {
      if (existingDay) {
        deletes.push({
          id: existingDay.id,
          date: existingDay.date,
        });
      }
      continue;
    }

    const dayTotals = recalculateDayTotals(mergedClientBreakdown);
    const modelBreakdown = buildModelBreakdown(mergedClientBreakdown);

    if (existingDay) {
      updates.push({
        id: existingDay.id,
        date: existingDay.date,
        tokens: dayTotals.tokens,
        cost: dayTotals.cost.toFixed(4),
        inputTokens: dayTotals.inputTokens,
        outputTokens: dayTotals.outputTokens,
        timestampMs: incomingDay
          ? mergeTimestampMs(existingDay.timestampMs, incomingDay.timestampMs ?? null)
          : existingDay.timestampMs ?? null,
        sourceBreakdown: mergedClientBreakdown,
        modelBreakdown,
      });
      continue;
    }

    if (!incomingDay) {
      continue;
    }

    inserts.push({
      submissionId,
      date: incomingDay.date,
      tokens: dayTotals.tokens,
      cost: dayTotals.cost.toFixed(4),
      inputTokens: dayTotals.inputTokens,
      outputTokens: dayTotals.outputTokens,
      timestampMs: incomingDay.timestampMs ?? null,
      sourceBreakdown: mergedClientBreakdown,
      modelBreakdown,
    });
  }

  return {
    inserts,
    updates,
    deletes,
  };
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
