import {
  deriveClientBreakdownProvenance,
  type ClientBreakdownData,
  type ModelBreakdownData,
} from "./helpers";
import { createSafeRecord, ownValue } from "../safeRecord";

/**
 * Clients whose submissions are bounded by a device/client lifetime
 * high-water instead of being merged day by day, mapped to the parser
 * generation the server accepts.
 *
 * A client belongs here when its parser can re-attribute usage it has already
 * submitted. The per-day merge guard refuses a decrease per (day, client), so
 * a re-attribution that moves tokens between days pins the days that fall and
 * writes the days that rise, inflating the stored total by exactly what moved.
 *
 * Copilot is pinned at generation 2 because generation 1 counted differently
 * and must not advance the high-water. Droid is registered at generation 1,
 * the generation every CLI already declares: its shapes differ in *where*
 * tokens land, never in the lifetime total. A full snapshot that still
 * covers the credited lifetime therefore replaces stored days so the web
 * graph matches the TUI, without the per-day merge guard inflating totals.
 *
 * Both Antigravity clients are registered at generation 1, the generation every
 * CLI already declares. Their parsers stopped dating usage at the session's
 * start: `antigravity-cli` reads a per-generation stamp out of `gen_metadata`,
 * and `antigravity` correlates standalone rows to trajectory steps. A rescan
 * therefore moves unchanged usage off the session-start day and onto the days
 * the work actually happened, which is precisely the shape the per-day guard
 * turns into permanent inflation. Antigravity CLI is registered at
 * generation 1 for the same reason: its turns used to be dated at the session
 * start and are now dated by the timestamp of the generation that produced
 * them, so a rescan spreads an unchanged session across the days it actually
 * ran without changing what it spent.
 */
export const SUPPORTED_VERSIONED_PARSERS: Readonly<Record<string, number>> = {
  copilot: 2,
  droid: 1,
  "antigravity-cli": 1,
  antigravity: 1,
};

/**
 * Parsers whose lifetime total is stable across re-attribution. A full
 * snapshot that covers the credited aggregate may replace stored days so the
 * web graph matches the TUI without the per-day merge guard inflating totals.
 *
 * Replacing is destructive by design: a day the snapshot no longer dates loses
 * its cell. So local history the user deleted is erased too, as long as the
 * remaining growth still covers the credited aggregate. Below that cover the
 * snapshot cannot replace anything and the bounded-growth plan applies, which
 * keeps every stored row.
 */
const SNAPSHOT_LAYOUT_CLIENTS: ReadonlySet<string> = new Set(["droid"]);

const TOKEN_FIELDS = [
  "input",
  "output",
  "cacheRead",
  "cacheWrite",
  "reasoning",
] as const;
type TokenField = (typeof TOKEN_FIELDS)[number];
const AGGREGATE_FIELDS = [
  "tokens",
  ...TOKEN_FIELDS,
  "messages",
  "inputIncludingCacheRead",
] as const;

export interface ParserAggregateHighWater {
  tokens: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
  /** Device/client input before cache-read normalization. */
  inputIncludingCacheRead: number;
}

export interface ParserClientHighWaterState {
  /** Server-side allocation-state schema; absent identifies the legacy envelope. */
  stateVersion?: number;
  version: number;
  /** False when identity was accepted from a partial scan but no baseline exists. */
  baselineEstablished?: boolean;
  aggregate: ParserAggregateHighWater;
  /** Stored/credited cells available for attribution of bounded aggregate growth. */
  days: Record<string, ClientBreakdownData>;
  /** Last complete parser snapshot, used only to locate the next positive delta. */
  observedDays?: Record<string, ClientBreakdownData>;
}

export type DeviceParserStates = Record<string, ParserClientHighWaterState>;

export interface IncomingParserContribution {
  date: string;
  clients: Array<{
    client: string;
    modelId: string;
    tokens: {
      input: number;
      output: number;
      cacheRead: number;
      cacheWrite: number;
      reasoning?: number;
    };
    cost: number;
    messages: number;
  }>;
}

export type ParserHighWaterMode =
  | "status-quo"
  | "freeze"
  | "baseline-legacy"
  | "baseline-new"
  | "incremental"
  | "replace";

