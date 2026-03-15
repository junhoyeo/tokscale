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

export interface DeviceClientData {
  [clientName: string]: ClientBreakdownData;
}

export interface SourceBreakdown {
  [clientName: string]: ClientBreakdownData | Record<string, DeviceClientData> | undefined;
  devices?: Record<string, DeviceClientData>;
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
  clientBreakdown: Record<string, ClientBreakdownData> | SourceBreakdown
): DayTotals {
  let tokens = 0;
  let cost = 0;
  let inputTokens = 0;
  let outputTokens = 0;
  let cacheReadTokens = 0;
  let cacheWriteTokens = 0;
  let reasoningTokens = 0;

  for (const [key, value] of Object.entries(clientBreakdown)) {
    if (key === "devices" || !isClientBreakdownData(value)) continue;
    const client = value;
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
  existing: Record<string, ClientBreakdownData> | SourceBreakdown | null | undefined,
  incoming: Record<string, ClientBreakdownData>,
  incomingClients: Set<string>,
  deviceId: string
): SourceBreakdown {
  const normalizedDeviceId = deviceId || "__legacy__";
  const devices = normalizeDevices(existing);
  const deviceClients: DeviceClientData = { ...(devices[normalizedDeviceId] || {}) };

  for (const clientName of incomingClients) {
    const incomingClient = incoming[clientName];
    if (incomingClient) {
      deviceClients[clientName] = normalizeClientBreakdown(incomingClient);
    } else {
      delete deviceClients[clientName];
    }
  }

  if (Object.keys(deviceClients).length > 0) {
    devices[normalizedDeviceId] = deviceClients;
  } else {
    delete devices[normalizedDeviceId];
  }

  return recalculateClientAggregate(devices);
}

export function recalculateClientAggregate(
  devices: Record<string, DeviceClientData>
): SourceBreakdown {
  const aggregated: SourceBreakdown = {
    devices: cloneDevices(devices),
  };

  for (const deviceClients of Object.values(devices)) {
    for (const [clientName, clientData] of Object.entries(deviceClients)) {
      if (!isClientBreakdownData(clientData)) continue;

      const normalized = normalizeClientBreakdown(clientData);
      const existing = aggregated[clientName];

      if (!isClientBreakdownData(existing)) {
        aggregated[clientName] = cloneClientBreakdown(normalized);
        continue;
      }

      existing.tokens += normalized.tokens;
      existing.cost += normalized.cost;
      existing.input += normalized.input;
      existing.output += normalized.output;
      existing.cacheRead += normalized.cacheRead;
      existing.cacheWrite += normalized.cacheWrite;
      existing.reasoning += normalized.reasoning;
      existing.messages += normalized.messages;

      for (const [modelId, modelData] of Object.entries(normalized.models)) {
        const existingModel = existing.models[modelId];
        if (!existingModel) {
          existing.models[modelId] = { ...modelData };
          continue;
        }
        existingModel.tokens += modelData.tokens;
        existingModel.cost += modelData.cost;
        existingModel.input += modelData.input;
        existingModel.output += modelData.output;
        existingModel.cacheRead += modelData.cacheRead;
        existingModel.cacheWrite += modelData.cacheWrite;
        existingModel.reasoning += modelData.reasoning;
        existingModel.messages += modelData.messages;
      }

      const modelIds = Object.keys(existing.models);
      if (modelIds.length > 0) {
        existing.modelId = modelIds[0];
      }
    }
  }

  return aggregated;
}

export function buildModelBreakdown(
  clientBreakdown: Record<string, ClientBreakdownData> | SourceBreakdown
): Record<string, number> {
  const result: Record<string, number> = {};

  for (const [key, value] of Object.entries(clientBreakdown)) {
    if (key === "devices" || !isClientBreakdownData(value)) continue;
    const client = value;
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

function isClientBreakdownData(value: unknown): value is ClientBreakdownData {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ClientBreakdownData>;
  return (
    typeof candidate.tokens === "number" &&
    typeof candidate.cost === "number" &&
    typeof candidate.input === "number" &&
    typeof candidate.output === "number" &&
    typeof candidate.cacheRead === "number" &&
    typeof candidate.cacheWrite === "number" &&
    typeof candidate.messages === "number"
  );
}

function normalizeDevices(
  source: Record<string, ClientBreakdownData> | SourceBreakdown | null | undefined
): Record<string, DeviceClientData> {
  const normalizedDevices: Record<string, DeviceClientData> = {};
  if (!source) return normalizedDevices;

  const sourceDevices = (source as SourceBreakdown).devices;
  if (sourceDevices && typeof sourceDevices === "object") {
    for (const [existingDeviceId, existingDeviceClients] of Object.entries(sourceDevices)) {
      const nextDeviceClients: DeviceClientData = {};
      for (const [clientName, clientData] of Object.entries(existingDeviceClients || {})) {
        if (!isClientBreakdownData(clientData)) continue;
        nextDeviceClients[clientName] = normalizeClientBreakdown(clientData);
      }
      if (Object.keys(nextDeviceClients).length > 0) {
        normalizedDevices[existingDeviceId] = nextDeviceClients;
      }
    }
    return normalizedDevices;
  }

  const legacyClients: DeviceClientData = {};
  for (const [clientName, clientData] of Object.entries(source)) {
    if (clientName === "devices" || !isClientBreakdownData(clientData)) continue;
    legacyClients[clientName] = normalizeClientBreakdown(clientData);
  }

  if (Object.keys(legacyClients).length > 0) {
    normalizedDevices["__legacy__"] = legacyClients;
  }

  return normalizedDevices;
}

function normalizeClientBreakdown(clientData: ClientBreakdownData): ClientBreakdownData {
  const normalizedModels: Record<string, ModelBreakdownData> = {};

  if (clientData.models && Object.keys(clientData.models).length > 0) {
    for (const [modelId, modelData] of Object.entries(clientData.models)) {
      normalizedModels[modelId] = {
        tokens: modelData.tokens || 0,
        cost: modelData.cost || 0,
        input: modelData.input || 0,
        output: modelData.output || 0,
        cacheRead: modelData.cacheRead || 0,
        cacheWrite: modelData.cacheWrite || 0,
        reasoning: modelData.reasoning || 0,
        messages: modelData.messages || 0,
      };
    }
  }

  if (Object.keys(normalizedModels).length === 0) {
    const fallbackModelId = (clientData.modelId || "unknown").trim() || "unknown";
    normalizedModels[fallbackModelId] = {
      tokens: clientData.tokens || 0,
      cost: clientData.cost || 0,
      input: clientData.input || 0,
      output: clientData.output || 0,
      cacheRead: clientData.cacheRead || 0,
      cacheWrite: clientData.cacheWrite || 0,
      reasoning: clientData.reasoning || 0,
      messages: clientData.messages || 0,
    };
  }

  const modelIds = Object.keys(normalizedModels);
  return {
    tokens: clientData.tokens || 0,
    cost: clientData.cost || 0,
    input: clientData.input || 0,
    output: clientData.output || 0,
    cacheRead: clientData.cacheRead || 0,
    cacheWrite: clientData.cacheWrite || 0,
    reasoning: clientData.reasoning || 0,
    messages: clientData.messages || 0,
    models: normalizedModels,
    modelId: modelIds[0],
  };
}

function cloneClientBreakdown(clientData: ClientBreakdownData): ClientBreakdownData {
  return {
    ...clientData,
    models: Object.fromEntries(
      Object.entries(clientData.models).map(([modelId, modelData]) => [modelId, { ...modelData }])
    ),
  };
}

function cloneDevices(devices: Record<string, DeviceClientData>): Record<string, DeviceClientData> {
  const cloned: Record<string, DeviceClientData> = {};
  for (const [deviceId, clients] of Object.entries(devices)) {
    const nextClients: DeviceClientData = {};
    for (const [clientName, clientData] of Object.entries(clients)) {
      nextClients[clientName] = cloneClientBreakdown(clientData);
    }
    cloned[deviceId] = nextClients;
  }
  return cloned;
}
