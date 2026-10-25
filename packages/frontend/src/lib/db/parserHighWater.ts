import {
  deriveClientBreakdownProvenance,
  type ClientBreakdownData,
  type ModelBreakdownData,
} from "./helpers";
import { createSafeRecord, ownValue } from "../safeRecord";

/**
 * Parser generations accepted for re-attributing clients. Most use a
 * device/client lifetime high-water; source families that can overlap use an
 * atomic family transition instead of independent client ledgers.
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
 * Antigravity parsers are registered at generation 1. The database parser is
 * shared by the CLI and IDE extension, and reads a per-generation stamp out of
 * `gen_metadata`; the `antigravity` parser correlates standalone rows to
 * trajectory steps. All three source labels are admitted and reconciled as one
 * family because the same provider response can appear in multiple surfaces.
 *
 * Devin CLI is registered at generation 2: generation 1 deduplicated
 * `message_nodes` rows by database row id and counted the same API request
 * once per persisted copy, while generation 2 deduplicates by
 * `metadata.request_id`. Because that generation counts the same history
 * strictly LOWER, neither the merge guard nor the high-water can heal the
 * inflation — both only ever defend stored values. See RECOUNT_PROOF_CLIENTS.
 */
export const SUPPORTED_VERSIONED_PARSERS: Readonly<Record<string, number>> = {
  copilot: 2,
  droid: 1,
  "devin-cli": 2,
  // v2 moves exported Amp messages from the synthesized thread-created day to
  // their server-provided usage timestamp. This is a pure re-attribution, so
  // the receiver must replace the daily layout instead of applying its normal
  // non-decreasing per-day merge guard.
  amp: 2,
  // Membership and accepted generations for the atomic Antigravity family
  // transition; the submit route deliberately skips independent client plans.
  "antigravity-cli": 1,
  "antigravity-extension": 1,
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
const SNAPSHOT_LAYOUT_CLIENTS: ReadonlySet<string> = new Set(["droid", "amp"]);

/**
 * Clients whose REGISTERED generation counts the same history differently in
 * absolute terms — unlike SNAPSHOT_LAYOUT_CLIENTS, where tokens only move
 * between days, a recounting generation legitimately lowers the credited
 * lifetime (Devin CLI gen 2 drops duplicate-row counting).
 *
 * Every ledger this file maintains is monotonic, so deflation cannot flow
 * through increments or a coverage-gated replace: a stored inflated cell
 * would be defended forever. A lower snapshot alone is not evidence either —
 * a database that failed to open or history the user deleted both lower it
 * too, and replacing a stored cell with such a snapshot deletes real usage.
 *
 * These clients therefore ship a per-day recount proof with every
 * full-history snapshot (`scanScope.recountProofs`): for each (day, model)
 * cell, what the previous generation counted from the same rows and what
 * this generation counts from them. The receiver subtracts ONLY that proven
 * over-count from its stored cell, never below the corrected count, and
 * never from usage the proof does not describe. A source that failed to
 * read contributes to neither side of the proof, so its stored usage is left
 * exactly as it was. The subtraction is idempotent and monotone, so it runs
 * on every submit rather than once: a source that comes back later, or cost
 * that could only be priced later, heals on the next complete snapshot.
 *
 * A non-fullHistory submit still freezes, so a partial scan never corrects
 * anything, and an empty snapshot keeps the transition pending rather than
 * establishing a baseline the parser never actually read.
 */
const RECOUNT_PROOF_CLIENTS: ReadonlySet<string> = new Set(["devin-cli"]);

/** One (day, model) cell as a parser generation counts it. */
export interface RecountProofCell {
  tokens: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    reasoning: number;
  };
  cost: number;
  messages: number;
}

/** What two parser generations count for one cell from the same local rows. */
export interface RecountProofEntry {
  previous: RecountProofCell;
  corrected: RecountProofCell;
}

/** `date -> model id -> proof`, keyed like the submitted contributions. */
export type ClientRecountProof = Record<string, Record<string, RecountProofEntry>>;

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
  /**
   * Days whose recount proof agrees with this snapshot. The credited ledger
   * behind `nextState` already has the proven over-count subtracted on these
   * days; the route applies `applyRecountProofToCell` to the stored cells of
   * the same days before adding `increments`.
   */
  recountDays?: string[];
  /** Days whose recount proof disagreed with the snapshot and was ignored. */
  rejectedRecountDays?: string[];
  nextState?: ParserClientHighWaterState;
  /**
   * Tokens the credited lifetime baseline holds beyond this snapshot. Positive
   * means no token growth is allocatable; messages have an independent budget
   * and can still advance.
   */
  highWaterDeficit?: number;
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