export interface ParserHighWaterPlan {
  mode: ParserHighWaterMode;
  increments: Record<string, ClientBreakdownData>;
  /** Absolute cells when `mode` is `replace`; omitted otherwise. */
  layoutDays?: Record<string, ClientBreakdownData>;
  nextState?: ParserClientHighWaterState;
}

function copyModels(
  source?: Record<string, ModelBreakdownData>
): Record<string, ModelBreakdownData> {
  const result = createSafeRecord<ModelBreakdownData>();
  for (const [modelId, model] of Object.entries(source ?? {})) {
    const normalized = emptyModel();
    for (const field of TOKEN_FIELDS) {
      normalized[field] = positive(model[field] ?? 0);
    }
    normalized.tokens = TOKEN_FIELDS.reduce(
      (sum, field) => sum + normalized[field],
      0
    );
    normalized.cost = quantizeCost(model.cost ?? 0);
    normalized.messages = positive(model.messages ?? 0);
    normalized.inputIncludingCacheRead = positive(
      model.inputIncludingCacheRead ??
        normalized.input + normalized.cacheRead
    );
    result[modelId] = normalized;
  }
  return result;
}

function modelsForHighWater(
  breakdown?: ClientBreakdownData
): Record<string, ModelBreakdownData> {
  const models = copyModels(breakdown?.models);
  if (!breakdown) return models;

  // Legacy rows can carry a scalar total alongside a partial nested model
  // breakdown. Preserve the unrepresented remainder as a real model cell;
  // otherwise normalizing the credited ledger would silently shrink it and a
  // later replay could re-credit tokens or messages that are already stored.
  const represented = emptyModel();
  for (const model of Object.values(models)) {
    for (const field of TOKEN_FIELDS) represented[field] += model[field] || 0;
    represented.cost += model.cost || 0;
    represented.messages += model.messages || 0;
  }
  const remainder = emptyModel();
  for (const field of TOKEN_FIELDS) {
    remainder[field] = positive((breakdown[field] || 0) - represented[field]);
  }
  remainder.tokens = TOKEN_FIELDS.reduce(
    (sum, field) => sum + remainder[field],
    0
  );
  remainder.cost = quantizeCost(
    positive((breakdown.cost || 0) - represented.cost)
  );
  remainder.messages = positive(
    (breakdown.messages || 0) - represented.messages
  );
  if (remainder.tokens > 0 || remainder.cost > 0 || remainder.messages > 0) {
    const remainderModel = breakdown.modelId || "unknown";
    const prior = { ...(ownValue(models, remainderModel) ?? emptyModel()) };
    const priorInclusiveInput = positive(
      prior.inputIncludingCacheRead ?? prior.input + prior.cacheRead
    );
    for (const field of TOKEN_FIELDS) prior[field] += remainder[field];
    prior.inputIncludingCacheRead =
      priorInclusiveInput + remainder.input + remainder.cacheRead;
    prior.tokens = TOKEN_FIELDS.reduce((sum, field) => sum + prior[field], 0);
    prior.cost = quantizeCost(prior.cost + remainder.cost);
    prior.messages += remainder.messages;
    models[remainderModel] = prior;
  }
  return models;
}

function emptyAggregate(): ParserAggregateHighWater {
  return {
    tokens: 0,
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages: 0,
    inputIncludingCacheRead: 0,
  };
}

function emptyModel(): ModelBreakdownData {
  return {
    tokens: 0,
    cost: 0,
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages: 0,
  };
}

function modelFromContribution(
  contribution: IncomingParserContribution["clients"][number]
): ModelBreakdownData {
  const input = contribution.tokens.input;
  const output = contribution.tokens.output;
  const cacheRead = contribution.tokens.cacheRead;
  const cacheWrite = contribution.tokens.cacheWrite;
  const reasoning = contribution.tokens.reasoning ?? 0;
  return {
    tokens: input + output + cacheRead + cacheWrite + reasoning,
    cost: contribution.cost,
    input,
    output,
    cacheRead,
    cacheWrite,
    reasoning,
    messages: contribution.messages,
    inputIncludingCacheRead: input + cacheRead,
  };
}

