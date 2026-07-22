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

export interface ClientBreakdownProvenanceData {
  schemaVersion: number;
  messageCount: number;
  modelCount: number;
  /**
   * "backfill" when this client's contribution was written by a
   * backfill-origin submission (`tokscale import`); absent (or "cli") for
   * locally-scanned CLI usage. Preserved by deriveClientBreakdownProvenance
   * so merges do not silently drop the tag.
   */
  origin?: "cli" | "backfill";
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
  provenance?: ClientBreakdownProvenanceData;
  /** @deprecated Legacy field for backward compat - use models instead */
  modelId?: string;
}

export interface MergeClientBreakdownsResult {
  merged: Record<string, ClientBreakdownData>;
  warnings: string[];
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

function formatTokens(value: number): string {
  return Math.round(value).toLocaleString("en-US");
}

export function deriveClientBreakdownProvenance(
  breakdown: ClientBreakdownData
): ClientBreakdownProvenanceData {
  const modelCount = breakdown.models
    ? Object.keys(breakdown.models).length
    : breakdown.modelId
    ? 1
    : 0;

  const origin = breakdown.provenance?.origin;

  return {
    schemaVersion: Math.max(1, breakdown.provenance?.schemaVersion ?? 1),
    messageCount: Math.max(
      0,
      breakdown.provenance?.messageCount ?? 0,
      breakdown.messages ?? 0
    ),
    modelCount: Math.max(0, breakdown.provenance?.modelCount ?? 0, modelCount),
    // Carry the origin tag through re-derivation (merges, alias folding) so
    // a backfill-tagged client row keeps its tag.
    ...(origin ? { origin } : {}),
  };
}

function withDerivedProvenance(breakdown: ClientBreakdownData): ClientBreakdownData {
  return {
    ...breakdown,
    provenance: deriveClientBreakdownProvenance(breakdown),
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

export function mergeClientBreakdownsWithRegressionGuard(
  existing: Record<string, ClientBreakdownData> | null | undefined,
  incoming: Record<string, ClientBreakdownData>,
  incomingClients: Set<string>,
  // Clients whose `existing` value came from normalizeClientBreakdownAliases
  // folding TWO source keys together (e.g. a stale legacy "kilocode" key
  // alongside "kilo" for the same underlying usage) rather than a simple
  // one-key rename. For these, a lower incoming token count is not a parser
  // regression to guard against — it is the healthy, complete-day total that
  // should replace the inflated fold. A pure rename-only fold (only the
  // legacy key was ever present) is NOT included here and keeps the normal
  // guard behavior.
  foldedClients?: Set<string>
): MergeClientBreakdownsResult {
  const merged: Record<string, ClientBreakdownData> = { ...(existing || {}) };
  const warnings: string[] = [];

  for (const clientName of incomingClients) {
    const existingClient = existing?.[clientName];
    const incomingClient = incoming[clientName];

    if (!incomingClient) {
      if (existingClient && existingClient.tokens > 0) {
        merged[clientName] = withDerivedProvenance(existingClient);
        warnings.push(
          `Preserved ${clientName} because it disappeared from this same-device resubmit; kept ${formatTokens(existingClient.tokens)} tokens.`
        );
      } else {
        delete merged[clientName];
      }
      continue;
    }

    const nextClient = withDerivedProvenance(incomingClient);
    if (existingClient && nextClient.tokens < existingClient.tokens) {
      if (foldedClients?.has(clientName)) {
        // The existing value is an alias-folded double count (e.g. stale
        // "kilocode" + "kilo" summed together), not real usage history.
        // This complete-day incoming submission is authoritative for this
        // client, so let it replace the fold instead of defending it.
        merged[clientName] = nextClient;
        const existingTokens = formatTokens(existingClient.tokens);
        const nextTokens = formatTokens(nextClient.tokens);
        warnings.push(
          `Healed ${clientName} alias-folded double count for this same-device resubmit: replaced ${existingTokens} tokens with ${nextTokens} tokens from the complete incoming day.`
        );
        continue;
      }

      // A token decrease alone signals a parser regression (e.g. the CLI
      // re-parsed only a subset of history). Preserve the existing row even
      // when coverage metrics are equal, because equal coverage + fewer tokens
      // still indicates data loss. The old AND-gate (tokens < existing AND lower
      // coverage) let equal-coverage regressions slip through undetected.
      merged[clientName] = withDerivedProvenance(existingClient);
      const existingTokens = formatTokens(existingClient.tokens);
      const nextTokens = formatTokens(nextClient.tokens);
      warnings.push(
        `Preserved ${clientName} because this same-device resubmit would reduce ${existingTokens} tokens to ${nextTokens}.`
      );
      continue;
    }

    merged[clientName] = nextClient;
  }

  return { merged, warnings };
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