export function modelsForHighWater(
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

export const PARSER_HIGH_WATER_STATE_VERSION = 2;

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

/**
 * Identity accepted, baseline deferred: the pending state freezes every
 * older or undeclared submit for this client without crediting anything.
 */
function pendingTransitionPlan(version: number): ParserHighWaterPlan {
  return {
    mode: "freeze",
    increments: createSafeRecord<ClientBreakdownData>(),
    nextState: {
      stateVersion: PARSER_HIGH_WATER_STATE_VERSION,
      version,
      baselineEstablished: false,
      aggregate: emptyAggregate(),
      days: createSafeRecord<ClientBreakdownData>(),
      observedDays: createSafeRecord<ClientBreakdownData>(),
    },
  };
}

const PROOF_TOKEN_FIELDS = TOKEN_FIELDS;

function proofCellIsOrdered(entry: RecountProofEntry): boolean {
  return (
    PROOF_TOKEN_FIELDS.every(
      (field) =>
        Number.isFinite(entry.previous.tokens[field]) &&
        Number.isFinite(entry.corrected.tokens[field]) &&
        entry.previous.tokens[field] >= entry.corrected.tokens[field]
    ) &&
    Number.isFinite(entry.previous.cost) &&
    Number.isFinite(entry.corrected.cost) &&
    entry.previous.cost >= entry.corrected.cost &&
    Number.isFinite(entry.previous.messages) &&
    Number.isFinite(entry.corrected.messages) &&
    entry.previous.messages >= entry.corrected.messages
  );
}

/**
 * Whether one day's proof describes exactly the snapshot it accompanies.
 *
 * The proof's `corrected` side must reproduce the submitted cell, token field
 * by token field and in message count, for every model either side names.
 * That is what ties the proof to "the same bytes": a proof for rows the
 * snapshot does not contain, or a snapshot with rows the proof never saw,
 * fails here and is ignored for the day. A day the snapshot does not date at
 * all is an empty cell: the proof may describe it only as rows the corrected
 * generation drops entirely.
 *
 * Cost is tied one-sidedly. Submission zeroes cost it cannot price
 * authoritatively AFTER the proof is built, so the submitted cell may carry
 * less cost than the proof's corrected side, never more. A proof whose
 * corrected cost sits below the snapshot would let the correction floor
 * stored spend under what this very scan reports, so it is refused.
 */
export function recountProofDayMatchesSnapshot(
  proofDay: Record<string, RecountProofEntry> | undefined,
  incoming: ClientBreakdownData | undefined
): boolean {
  if (!proofDay) return false;
  const incomingModels = incoming
    ? modelsForHighWater(incoming)
    : createSafeRecord<ModelBreakdownData>();
  const modelIds = new Set([
    ...Object.keys(proofDay),
    ...Object.keys(incomingModels),
  ]);
  if (modelIds.size === 0) return false;
  for (const modelId of modelIds) {
    const entry = ownValue(proofDay, modelId);
    const model = ownValue(incomingModels, modelId) ?? emptyModel();
    if (!entry || !proofCellIsOrdered(entry)) return false;
    if (
      PROOF_TOKEN_FIELDS.some(
        (field) => entry.corrected.tokens[field] !== model[field]
      ) ||
      entry.corrected.messages !== model.messages ||
      model.cost - entry.corrected.cost > PROOF_COST_TOLERANCE
    ) {
      return false;
    }
  }
  return true;
}

/**
 * Split a client's recount proof into the days that may be applied against
 * this snapshot and the days that must be ignored.
 */
export function partitionRecountProof(
  proof: ClientRecountProof | undefined,
  incomingDays: Record<string, ClientBreakdownData>
): { accepted: string[]; rejected: string[] } {
  const accepted: string[] = [];
  const rejected: string[] = [];
  for (const date of Object.keys(proof ?? {}).sort()) {
    if (
      recountProofDayMatchesSnapshot(
        ownValue(proof ?? {}, date),
        ownValue(incomingDays, date)
      )
    ) {
      accepted.push(date);
    } else {
      rejected.push(date);
    }
  }
  return { accepted, rejected };
}

/** Subtract `delta` from `stored`, but never below `floor`. */
function subtractProven(stored: number, delta: number, floor: number): number {
  const room = positive(stored - floor);
  return stored - Math.min(positive(delta), room);
}

export interface RecountCorrection {
  cell: ClientBreakdownData;
  /** What the correction removed from the stored cell. */
  removed: { tokens: number; cost: number; messages: number };
}

/**
 * Apply one day's recount proof to a stored cell.
 *
 * Per model and per field the stored value drops by the proven over-count
 * (`previous - corrected`), but never below the corrected count. The floor is
 * what makes this safe to run on every submit and against any stored cell:
 *
 * - a cell already at the corrected count loses nothing (idempotent);
 * - a cell carrying usage the proof never saw — a database that failed to
 *   open, history deleted locally — keeps all of it, because only the proven
 *   duplicate amount is removed;
 * - a cell BELOW the corrected count (new usage not yet credited) is left for
 *   the bounded-growth allocation to raise, never lowered;
 * - a source that becomes readable again, or cost that becomes priceable
 *   later, enlarges the proof and the extra over-count comes off then.
 *
 * Models the proof does not name are untouched; proof models the cell does
 * not hold have nothing to correct.
 */
export function applyRecountProofToCell(
  stored: ClientBreakdownData,
  proofDay: Record<string, RecountProofEntry>
): RecountCorrection {
  const models = modelsForHighWater(stored);
  const removed = { tokens: 0, cost: 0, messages: 0 };
  for (const [modelId, entry] of Object.entries(proofDay)) {
    const model = ownValue(models, modelId);
    if (!model || !proofCellIsOrdered(entry)) continue;
    const next = { ...model };
    const priorInclusiveInput = positive(
      model.inputIncludingCacheRead ?? model.input + model.cacheRead
    );
    for (const field of PROOF_TOKEN_FIELDS) {
      next[field] = subtractProven(
        model[field],
        entry.previous.tokens[field] - entry.corrected.tokens[field],
        entry.corrected.tokens[field]
      );
      removed.tokens += model[field] - next[field];
    }
    next.inputIncludingCacheRead = positive(
      priorInclusiveInput -
        (model.input - next.input) -
        (model.cacheRead - next.cacheRead)
    );
    next.tokens = PROOF_TOKEN_FIELDS.reduce((sum, field) => sum + next[field], 0);
    next.cost = quantizeCost(
      subtractProven(
        model.cost,
        entry.previous.cost - entry.corrected.cost,
        entry.corrected.cost
      )
    );
    removed.cost += model.cost - next.cost;
    next.messages = subtractProven(
      model.messages,
      entry.previous.messages - entry.corrected.messages,
      entry.corrected.messages
    );
    removed.messages += model.messages - next.messages;
    models[modelId] = next;
  }
  const cell = breakdownFromModels(models);
  // Re-derive provenance from the corrected totals, carrying the stored
  // origin and cost-completeness tags: a correction restates amounts, not
  // where they came from or whether their cost was fully priced.
  cell.provenance = deriveClientBreakdownProvenance({
    ...cell,
    provenance: stored.provenance,
  });
  removed.cost = quantizeCost(removed.cost);
  return { cell, removed };
}

function applyRecountProofToDays(
  days: Record<string, ClientBreakdownData>,
  proof: ClientRecountProof,
  dates: readonly string[]
): Record<string, ClientBreakdownData> {
  const next = normalizeStateDays(days);
  for (const date of dates) {
    const stored = ownValue(next, date);
    const proofDay = ownValue(proof, date);
    if (!stored || !proofDay) continue;
    next[date] = applyRecountProofToCell(stored, proofDay).cell;
  }
  return next;
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

/**
 * Submitted and proof costs are sums of the same per-message prices, so they
 * agree to floating-point noise; this is the slack allowed before a cost is
 * treated as a different number (one unit of the stored cost scale).
 */
const PROOF_COST_TOLERANCE = 1 / COST_SCALE;

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
  // Keep the complete credited lifetime in the budget. A client-reported
  // retention boundary cannot prove that a missing old cell was pruned rather
  // than re-dated into this snapshot, so excluding it could credit it twice.
  let tokenBudget = positive(incomingAggregate.tokens - previousAggregate.tokens);
  let messageBudget = positive(
    incomingAggregate.messages - previousAggregate.messages
  );
  // Per-cell capacity below still reads the FULL credited ledger, so a
  // re-reported day cannot be credited twice.
  const previousObservedAggregate = aggregateSnapshot(previousObservedDays);
  const inclusiveInputBudget = positive(
    incomingAggregate.inputIncludingCacheRead -
      previousObservedAggregate.inputIncludingCacheRead
  );
  // Residual allocation is intentionally newest-first. ISO day keys sort by
  // code point, avoiding a host's default locale changing which cell wins.
  const dates = Object.keys(incomingDays).sort((a, b) =>
    a === b ? 0 : a < b ? 1 : -1
  );
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
export function scanScopeCoversFullHistory(
  scope: { fullHistory: boolean; fullHistoryClients?: readonly string[] } | undefined,
  clients: readonly string[],
): boolean {
  if (scope == null) return false;
  if (scope.fullHistoryClients == null) return scope.fullHistory;
  const complete = new Set(scope.fullHistoryClients);
  return clients.every((client) => complete.has(client));
}

export function planParserHighWaterSubmission(args: {
  client: string;
  incomingVersion?: number;
  fullHistory: boolean;
  /**
   * Client-reported earliest `YYYY-MM-DD` still retained locally. It is
   * preserved for the submission protocol but cannot relax the high-water
   * without server-verifiable evidence of that retention transition.
   */
  retentionFloor?: string;
  existingLegacyDays: Record<string, ClientBreakdownData>;
  incomingDays: Record<string, ClientBreakdownData>;
  state?: ParserClientHighWaterState;
  persistedVersion?: number;
  /**
   * The client's `scanScope.recountProofs` entry. Honored only for
   * RECOUNT_PROOF_CLIENTS and only on a full-history snapshot that is not
   * frozen; see `applyRecountProofToCell`.
   */
  recountProof?: ClientRecountProof;
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
  // Amp v1 and Devin CLI v1 are already released when their receivers deploy.
  // Keep accepting them for devices that have not transitioned yet, but never
  // let an older CLI rewrite a device after the receiver has started the new
  // generation. This preserves the existing submit behavior during the
  // server-before-CLI rollout without reopening a completed (or pending)
  // transition to the old parser.
  if (
    (args.client === "amp" || args.client === "devin-cli") &&
    args.incomingVersion === 1
  ) {
    return args.state
      ? { mode: "freeze", increments: {} }
      : { mode: "status-quo", increments: {} };
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
  // A recount proof corrects the credited ledger BEFORE growth is allocated
  // against it, in both the transition and the incremental branch. The
  // snapshot then raises any cell the correction left below it, bounded as
  // ever by provable lifetime growth, so a corrected cell converges on the
  // truth from either side.
  const proof = RECOUNT_PROOF_CLIENTS.has(args.client)
    ? args.recountProof
    : undefined;
  const recount = proof
    ? partitionRecountProof(proof, args.incomingDays)
    : { accepted: [], rejected: [] };
  const correctLedger = (
    days: Record<string, ClientBreakdownData>
  ): Record<string, ClientBreakdownData> =>
    proof && recount.accepted.length > 0
      ? applyRecountProofToDays(days, proof, recount.accepted)
      : days;
  const recountFields = {
    ...(recount.accepted.length > 0 ? { recountDays: recount.accepted } : {}),
    ...(recount.rejected.length > 0
      ? { rejectedRecountDays: recount.rejected }
      : {}),
  };
  if (args.state == null || args.state.baselineEstablished === false) {
    const existingLegacyDays = correctLedger(args.existingLegacyDays);
    const legacyAggregate = aggregateSnapshot(existingLegacyDays);
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
    // A default all-client scan declares every default parser, including
    // clients that produced no rows. For a client whose generation only ever
    // heals through proof, an empty snapshot is not evidence that the parser
    // read anything, so keep the transition pending rather than baselining
    // the inflated legacy rows as this generation's ledger. With nothing
    // stored the same applies: a failed first read must not consume the
    // transition and freeze the generation-1 submits this device may still
    // make.
    if (
      RECOUNT_PROOF_CLIENTS.has(args.client) &&
      Object.keys(args.incomingDays).length === 0
    ) {
      return pendingTransitionPlan(args.incomingVersion);
    }
    // At transition, preserving all legacy rows and crediting at most positive
    // lifetime aggregate growth is non-destructive. With deleted old usage D
    // and genuinely new usage N, incoming - legacy = N - D <= N; the existing
    // rows remain untouched and the normal aggregate/cell caps allocate only
    // that provable remainder.
    const increments = hasLegacy
      ? allocateIncrements(
          existingLegacyDays,
          existingLegacyDays,
          args.incomingDays,
          legacyAggregate,
          incomingAggregate
        )
      : createSafeRecord<ClientBreakdownData>();
    const nextState = hasLegacy
      ? stateAfterCreditedIncrements(
          supportedVersion,
          existingLegacyDays,
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
      ...recountFields,
    };
  }
  // State written before allocation schema v2 stored every observed cell
  // maximum and advanced its aggregate even when no cell could accept that
  // growth. Rebuild that one-time migration baseline from the rows that were
  // actually credited, otherwise already-suppressed usage remains lost.
  const previousCreditedDays = correctLedger(
    args.state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION
      ? args.state.days
      : args.existingLegacyDays
  );
  // A v2 aggregate must describe exactly the credited cells (see validState),
  // so once a correction touched them it is re-derived rather than reused.
  const previousAggregate =
    args.state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION &&
    recount.accepted.length === 0
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

  const highWaterDeficit = positive(
    previousAggregate.tokens - incomingAggregate.tokens
  );
  return {
    mode: "incremental",
    increments,
    ...(highWaterDeficit > 0 ? { highWaterDeficit } : {}),
    nextState: stateAfterCreditedIncrements(
      supportedVersion,
      previousCreditedDays,
      increments,
      args.incomingDays
    ),
    ...recountFields,
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