function breakdownFromModels(
  models: Record<string, ModelBreakdownData>
): ClientBreakdownData {
  const breakdown: ClientBreakdownData = {
    ...emptyModel(),
    models,
  };
  for (const model of Object.values(models)) {
    model.cost = quantizeCost(model.cost);
    breakdown.tokens += model.tokens;
    breakdown.cost += model.cost;
    breakdown.input += model.input;
    breakdown.output += model.output;
    breakdown.cacheRead += model.cacheRead;
    breakdown.cacheWrite += model.cacheWrite;
    breakdown.reasoning += model.reasoning;
    breakdown.messages += model.messages;
  }
  breakdown.cost = quantizeCost(breakdown.cost);
  breakdown.provenance = deriveClientBreakdownProvenance(breakdown);
  return breakdown;
}

export function foldParserClientSnapshot(
  contributions: IncomingParserContribution[],
  client: string
): Record<string, ClientBreakdownData> {
  const modelsByDay = new Map<string, Record<string, ModelBreakdownData>>();
  for (const day of contributions) {
    for (const contribution of day.clients) {
      if (contribution.client !== client) continue;
      const models = modelsByDay.get(day.date) ?? createSafeRecord<ModelBreakdownData>();
      const incoming = modelFromContribution(contribution);
      const model = ownValue(models, contribution.modelId) ?? emptyModel();
      for (const field of TOKEN_FIELDS) model[field] += incoming[field];
      model.inputIncludingCacheRead =
        positive(model.inputIncludingCacheRead ?? 0) +
        positive(incoming.inputIncludingCacheRead ?? incoming.input + incoming.cacheRead);
      model.tokens = TOKEN_FIELDS.reduce((sum, field) => sum + model[field], 0);
      model.cost += incoming.cost;
      model.messages += incoming.messages;
      models[contribution.modelId] = model;
      modelsByDay.set(day.date, models);
    }
  }

  const days = createSafeRecord<ClientBreakdownData>();
  for (const [date, models] of modelsByDay) {
    days[date] = breakdownFromModels(models);
  }
  return days;
}

function aggregateSnapshot(
  days: Record<string, ClientBreakdownData>
): ParserAggregateHighWater {
  const aggregate = emptyAggregate();
  for (const day of Object.values(days)) {
    for (const field of TOKEN_FIELDS) aggregate[field] += day[field] || 0;
    aggregate.tokens += day.tokens || 0;
    aggregate.messages += day.messages || 0;
    aggregate.inputIncludingCacheRead +=
      (day.input || 0) + (day.cacheRead || 0);
  }
  return aggregate;
}

const PARSER_HIGH_WATER_STATE_VERSION = 2;

function normalizeStateDays(
  source: Record<string, ClientBreakdownData>
): Record<string, ClientBreakdownData> {
  const normalized = createSafeRecord<ClientBreakdownData>();
  for (const [date, day] of Object.entries(source)) {
    normalized[date] = breakdownFromModels(modelsForHighWater(day));
  }
  return normalized;
}

function applyCreditedIncrements(
  previous: Record<string, ClientBreakdownData>,
  increments: Record<string, ClientBreakdownData>
): Record<string, ClientBreakdownData> {
  const next = normalizeStateDays(previous);
  for (const [date, increment] of Object.entries(increments)) {
    next[date] = addClientBreakdownIncrement(
      ownValue(next, date),
      increment
    );
  }
  return next;
}

function snapshotCoversCredited(
  incoming: ParserAggregateHighWater,
  credited: ParserAggregateHighWater
): boolean {
  return AGGREGATE_FIELDS.every(
    (field) => incoming[field] >= credited[field]
  );
}

function stateFromSnapshot(
  version: number,
  incomingDays: Record<string, ClientBreakdownData>
): ParserClientHighWaterState {
  const days = normalizeStateDays(incomingDays);
  return {
    stateVersion: PARSER_HIGH_WATER_STATE_VERSION,
    version,
    baselineEstablished: true,
    aggregate: aggregateSnapshot(days),
    days,
    observedDays: days,
  };
}

