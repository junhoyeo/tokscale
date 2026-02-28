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

/**
 * Per-device contribution data for cross-machine aggregation.
 * Each device (identified by apiTokenId) tracks its own usage.
 */
export interface DeviceClientData {
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
  /** Per-device contributions for cross-machine aggregation */
  devices?: Record<string, DeviceClientData>;
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

/**
 * Recalculate a client's aggregate totals and models from its device-level data.
 * Called after device contributions are added/removed to keep client totals consistent.
 */
function recalculateClientAggregate(client: ClientBreakdownData): void {
  client.tokens = 0;
  client.cost = 0;
  client.input = 0;
  client.output = 0;
  client.cacheRead = 0;
  client.cacheWrite = 0;
  client.reasoning = 0;
  client.messages = 0;
  client.models = {};

  if (!client.devices || Object.keys(client.devices).length === 0) return;

  for (const deviceData of Object.values(client.devices)) {
    client.tokens += Number(deviceData.tokens) || 0;
    client.cost += Number(deviceData.cost) || 0;
    client.input += Number(deviceData.input) || 0;
    client.output += Number(deviceData.output) || 0;
    client.cacheRead += Number(deviceData.cacheRead) || 0;
    client.cacheWrite += Number(deviceData.cacheWrite) || 0;
    client.reasoning += Number(deviceData.reasoning) || 0;
    client.messages += Number(deviceData.messages) || 0;

    for (const [modelId, modelData] of Object.entries(deviceData.models ?? {})) {
      if (!client.models[modelId]) {
        client.models[modelId] = {
          tokens: Number(modelData.tokens) || 0,
          cost: Number(modelData.cost) || 0,
          input: Number(modelData.input) || 0,
          output: Number(modelData.output) || 0,
          cacheRead: Number(modelData.cacheRead) || 0,
          cacheWrite: Number(modelData.cacheWrite) || 0,
          reasoning: Number(modelData.reasoning) || 0,
          messages: Number(modelData.messages) || 0,
        };
      } else {
        const m = client.models[modelId];
        m.tokens += Number(modelData.tokens) || 0;
        m.cost += Number(modelData.cost) || 0;
        m.input += Number(modelData.input) || 0;
        m.output += Number(modelData.output) || 0;
        m.cacheRead += Number(modelData.cacheRead) || 0;
        m.cacheWrite += Number(modelData.cacheWrite) || 0;
        m.reasoning += Number(modelData.reasoning) || 0;
        m.messages += Number(modelData.messages) || 0;
      }
    }
  }
}

export function mergeClientBreakdowns(
  existing: Record<string, ClientBreakdownData> | null | undefined,
  incoming: Record<string, ClientBreakdownData>,
  incomingClients: Set<string>,
  deviceId: string
): Record<string, ClientBreakdownData> {
  const merged: Record<string, ClientBreakdownData> = JSON.parse(
    JSON.stringify(existing || {})
  );

  for (const clientName of incomingClients) {
    if (incoming[clientName]) {
      const incomingClient = incoming[clientName];

      if (!merged[clientName]) {
        merged[clientName] = {
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

      // MIGRATION: If existing client has NO devices field (legacy data),
      // preserve historical data under "__legacy__" device key
      if (!merged[clientName].devices) {
        const hasModels = merged[clientName].models &&
                          Object.keys(merged[clientName].models).length > 0;

        const legacyModels: Record<string, ModelBreakdownData> = hasModels
          ? { ...merged[clientName].models }
          : merged[clientName].modelId?.trim()
            ? {
                [merged[clientName].modelId!]: {
                  tokens: merged[clientName].tokens,
                  cost: merged[clientName].cost,
                  input: merged[clientName].input,
                  output: merged[clientName].output,
                  cacheRead: merged[clientName].cacheRead,
                  cacheWrite: merged[clientName].cacheWrite,
                  reasoning: merged[clientName].reasoning || 0,
                  messages: merged[clientName].messages,
                },
              }
            : {};

        merged[clientName].devices = {
          "__legacy__": {
            tokens: merged[clientName].tokens,
            cost: merged[clientName].cost,
            input: merged[clientName].input,
            output: merged[clientName].output,
            cacheRead: merged[clientName].cacheRead,
            cacheWrite: merged[clientName].cacheWrite,
            reasoning: merged[clientName].reasoning || 0,
            messages: merged[clientName].messages,
            models: legacyModels,
          },
        };
      }

      // REPLACE this device's contribution (handles resubmits correctly).
      // Other devices' contributions are preserved.
      merged[clientName].devices![deviceId] = {
        tokens: incomingClient.tokens,
        cost: incomingClient.cost,
        input: incomingClient.input,
        output: incomingClient.output,
        cacheRead: incomingClient.cacheRead,
        cacheWrite: incomingClient.cacheWrite,
        reasoning: Number(incomingClient.reasoning) || 0,
        messages: incomingClient.messages,
        models: { ...(incomingClient.models ?? {}) },
      };

      recalculateClientAggregate(merged[clientName]);
    } else {
      // Client declared in submission but has no data: remove this device's contribution
      if (merged[clientName]?.devices?.[deviceId]) {
        delete merged[clientName].devices![deviceId];
        if (Object.keys(merged[clientName].devices!).length === 0) {
          delete merged[clientName];
        } else {
          recalculateClientAggregate(merged[clientName]);
        }
      }
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
