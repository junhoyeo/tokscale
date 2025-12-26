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

/**
 * Per-device contribution data for cross-machine aggregation.
 * Each device (identified by apiTokenId) tracks its own usage.
 */
export interface DeviceSourceData {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
  models: Record<string, ModelBreakdownData>;
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
  /** Per-device contributions for cross-machine aggregation */
  devices?: Record<string, DeviceSourceData>;
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

function recalculateSourceAggregate(source: SourceBreakdownData): void {
  if (!source.devices || Object.keys(source.devices).length === 0) return;

  source.tokens = 0;
  source.cost = 0;
  source.input = 0;
  source.output = 0;
  source.cacheRead = 0;
  source.cacheWrite = 0;
  source.reasoning = 0;
  source.messages = 0;
  source.models = {};

  for (const deviceData of Object.values(source.devices)) {
    source.tokens += deviceData.tokens;
    source.cost += deviceData.cost;
    source.input += deviceData.input;
    source.output += deviceData.output;
    source.cacheRead += deviceData.cacheRead;
    source.cacheWrite += deviceData.cacheWrite;
    source.reasoning += deviceData.reasoning || 0;
    source.messages += deviceData.messages;

    for (const [modelId, modelData] of Object.entries(deviceData.models)) {
      if (!source.models[modelId]) {
        source.models[modelId] = { ...modelData };
      } else {
        const m = source.models[modelId];
        m.tokens += modelData.tokens;
        m.cost += modelData.cost;
        m.input += modelData.input;
        m.output += modelData.output;
        m.cacheRead += modelData.cacheRead;
        m.cacheWrite += modelData.cacheWrite;
        m.reasoning += modelData.reasoning || 0;
        m.messages += modelData.messages;
      }
    }
  }
}

export function mergeSourceBreakdowns(
  existing: Record<string, SourceBreakdownData> | null | undefined,
  incoming: Record<string, SourceBreakdownData>,
  incomingSources: Set<string>,
  deviceId: string
): Record<string, SourceBreakdownData> {
  const merged: Record<string, SourceBreakdownData> = JSON.parse(
    JSON.stringify(existing || {})
  );

  for (const sourceName of incomingSources) {
    if (incoming[sourceName]) {
      const incomingSource = incoming[sourceName];

      if (!merged[sourceName]) {
        merged[sourceName] = {
          tokens: 0,
          cost: 0,
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 0,
          models: {},
          devices: {},
        };
      }

      if (!merged[sourceName].devices) {
        merged[sourceName].devices = {
          __legacy__: {
            tokens: merged[sourceName].tokens,
            cost: merged[sourceName].cost,
            input: merged[sourceName].input,
            output: merged[sourceName].output,
            cacheRead: merged[sourceName].cacheRead,
            cacheWrite: merged[sourceName].cacheWrite,
            reasoning: merged[sourceName].reasoning || 0,
            messages: merged[sourceName].messages,
            models: { ...merged[sourceName].models },
          },
        };
      }

      merged[sourceName].devices![deviceId] = {
        tokens: incomingSource.tokens,
        cost: incomingSource.cost,
        input: incomingSource.input,
        output: incomingSource.output,
        cacheRead: incomingSource.cacheRead,
        cacheWrite: incomingSource.cacheWrite,
        reasoning: incomingSource.reasoning || 0,
        messages: incomingSource.messages,
        models: { ...incomingSource.models },
      };

      recalculateSourceAggregate(merged[sourceName]);
    } else {
      if (merged[sourceName]?.devices?.[deviceId]) {
        delete merged[sourceName].devices![deviceId];
        if (Object.keys(merged[sourceName].devices!).length === 0) {
          delete merged[sourceName];
        } else {
          recalculateSourceAggregate(merged[sourceName]);
        }
      }
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