function replaceLayoutPlan(args: {
  client: string;
  version: number;
  incoming: ParserAggregateHighWater;
  credited: ParserAggregateHighWater;
  incomingDays: Record<string, ClientBreakdownData>;
}): ParserHighWaterPlan | null {
  if (
    !SNAPSHOT_LAYOUT_CLIENTS.has(args.client) ||
    !snapshotCoversCredited(args.incoming, args.credited)
  ) {
    return null;
  }
  return {
    mode: "replace",
    increments: createSafeRecord<ClientBreakdownData>(),
    layoutDays: normalizeStateDays(args.incomingDays),
    nextState: stateFromSnapshot(args.version, args.incomingDays),
  };
}

function stateAfterCreditedIncrements(
  version: number,
  previousDays: Record<string, ClientBreakdownData>,
  increments: Record<string, ClientBreakdownData>,
  observedDays: Record<string, ClientBreakdownData>
): ParserClientHighWaterState {
  const days = applyCreditedIncrements(previousDays, increments);
  return {
    stateVersion: PARSER_HIGH_WATER_STATE_VERSION,
    version,
    baselineEstablished: true,
    aggregate: aggregateSnapshot(days),
    days,
    observedDays: normalizeStateDays(observedDays),
  };
}

const COST_SCALE = 10_000;

function quantizeCost(value: number): number {
  return Math.round((positive(value) + Number.EPSILON) * COST_SCALE) / COST_SCALE;
}

