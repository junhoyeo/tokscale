/**
 * Client-level merge helpers for submission API
 */

import { createSafeRecord, ownValue } from "../safeRecord";

export interface ModelBreakdownData {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
  /** Internal parser-state high-water for input before cache-read normalization. */
  inputIncludingCacheRead?: number;
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
  /**
   * `false` when this client's stored `cost` is a floor rather than a total —
   * the submission that wrote it could not price every token it counted
   * (#1044). Absent means complete, so rows written before this existed, and
   * every already-released CLI, keep exact semantics.
   *
   * Carried PER CLIENT, not per day, because a day's clients are written
   * independently: a healthy resubmit naming only client B must not clear the
   * incompleteness of client A, which the merge preserves untouched.
   */
  costIsComplete?: boolean;
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
  // Folded clients (had an entry in foldedClientFloors) whose existing value
  // was PRESERVED — the incoming submission was below the heal floor or
  // omitted the client entirely. `merged` holds their collapsed folded entry;
  // the caller must write the ORIGINAL raw alias keys back to storage for
  // these clients instead, otherwise the fold evidence (and with it the heal
  // floor) is destroyed by the writeback and the one heal opportunity is
  // burned by a partial resubmit — permanently re-cementing the double count
  // this mechanism exists to repair.
  foldPreservedClients: Set<string>;
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
    // Same: re-derivation must not silently promote a floored cost back to
    // complete. Only an explicitly complete write clears this, in the merge.
    ...(breakdown.provenance?.costIsComplete === false
      ? { costIsComplete: false }
      : {}),
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
  // one-key rename, mapped to the largest token count any single raw key
  // contributed to the fold. For these, a lower incoming token count is not
  // automatically a parser regression — it may be the healthy value the
  // inflated fold should be replaced with. But nothing proves an incoming
  // submission covers the full day (partial re-parses are the exact case the
  // guard exists for), so healing only happens when the incoming value is at
  // least the largest single contribution: any truthful complete-day total
  // must be >= each of the components that were summed. Below that floor the
  // normal guard still applies. A pure rename-only fold (only the legacy key
  // was ever present) is NOT included here and keeps the normal guard
  // behavior.
  foldedClientFloors?: Map<string, number>,
  // `false` when the submission declared its own pricing incomplete (#1044).
  // Its per-client costs are then floors, not totals, so a client's stored
  // cost may only rise, and the client is tagged incomplete.
  //
  // This is deliberately applied HERE rather than in SQL. The day's scalar
  // `cost` is recomputed from this merged breakdown by recalculateDayTotals,
  // so flooring the scalar in the write statement while replacing the JSON
  // wholesale would leave the row's two representations disagreeing — and
  // different readers use different ones (profile totals read the scalar,
  // filtered leaderboards sum the JSON). Flooring the breakdown keeps them
  // reconcilable by construction.
  incomingCostIsComplete: boolean = true
): MergeClientBreakdownsResult {
  const merged: Record<string, ClientBreakdownData> = { ...(existing || {}) };
  const warnings: string[] = [];
  // Every folded client starts as "preserved" and is unmarked only when the
  // incoming submission actually heals or replaces it. This also covers
  // folded clients the incoming submission never mentions (carried over by
  // the spread above), whose collapsed entry would otherwise overwrite the
  // raw alias keys on writeback just the same.
  const foldPreservedClients = new Set<string>(foldedClientFloors?.keys() ?? []);

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
        foldPreservedClients.delete(clientName);
      }
      continue;
    }

    const nextClient = withDerivedProvenance(incomingClient);
    if (existingClient && nextClient.tokens < existingClient.tokens) {
      const healFloor = foldedClientFloors?.get(clientName);
      if (healFloor !== undefined && nextClient.tokens >= healFloor) {
        // The existing value is an alias-folded double count (e.g. stale
        // "kilocode" + "kilo" summed together), not real usage history, and
        // the incoming value clears the largest single contribution to that
        // fold — consistent with a complete-day recomputation rather than a
        // partial re-parse. Let it replace the fold instead of defending it.
        //
        // The heal gate is evidence about TOKENS (incoming >= the largest
        // component), which says nothing about pricing coverage. So the cost
        // still goes through the #1044 floor: an incomplete submission heals
        // the token count but must not be trusted to restate the cost. Keeping
        // the folded (inflated) cost is no worse than declining to heal, and
        // the tag lets a later complete submission correct it exactly.
        merged[clientName] = applyCostCompleteness(
          nextClient,
          existingClient,
          incomingCostIsComplete
        );
        foldPreservedClients.delete(clientName);
        const existingTokens = formatTokens(existingClient.tokens);
        const nextTokens = formatTokens(nextClient.tokens);
        warnings.push(
          `Healed ${clientName} alias-folded double count for this same-device resubmit: replaced ${existingTokens} tokens with ${nextTokens} tokens from the incoming day${
            incomingCostIsComplete ? "" : " (cost kept as a floor: pricing was incomplete)"
          }.`
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

    merged[clientName] = applyCostCompleteness(
      nextClient,
      existingClient,
      incomingCostIsComplete
    );
    foldPreservedClients.delete(clientName);
  }

  return { merged, warnings, foldPreservedClients };
}

/**
 * Resolve one client's cost and completeness tag against what is stored.
 *
 * A complete submission overwrites exactly, clearing any earlier floor — that
 * is how a day recovers once pricing is healthy again, and it keeps legitimate
 * downward corrections working.
 *
 * An incomplete one may only raise the cost. Note the tag does not depend on
 * which value won: a floored client stays incomplete even when its own number
 * was kept, because the stored cost still may not cover the tokens now
 * recorded alongside it. Deriving the tag from the comparison instead would
 * mark a client complete whenever an incomplete rescan happened to report less
 * than the old total for a *larger* token set.
 */
export function applyCostCompleteness(
  next: ClientBreakdownData,
  existing: ClientBreakdownData | undefined,
  incomingCostIsComplete: boolean
): ClientBreakdownData {
  if (incomingCostIsComplete) {
    const { costIsComplete: _dropped, ...provenance } =
      next.provenance ?? deriveClientBreakdownProvenance(next);
    return { ...next, provenance };
  }

  // The floor has to reach the NESTED model costs, not just the client
  // aggregate. `model:`-filtered leaderboards and profile model breakdowns sum
  // `models[*].cost` rather than the client total, so flooring only the
  // aggregate leaves a row whose client says $10 while its models rank at $0 —
  // the same scalar-vs-JSON divergence one level deeper.
  //
  // Every model in the union is floored: one present in both takes the higher
  // cost, and one the incomplete payload dropped entirely is preserved rather
  // than allowed to vanish (its usage did not stop existing because this
  // submission could not price it).
  const models = createSafeRecord<ModelBreakdownData>();
  for (const [modelId, model] of Object.entries(existing?.models ?? {})) {
    models[modelId] = { ...model };
  }
  for (const [modelId, model] of Object.entries(next.models ?? {})) {
    const priorCost = ownValue(models, modelId)?.cost ?? 0;
    models[modelId] = { ...model, cost: Math.max(priorCost, model.cost) };
  }

  // Derive the client total from those floors rather than taking
  // max(existing, next) independently. Deriving is what makes the three levels
  // agree by construction: day = Σ clients (recalculateDayTotals) and now
  // client = Σ models. Taking the max here instead can land between the two —
  // e.g. existing {A:6,B:4}=10 against incoming {A:0,C:5}=5 floors to
  // {A:6,B:4,C:5}=15, which max(10,5)=10 would contradict.
  const modelCosts = Object.values(models);
  const flooredCost = modelCosts.length
    ? modelCosts.reduce((sum, model) => sum + (model.cost || 0), 0)
    : Math.max(existing?.cost ?? 0, next.cost);

  return {
    ...next,
    cost: flooredCost,
    models,
    provenance: {
      ...(next.provenance ?? deriveClientBreakdownProvenance(next)),
      costIsComplete: false,
    },
  };
}

/**
 * Tag every client in a breakdown with the submission's completeness.
 *
 * For a day with no stored row there is nothing to floor against, but the tags
 * still have to be written: a later filtered resubmit naming only healthy
 * clients must be able to see that this one was incomplete.
 */
export function tagBreakdownCostCompleteness(
  breakdown: Record<string, ClientBreakdownData>,
  costIsComplete: boolean
): Record<string, ClientBreakdownData> {
  if (costIsComplete) return breakdown;

  const tagged: Record<string, ClientBreakdownData> = {};
  for (const [clientName, client] of Object.entries(breakdown)) {
    tagged[clientName] = applyCostCompleteness(client, undefined, false);
  }
  return tagged;
}

/**
 * True when every client in a day carries a complete cost.
 *
 * Absent tags mean complete, so legacy rows and released CLIs read as
 * complete. Clients the submission never mentioned are included, which is the
 * point: a filtered resubmit of one healthy client must not clear a preserved
 * sibling's incompleteness.
 */
export function breakdownCostIsComplete(
  breakdown: Record<string, ClientBreakdownData> | null | undefined
): boolean {
  return Object.values(breakdown ?? {}).every(
    (client) => client.provenance?.costIsComplete !== false
  );
}

export function replaceLayoutCostFloors(
  existingDays: Array<{ sourceBreakdown: unknown }>,
  replaceClients: Iterable<string>
): Map<string, number> {
  const floors = new Map<string, number>();
  for (const client of replaceClients) {
    let cost = 0;
    for (const day of existingDays) {
      const breakdown = day.sourceBreakdown as Record<
        string,
        ClientBreakdownData
      > | null;
      cost += ownValue(breakdown ?? {}, client)?.cost ?? 0;
    }
    if (cost > 0) floors.set(client, cost);
  }
  return floors;
}

const COST_SCALE = 10_000;

function quantizeCost(value: number): number {
  return Math.round((value + Number.EPSILON) * COST_SCALE) / COST_SCALE;
}

function addClientCostFloor(
  cell: ClientBreakdownData,
  extra: number
): void {
  if (extra <= 0) return;
  const models = Object.values(cell.models ?? {});
  if (models.length === 0) {
    cell.cost = quantizeCost((cell.cost || 0) + extra);
  } else {
    const tokenTotal = models.reduce((sum, model) => sum + (model.tokens || 0), 0);
    let assigned = 0;
    for (let i = 0; i < models.length; i++) {
      const share =
        i === models.length - 1
          ? quantizeCost(extra - assigned)
          : tokenTotal > 0
            ? quantizeCost((extra * (models[i].tokens || 0)) / tokenTotal)
            : quantizeCost(extra / models.length);
      models[i].cost = quantizeCost((models[i].cost || 0) + share);
      assigned = quantizeCost(assigned + share);
    }
    cell.cost = quantizeCost(
      models.reduce((sum, model) => sum + (model.cost || 0), 0)
    );
  }
  cell.provenance = {
    ...(cell.provenance ?? deriveClientBreakdownProvenance(cell)),
    costIsComplete: false,
  };
}

/**
 * Same-day cost floors cannot follow tokens that a snapshot layout moves onto
 * a day with no previously stored cell. Re-apply the pre-rewrite client total
 * across the new cells so an unpriced re-date cannot drop lifetime cost.
 */
export function reapplyReplaceLayoutCostFloors(
  rows: Array<{ sourceBreakdown: Record<string, ClientBreakdownData> }>,
  floors: Map<string, number>,
  incompleteClients: Set<string>
): void {
  for (const [client, floor] of floors) {
    if (!incompleteClients.has(client)) continue;
    const cells: ClientBreakdownData[] = [];
    for (const row of rows) {
      const cell = ownValue(row.sourceBreakdown, client);
      if (cell) cells.push(cell);
    }
    if (cells.length === 0) continue;
    const current = cells.reduce((sum, cell) => sum + (cell.cost || 0), 0);
    const deficit = quantizeCost(floor - current);
    if (deficit <= 0) continue;
    const tokenTotal = cells.reduce((sum, cell) => sum + (cell.tokens || 0), 0);
    let assigned = 0;
    for (let i = 0; i < cells.length; i++) {
      const share =
        i === cells.length - 1
          ? quantizeCost(deficit - assigned)
          : tokenTotal > 0
            ? quantizeCost((deficit * cells[i].tokens) / tokenTotal)
            : quantizeCost(deficit / cells.length);
      addClientCostFloor(cells[i], share);
      assigned = quantizeCost(assigned + share);
    }
  }
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
