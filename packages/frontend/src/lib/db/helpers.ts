/**
 * Submission merge helpers for per-client, per-device daily breakdown state.
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

export const LEGACY_DEVICE_ID = "__legacy__";

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

  if (!client.devices || Object.keys(client.devices).length === 0) {
    return;
  }

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
      const existingModel = client.models[modelId];

      if (existingModel) {
        existingModel.tokens += Number(modelData.tokens) || 0;
        existingModel.cost += Number(modelData.cost) || 0;
        existingModel.input += Number(modelData.input) || 0;
        existingModel.output += Number(modelData.output) || 0;
        existingModel.cacheRead += Number(modelData.cacheRead) || 0;
        existingModel.cacheWrite += Number(modelData.cacheWrite) || 0;
        existingModel.reasoning += Number(modelData.reasoning) || 0;
        existingModel.messages += Number(modelData.messages) || 0;
      } else {
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
      }
    }
  }
}

function toDeviceClientData(client: ClientBreakdownData): DeviceClientData {
  return {
    tokens: Number(client.tokens) || 0,
    cost: Number(client.cost) || 0,
    input: Number(client.input) || 0,
    output: Number(client.output) || 0,
    cacheRead: Number(client.cacheRead) || 0,
    cacheWrite: Number(client.cacheWrite) || 0,
    reasoning: Number(client.reasoning) || 0,
    messages: Number(client.messages) || 0,
    models: structuredClone(client.models ?? {}),
  };
}

export function attachDeviceContributions(
  incoming: Record<string, ClientBreakdownData>,
  deviceId: string
): Record<string, ClientBreakdownData> {
  const withDevices = structuredClone(incoming);

  for (const [clientName, clientData] of Object.entries(withDevices)) {
    withDevices[clientName] = {
      ...clientData,
      devices: {
        [deviceId]: toDeviceClientData(clientData),
      },
    };
  }

  return withDevices;
}

export function mergeClientBreakdowns(
  existing: Record<string, ClientBreakdownData> | null | undefined,
  incoming: Record<string, ClientBreakdownData>,
  incomingClients: Set<string>,
  deviceId: string
): Record<string, ClientBreakdownData> {
  const merged = structuredClone(existing || {}) as Record<string, ClientBreakdownData>;

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

      if (!merged[clientName].devices) {
        const legacyModels =
          merged[clientName].models &&
          Object.keys(merged[clientName].models).length > 0
            ? { ...merged[clientName].models }
            : merged[clientName].modelId?.trim()
              ? {
                  [merged[clientName].modelId]: {
                    tokens: merged[clientName].tokens,
                    cost: merged[clientName].cost,
                    input: merged[clientName].input,
                    output: merged[clientName].output,
                    cacheRead: merged[clientName].cacheRead,
                    cacheWrite: merged[clientName].cacheWrite,
                    reasoning: Number(merged[clientName].reasoning) || 0,
                    messages: merged[clientName].messages,
                  },
                }
              : {};

        merged[clientName].devices = {
          [LEGACY_DEVICE_ID]: {
            tokens: Number(merged[clientName].tokens) || 0,
            cost: Number(merged[clientName].cost) || 0,
            input: Number(merged[clientName].input) || 0,
            output: Number(merged[clientName].output) || 0,
            cacheRead: Number(merged[clientName].cacheRead) || 0,
            cacheWrite: Number(merged[clientName].cacheWrite) || 0,
            reasoning: Number(merged[clientName].reasoning) || 0,
            messages: Number(merged[clientName].messages) || 0,
            models: legacyModels,
          },
        };
      }

      merged[clientName].devices![deviceId] = toDeviceClientData(incomingClient);

      recalculateClientAggregate(merged[clientName]);
      continue;
    }

    if (merged[clientName]?.devices?.[deviceId]) {
      delete merged[clientName].devices![deviceId];

      if (Object.keys(merged[clientName].devices!).length === 0) {
        delete merged[clientName];
      } else {
        recalculateClientAggregate(merged[clientName]);
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