function positive(value: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

function tokenCandidates(
  previous: ModelBreakdownData | undefined,
  incoming: ModelBreakdownData
): Record<TokenField, number> {
  const candidates = Object.fromEntries(
    TOKEN_FIELDS.map((field) => [
      field,
      positive(incoming[field] - (previous?.[field] ?? 0)),
    ])
  ) as Record<TokenField, number>;
  // Submitted `input` is already exclusive of cache reads. Reconstruct the
  // producer's inclusive input counter so a cache-composition shift is not
  // itself growth, while a genuinely fully-cached request can still advance.
  const inclusiveInputCandidate = positive(
    positive(
      incoming.inputIncludingCacheRead ??
        incoming.input + incoming.cacheRead
    ) -
      positive(
        previous?.inputIncludingCacheRead ??
          (previous?.input ?? 0) + (previous?.cacheRead ?? 0)
      )
  );
  candidates.cacheRead = Math.min(
    candidates.cacheRead,
    inclusiveInputCandidate
  );
  candidates.input = inclusiveInputCandidate - candidates.cacheRead;
  return candidates;
}

function allocateIncrements(
  previousCreditedDays: Record<string, ClientBreakdownData>,
  previousObservedDays: Record<string, ClientBreakdownData>,
  incomingDays: Record<string, ClientBreakdownData>,
  previousAggregate: ParserAggregateHighWater,
  incomingAggregate: ParserAggregateHighWater
): Record<string, ClientBreakdownData> {
  let tokenBudget = positive(incomingAggregate.tokens - previousAggregate.tokens);
  let messageBudget = positive(
    incomingAggregate.messages - previousAggregate.messages
  );
  const previousObservedAggregate = aggregateSnapshot(previousObservedDays);
  const inclusiveInputBudget = positive(
    incomingAggregate.inputIncludingCacheRead -
      previousObservedAggregate.inputIncludingCacheRead
  );
  const dates = Object.keys(incomingDays).sort((a, b) => b.localeCompare(a));
  const cells = dates.flatMap((date) => {
    const incoming = incomingDays[date];
    const creditedModels = modelsForHighWater(
      ownValue(previousCreditedDays, date)
    );
    const observedModels = modelsForHighWater(
      ownValue(previousObservedDays, date)
    );
    return Object.keys(incoming.models)
      .sort()
      .map((modelId) => {
        const model = incoming.models[modelId];
        const credited = ownValue(creditedModels, modelId);
        return {
          date,
          modelId,
          model,
          credited,
          observed: ownValue(observedModels, modelId),
          increment: emptyModel(),
          tokenCapacity: positive(model.tokens - (credited?.tokens ?? 0)),
          messageCapacity: positive(
            model.messages - (credited?.messages ?? 0)
          ),
        };
      });
  });
  const supportedCellCacheReadGrowth = cells.reduce(
    (sum, cell) =>
      sum + tokenCandidates(cell.observed, cell.model).cacheRead,
    0
  );
  const cacheReadBudget = Math.min(
    positive(
      incomingAggregate.cacheRead - previousObservedAggregate.cacheRead
    ),
    supportedCellCacheReadGrowth,
    inclusiveInputBudget
  );
  const bucketBudgets: Record<TokenField, number> = {
    // Reserve inclusive growth for cache only when BOTH the aggregate cache
    // high-water and at least one cell's observed inclusive growth support it.
    // Unsupported cache moves reserve nothing, so the remainder flows to input.
    input: inclusiveInputBudget - cacheReadBudget,
    output: positive(
      incomingAggregate.output - previousObservedAggregate.output
    ),
    cacheRead: cacheReadBudget,
    cacheWrite: positive(
      incomingAggregate.cacheWrite - previousObservedAggregate.cacheWrite
    ),
    reasoning: positive(
      incomingAggregate.reasoning - previousObservedAggregate.reasoning
    ),
  };

  const allocateTokenPass = (
    candidatesFor: (cell: (typeof cells)[number]) => Record<TokenField, number>,
    respectBucketBudgets: boolean
  ) => {
    for (const cell of cells) {
      const candidates = candidatesFor(cell);
      let cellBudget = positive(
        cell.tokenCapacity - cell.increment.tokens
      );
      for (const field of TOKEN_FIELDS) {
        const accepted = Math.min(
          candidates[field],
          tokenBudget,
          cellBudget,
          respectBucketBudgets ? bucketBudgets[field] : Number.POSITIVE_INFINITY
        );
        cell.increment[field] += accepted;
        cell.increment.tokens += accepted;
        tokenBudget -= accepted;
        cellBudget -= accepted;
        if (respectBucketBudgets) bucketBudgets[field] -= accepted;
      }
    }
  };

  // Observed deltas are the most faithful attribution signal. Run this as a
  // global pass so a newer cell's residual capacity cannot pre-empt genuine
  // growth observed in another date/model cell.
  allocateTokenPass(
    (cell) => tokenCandidates(cell.observed, cell.model),
    true
  );

  const currentDeficits = (cell: (typeof cells)[number]) =>
    Object.fromEntries(
      TOKEN_FIELDS.map((field) => [
        field,
        positive(
          cell.model[field] -
            (cell.credited?.[field] ?? 0) -
            cell.increment[field]
        ),
      ])
    ) as Record<TokenField, number>;

  // First keep residual attribution inside the aggregate bucket deltas. Then,
  // if independent bucket moves made those caps jointly infeasible, spend the
  // remaining provable lifetime growth on current per-field deficits. The
  // second pass still cannot mint usage: total and per-cell capacity remain
  // hard caps, and only successfully allocated increments advance the ledger.
  allocateTokenPass(currentDeficits, true);
  allocateTokenPass(currentDeficits, false);

  const allocateMessagePass = (
    candidateFor: (cell: (typeof cells)[number]) => number
  ) => {
    for (const cell of cells) {
      const cellBudget = positive(
        cell.messageCapacity - cell.increment.messages
      );
      const accepted = Math.min(
        positive(candidateFor(cell)),
        messageBudget,
        cellBudget
      );
      cell.increment.messages += accepted;
      messageBudget -= accepted;
    }
  };

  // Message counts use the same observed-first rule, with their own lifetime
  // budget so a tiny-token new request can still add one complete message.
  allocateMessagePass(
    (cell) => cell.model.messages - (cell.observed?.messages ?? 0)
  );
  allocateMessagePass(
    (cell) =>
      cell.model.messages -
      (cell.credited?.messages ?? 0) -
      cell.increment.messages
  );

  const modelsByDate = createSafeRecord<
    Record<string, ModelBreakdownData>
  >();
  for (const cell of cells) {
    if (cell.increment.tokens > 0) {
      const acceptedFraction =
        cell.tokenCapacity > 0
          ? cell.increment.tokens / cell.tokenCapacity
          : 0;
      const marginalCost = positive(
        cell.model.cost - (cell.credited?.cost ?? 0)
      );
      cell.increment.cost = quantizeCost(
        marginalCost * acceptedFraction
      );
    }
    if (cell.increment.tokens > 0 || cell.increment.messages > 0) {
      const models =
        ownValue(modelsByDate, cell.date) ??
        createSafeRecord<ModelBreakdownData>();
      models[cell.modelId] = cell.increment;
      modelsByDate[cell.date] = models;
    }
  }

  const increments = createSafeRecord<ClientBreakdownData>();
  for (const [date, models] of Object.entries(modelsByDate)) {
    increments[date] = breakdownFromModels(models);
  }
  return increments;
}

function validState(
  state: ParserClientHighWaterState | undefined,
  version: number
): state is ParserClientHighWaterState {
  if (
    state == null ||
    state.version !== version ||
    (state.stateVersion != null &&
      state.stateVersion !== PARSER_HIGH_WATER_STATE_VERSION) ||
    state.aggregate == null ||
    state.days == null ||
    (state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION &&
      state.observedDays == null) ||
    !AGGREGATE_FIELDS.every(
      (field) =>
        Number.isFinite(state.aggregate[field]) &&
        state.aggregate[field] >= 0
    )
  ) {
    return false;
  }

  // Allocation state v2 is an accounting ledger: its aggregate must describe
  // exactly the credited cells. A mismatch would recreate the lost-growth bug
  // by authorizing from one representation and persisting another, so freeze
  // instead of trying to repair an ambiguous v2 state.
  if (state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION) {
    const creditedAggregate = aggregateSnapshot(state.days);
    return AGGREGATE_FIELDS.every(
      (field) => creditedAggregate[field] === state.aggregate[field]
    );
  }
  return true;
}

/**
 * Build a non-destructive parser rollout plan.
 *
 * The first supported full snapshot becomes a baseline. Legacy rows are always
 * preserved; only positive lifetime growth beyond their aggregate may be added
 * at transition. Later full snapshots may add only growth bounded BOTH by
 * positive per-cell deltas and by device/client-wide cumulative high-water
 * growth. The latest observed parser layout locates real growth; when its cell
 * is already covered by a preserved legacy row, a deterministic current cell
 * with uncredited capacity receives the bounded remainder. Only increments
 * actually written advance the credited ledger, so growth cannot be lost.
 * Date/model reshuffles and deleted local history never erase stored rows.
 *
 * SNAPSHOT_LAYOUT_CLIENTS are the deliberate exception to both absolutes
 * above: once a full snapshot's aggregate covers the credited one, the stored
 * layout is replaced outright so the web graph matches the TUI, which also
 * drops the cells of days the snapshot no longer dates. A snapshot that does
 * not cover it falls back to the bounded-growth plan and keeps every row.
 */
export function planParserHighWaterSubmission(args: {
  client: string;
  incomingVersion?: number;
  fullHistory: boolean;
  existingLegacyDays: Record<string, ClientBreakdownData>;
  incomingDays: Record<string, ClientBreakdownData>;
  state?: ParserClientHighWaterState;
  persistedVersion?: number;
}): ParserHighWaterPlan {
  const supportedVersion = ownValue(SUPPORTED_VERSIONED_PARSERS, args.client);
  // A generation marker without its high-water can exist after an interrupted
  // rollout or state corruption. Re-baselining could re-credit old history, so
  // fail closed until an operator repairs the state.
  if (args.state == null && args.persistedVersion != null) {
    return { mode: "freeze", increments: {} };
  }
  if (args.incomingVersion == null) {
    return args.state ? { mode: "freeze", increments: {} } : { mode: "status-quo", increments: {} };
  }
  if (args.incomingVersion !== supportedVersion) {
    return { mode: "freeze", increments: {} };
  }
  if (!args.fullHistory) {
    if (args.state) return { mode: "freeze", increments: {} };
    // Identity is trustworthy even though coverage is not. Persist a pending
    // state so every later old/undeclared submit freezes, but do not let this
    // partial snapshot establish or advance any token/cost high-water.
    return {
      mode: "freeze",
      increments: {},
      nextState: {
        stateVersion: PARSER_HIGH_WATER_STATE_VERSION,
        version: args.incomingVersion,
        baselineEstablished: false,
        aggregate: emptyAggregate(),
        days: createSafeRecord<ClientBreakdownData>(),
        observedDays: createSafeRecord<ClientBreakdownData>(),
      },
    };
  }

  const incomingAggregate = aggregateSnapshot(args.incomingDays);
  if (args.state && !validState(args.state, supportedVersion)) {
    return { mode: "freeze", increments: {} };
  }
  if (args.state == null || args.state.baselineEstablished === false) {
    const legacyAggregate = aggregateSnapshot(args.existingLegacyDays);
    const hasLegacy =
      legacyAggregate.tokens > 0 || legacyAggregate.messages > 0;
    if (hasLegacy) {
      // Tried before the preserving transition below, so a snapshot-layout
      // client adopts the snapshot's own dating instead of preserving legacy
      // rows. Everything after this point is the fallback for the rest.
      const followed = replaceLayoutPlan({
        client: args.client,
        version: args.incomingVersion,
        incoming: incomingAggregate,
        credited: legacyAggregate,
        incomingDays: args.incomingDays,
      });
      if (followed) return followed;
    }
    // At transition, preserving all legacy rows and crediting at most positive
    // lifetime aggregate growth is non-destructive. With deleted old usage D
    // and genuinely new usage N, incoming - legacy = N - D <= N; the existing
    // rows remain untouched and the normal aggregate/cell caps allocate only
    // that provable remainder.
    const increments = hasLegacy
      ? allocateIncrements(
          args.existingLegacyDays,
          args.existingLegacyDays,
          args.incomingDays,
          legacyAggregate,
          incomingAggregate
        )
      : createSafeRecord<ClientBreakdownData>();
    const nextState = hasLegacy
      ? stateAfterCreditedIncrements(
          supportedVersion,
          args.existingLegacyDays,
          increments,
          args.incomingDays
        )
      : {
          stateVersion: PARSER_HIGH_WATER_STATE_VERSION,
          version: args.incomingVersion,
          baselineEstablished: true,
          aggregate: incomingAggregate,
          days: normalizeStateDays(args.incomingDays),
          observedDays: normalizeStateDays(args.incomingDays),
        };
    return {
      mode: hasLegacy ? "baseline-legacy" : "baseline-new",
      increments,
      nextState,
    };
  }
  // State written before allocation schema v2 stored every observed cell
  // maximum and advanced its aggregate even when no cell could accept that
  // growth. Rebuild that one-time migration baseline from the rows that were
  // actually credited, otherwise already-suppressed usage remains lost.
  const previousCreditedDays =
    args.state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION
      ? args.state.days
      : args.existingLegacyDays;
  const previousAggregate =
    args.state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION
      ? args.state.aggregate
      : aggregateSnapshot(previousCreditedDays);
  const previousObservedDays =
    args.state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION
      ? args.state.observedDays!
      : previousCreditedDays;
  const followed = replaceLayoutPlan({
    client: args.client,
    version: args.incomingVersion,
    incoming: incomingAggregate,
    credited: previousAggregate,
    incomingDays: args.incomingDays,
  });
  if (followed) return followed;
  const increments = allocateIncrements(
    previousCreditedDays,
    previousObservedDays,
    args.incomingDays,
    previousAggregate,
    incomingAggregate
  );

  return {
    mode: "incremental",
    increments,
    nextState: stateAfterCreditedIncrements(
      supportedVersion,
      previousCreditedDays,
      increments,
      args.incomingDays
    ),
  };
}

export function addClientBreakdownIncrement(
  existing: ClientBreakdownData | undefined,
  increment: ClientBreakdownData
): ClientBreakdownData {
  if (!existing) return increment;
  const models = modelsForHighWater(existing);
  for (const [modelId, delta] of Object.entries(increment.models)) {
    const model = { ...(ownValue(models, modelId) ?? emptyModel()) };
    const priorInclusiveInput = positive(
      model.inputIncludingCacheRead ?? model.input + model.cacheRead
    );
    for (const field of TOKEN_FIELDS) model[field] = (model[field] || 0) + delta[field];
    model.inputIncludingCacheRead =
      priorInclusiveInput + delta.input + delta.cacheRead;
    model.tokens = TOKEN_FIELDS.reduce((sum, field) => sum + model[field], 0);
    model.cost = quantizeCost((model.cost || 0) + delta.cost);
    model.messages = (model.messages || 0) + delta.messages;
    models[modelId] = model;
  }
  const merged = breakdownFromModels(models);
  merged.provenance = {
    ...deriveClientBreakdownProvenance(merged),
    ...(existing.provenance?.costIsComplete === false
      ? { costIsComplete: false }
      : {}),
  };
  return merged;
}
