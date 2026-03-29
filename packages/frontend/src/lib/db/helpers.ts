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
  instances?: Record<string, ClientSourceInstanceData>;
  /** @deprecated Legacy field for backward compat - use models instead */
  modelId?: string;
}

export interface ClientSourceInstanceData {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
  models: Record<string, ModelBreakdownData>;
  sourceName?: string;
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

export function mergeClientBreakdowns(
  existing: Record<string, ClientBreakdownData> | null | undefined,
  incoming: Record<string, ClientBreakdownData>,
  incomingClients: Set<string>,
  sourceId: string,
  sourceName?: string,
): Record<string, ClientBreakdownData> {
  const merged: Record<string, ClientBreakdownData> = {};

  for (const [clientName, clientData] of Object.entries(existing || {})) {
    if (!incomingClients.has(clientName)) {
      merged[clientName] = cloneClientBreakdown(clientData, "__legacy__");
    }
  }

  for (const clientName of incomingClients) {
    const existingClient = existing?.[clientName];
    const instances = extractClientInstances(existingClient, sourceId);

    if (incoming[clientName]) {
      instances[sourceId] = {
        ...cloneSourceInstance(incoming[clientName]),
        sourceName,
      };
      merged[clientName] = aggregateClientInstances(instances);
    } else {
      delete instances[sourceId];
      if (Object.keys(instances).length === 0) {
        delete merged[clientName];
      } else {
        merged[clientName] = aggregateClientInstances(instances);
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

function cloneModelBreakdown(modelData: ModelBreakdownData): ModelBreakdownData {
  return { ...modelData };
}

function cloneSourceInstance(instance: ClientSourceInstanceData | ClientBreakdownData): ClientSourceInstanceData {
  return {
    tokens: instance.tokens || 0,
    cost: instance.cost || 0,
    input: instance.input || 0,
    output: instance.output || 0,
    cacheRead: instance.cacheRead || 0,
    cacheWrite: instance.cacheWrite || 0,
    reasoning: instance.reasoning || 0,
    messages: instance.messages || 0,
    models: Object.fromEntries(
      Object.entries(instance.models || {}).map(([modelId, modelData]) => [
        modelId,
        cloneModelBreakdown(modelData),
      ])
    ),
    modelId: instance.modelId,
    sourceName: "sourceName" in instance ? instance.sourceName : undefined,
  };
}

function extractClientInstances(
  clientData: ClientBreakdownData | undefined,
  fallbackSourceId: string,
): Record<string, ClientSourceInstanceData> {
  if (!clientData) {
    return {};
  }

  if (clientData.instances && Object.keys(clientData.instances).length > 0) {
    return Object.fromEntries(
      Object.entries(clientData.instances).map(([instanceId, instance]) => [
        instanceId,
        cloneSourceInstance(instance),
      ])
    );
  }

  return {
    [fallbackSourceId]: cloneSourceInstance(clientData),
  };
}

function aggregateClientInstances(
  instances: Record<string, ClientSourceInstanceData>
): ClientBreakdownData {
  let tokens = 0;
  let cost = 0;
  let input = 0;
  let output = 0;
  let cacheRead = 0;
  let cacheWrite = 0;
  let reasoning = 0;
  let messages = 0;
  const models: Record<string, ModelBreakdownData> = {};

  for (const instance of Object.values(instances)) {
    tokens += instance.tokens || 0;
    cost += instance.cost || 0;
    input += instance.input || 0;
    output += instance.output || 0;
    cacheRead += instance.cacheRead || 0;
    cacheWrite += instance.cacheWrite || 0;
    reasoning += instance.reasoning || 0;
    messages += instance.messages || 0;

    for (const [modelId, modelData] of Object.entries(instance.models || {})) {
      if (models[modelId]) {
        models[modelId].tokens += modelData.tokens || 0;
        models[modelId].cost += modelData.cost || 0;
        models[modelId].input += modelData.input || 0;
        models[modelId].output += modelData.output || 0;
        models[modelId].cacheRead += modelData.cacheRead || 0;
        models[modelId].cacheWrite += modelData.cacheWrite || 0;
        models[modelId].reasoning += modelData.reasoning || 0;
        models[modelId].messages += modelData.messages || 0;
      } else {
        models[modelId] = cloneModelBreakdown(modelData);
      }
    }
  }

  const instanceEntries = Object.entries(instances);
  const singleModelId =
    instanceEntries.length === 1 ? instanceEntries[0][1].modelId : undefined;

  return {
    tokens,
    cost,
    input,
    output,
    cacheRead,
    cacheWrite,
    reasoning,
    messages,
    models,
    instances: Object.fromEntries(
      instanceEntries.map(([instanceId, instance]) => [
        instanceId,
        cloneSourceInstance(instance),
      ])
    ),
    modelId: singleModelId,
  };
}

function cloneClientBreakdown(
  clientData: ClientBreakdownData,
  fallbackSourceId: string,
): ClientBreakdownData {
  return aggregateClientInstances(extractClientInstances(clientData, fallbackSourceId));
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
