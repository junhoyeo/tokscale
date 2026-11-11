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
 * Antigravity parsers are registered at generation 2. The database parser is
 * shared by the CLI and IDE extension, and reads a per-generation stamp out of
 * `gen_metadata`; the `antigravity` parser correlates standalone rows to
 * trajectory steps. All three source labels are admitted and reconciled as one
 * family because the same provider response can appear in multiple surfaces.
 * Generation 1 added the model enum to input tokens, swapped response and
 * thinking output and dropped cache writes, so generation 2 counts the same
 * history differently in absolute terms. `planAntigravityTransition` keeps
 * accepting generation 1 from a device until a generation-2 snapshot has
 * replaced that device's stored layout.
 *
 * Devin CLI and Cursor are registered at submission generation 2. Devin CLI
 * generation 1 counted one API request once per persisted database row; Cursor
 * generation 1 counted every running snapshot of one `usageEventId`. Their
 * generation-2 parsers collapse those duplicates, so the same history counts
 * strictly LOWER. Neither the merge guard nor the high-water can heal that
 * inflation because both only ever defend stored values. Submission generations
 * are independent of each client's local source-cache parser version. See
 * RECOUNT_PROOF_CLIENTS.
 */
export const SUPPORTED_VERSIONED_PARSERS: Readonly<Record<string, number>> = {
  copilot: 2,
  cursor: 2,
  droid: 1,
  "devin-cli": 2,
  // v2 moves exported Amp messages from the synthesized thread-created day to
  // their server-provided usage timestamp. This is a pure re-attribution, so
  // the receiver must replace the daily layout instead of applying its normal
  // non-decreasing per-day merge guard.
  amp: 2,
  // Membership and accepted generations for the atomic Antigravity family
  // transition; the submit route deliberately skips independent client plans.
  "antigravity-cli": 2,
  "antigravity-extension": 2,
  antigravity: 2,
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
 * lifetime (Devin CLI drops duplicate database rows; Cursor drops repeated
 * event snapshots).
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
 * this generation counts from them. At the device's transition to the new
 * generation, a stored cell drops to the corrected counts only when its token
 * and message counts are exactly what the previous generation counted from
 * those rows. Any other stored cell may hold usage the proof cannot describe
 * (a source deleted or no longer cached locally) and keeps its counts. After
 * the transition a proof never changes counts: the stored cells are then in
 * this generation's units, and matching counts no longer show which rows they
 * came from. Cost changes only on a complete snapshot whose proof cost is the
 * submitted cost; an incomplete transition keeps stored costs as floors, and
 * a later complete proof restates a floored cell's cost.
 *
 * The transition assumes every stored cell came from the previous generation.
 * A receiver that did not version the client yet may already hold cells the
 * new generation counted. Such a cell drops when its counts equal a proof's
 * previous side in all five token fields and messages: for example, sources A
 * and B each count 200 under the previous generation and 100 under the new
 * one, the old receiver stored their 200, B is gone at the upgrade, and A's
 * previous side is 200, so the day drops to 100.
 *
 * A partial scan never corrects anything: it freezes, or for
 * PER_DAY_GROWTH_CLIENTS credits growth on its complete days. An empty
 * snapshot keeps the transition pending rather than establishing a baseline
 * the parser never actually read.
 */
const RECOUNT_PROOF_CLIENTS: ReadonlySet<string> = new Set([
  "cursor",
  "devin-cli",
]);

/**
 * Recounting clients whose generations only drop rows the previous generation
 * counted and never move usage to another day. Cursor generation 2 keeps one
 * snapshot of each event, on a day where generation 1 already counted it.
 * Devin CLI generation 2 keeps the first persisted copy of each request,
 * dated by that row's own write time, as generation 1 dated it too.
 *
 * The lifetime budget below exists so that usage a generation re-dates cannot
 * be credited twice. Under it, history that leaves the local scan would stop
 * every later token from counting until the missing total was earned again:
 * `tokscale cursor logout` archives that account's cache and the scanner
 * skips archives, and Devin CLI deletes `message_nodes` rows outright. These
 * clients budget growth per day instead: a day's token and message totals may
 * rise to the snapshot's totals for that day and never above them, as they
 * could under the ordinary per-day merge before registration. Costs and token
 * buckets are attributed as for every versioned client, and only a recount
 * proof lowers a stored day.
 *
 * This relies on each record keeping its day across scans. Real Cursor
 * snapshots keep their event's timestamp (#1544). A Devin CLI request keeps
 * its day while its first copy exists, and in a real database no surviving
 * request had lost its first copy while a later compaction copy remained. A
 * record that moved to another day after the transition would be credited on
 * both days, as the ordinary per-day merge did.
 *
 * Because no day's budget reads another day, a date-bounded scan that read
 * each of its days in full (`scanScope.completeDayClients`) may credit them
 * once the baseline is set, so `tokscale submit --week` need not ship the
 * whole history.
 */
const PER_DAY_GROWTH_CLIENTS: ReadonlySet<string> = new Set([
  "cursor",
  "devin-cli",
]);

/**
 * Recounting clients whose duplicates all sit on their survivor's day, so a
 * recount proof moves a model's cost on that day's own pricing, whatever
 * other days hold (`recountCostUnsafeModels`). Every snapshot of one Cursor
 * event carries the event's timestamp. Devin CLI does not qualify: a
 * compaction copy of a request is written when the session is compacted,
 * often on a later day than the request.
 *
 * A model the proof shows with no survivor on some day contradicts that for
 * itself and still waits while any proof day is unpriced, as for every
 * client. An event whose snapshots moved only in part cannot be told apart;
 * if its survivor's day is unpriced, the moved snapshot's cost can be dropped
 * with it.
 */
const SAME_DAY_DUPLICATE_CLIENTS: ReadonlySet<string> = new Set(["cursor"]);

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

/**
 * Day -> models whose stored cells are still in a previous parser
 * generation's units: cells a recounting generation's transition kept
 * because its proof could not describe them, plus cells inherited from a
 * pre-`stateVersion` ledger. A matching recount proof may still correct
 * them; see `applyRecountProofToCell`.
 */
export type ResidueDays = Record<string, Record<string, true>>;

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
  residueDays?: ResidueDays;
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
   * behind `nextState` already has the proof applied on these days; the route
   * applies `applyRecountProofToCell` to the stored cells of the same days,
   * with the same `recountTransition`, before adding `increments`.
   */
  recountDays?: string[];
  /**
   * Set on the device's transition to the recounting generation, the one
   * submit whose proof may correct counts unconditionally. A later proof
   * corrects only cells still marked as previous-generation residue
   * (`residueDays`) and can otherwise only restate a floored cost.
   */
  recountTransition?: boolean;
  /**
   * Models whose cost no proof day may move on this submit
   * (`recountCostUnsafeModels`); the route passes them to
   * `applyRecountProofToCell` as the planner did.
   */
  recountCostUnsafeModels?: string[];
  /**
   * Stored models still in the previous generation's units, as consulted
   * during this submit's recount corrections. `nextState.residueDays`
   * persists what survives it; the route passes the same set to
   * `applyRecountProofToCell`.
   */
  residueDays?: ResidueDays;
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

export interface RecountCorrection {
  cell: ClientBreakdownData;
  /** What the correction removed from the stored cell (negative if it raised a cost). */
  removed: { tokens: number; cost: number; messages: number };
  /** Whether any count or cost of the stored cell changed. */
  changed: boolean;
  /**
   * Whether the proof now accounts for the cell's whole cost: see
   * `recountProofProvesCost`.
   */
  costProven: boolean;
  /**
   * Whether a model's counts were corrected while its cost was kept because
   * pricing could not vouch for the proof's cost. That cost is a floor: the
   * route tags the cell, and a later complete proof restates it.
   */
  costDeferred: boolean;
  /** Models whose counts the proof corrected, now in this generation's units. */
  recounted: string[];
}

/** What `applyRecountProofToCell` needs to know about the submit. */
export interface RecountProofContext {
  /**
   * The device's transition to the recounting generation: the one submit
   * whose proof may correct counts unconditionally on a `previous` match.
   */
  transition: boolean;
  /**
   * The day's models whose stored cells are still in the previous
   * generation's units (`residueDays` in the v2 parser state): cells the
   * transition could not prove and therefore kept. A `previous` match on a
   * residue model corrects it; a match on any other model is a coincidence —
   * a disappearing source can make `previous` land on a corrected cell's
   * counts exactly — and the cell is kept.
   */
  residueModels?: Record<string, true>;
  /** Whether this snapshot priced every token it counted on the day. */
  costIsComplete: boolean;
  /**
   * Whether the stored row's cost for the client is a floor
   * (`provenance.costIsComplete === false`). The planner reads it from the
   * stored row, because its own ledger does not keep the tag.
   */
  storedCostIsFloor: boolean;
  /**
   * Models named by the proof on a day the snapshot could not price
   * completely (all of them when the proof was rejected for some day). A
   * duplicate's survivor can lie on another day (a response persisted across
   * midnight), but always under the same model, so such a model's cost moves
   * on no day: its survivors' price is not known (`recountCostUnsafeModels`).
   */
  costUnsafeModels: ReadonlySet<string>;
}

/**
 * Apply one day's recount proof to a stored cell.
 *
 * Counts are corrected only at the device's transition, the one submit at
 * which every stored cell still holds what the previous generation counted.
 * There, per model, a stored cell whose token and message counts equal exactly
 * what the previous generation counts from the proof's rows drops to the
 * corrected counts. Any other cell keeps its counts. It holds usage the proof
 * cannot describe (a source deleted or no longer cached locally), or rows that
 * changed after its last submit (usually the last day submitted before the
 * upgrade). The proof's other side already passes the same test: its
 * corrected counts must equal the submitted cell
 * (`recountProofDayMatchesSnapshot`).
 *
 * After the transition a stored cell is normally in this generation's
 * units, and matching counts alone prove nothing: when one of two sources
 * disappears, the remaining source's previous count can equal the corrected
 * day exactly, and lowering the day would erase the other source. The state
 * records which cells the transition left unproven (`residueDays`); a later
 * proof may lower counts only on those residue cells. A residue cell that
 * takes growth keeps its mark — the proof counts the new rows on both
 * sides, so `stored == previous` still means the cell tracks the previous
 * generation's count — and a model drops out of the set only when a recount
 * lands, after which no later proof may touch its counts again.
 *
 * Cost changes only on a complete snapshot, and only where the proof's
 * corrected cost is the cost the snapshot submitted for the model and the
 * previous cost accounts for the stored cost (within one stored cost unit).
 * A duplicate's survivor can lie on another day, so a model's cost moves only
 * when every day the proof names that model on was priced completely
 * (`RecountProofContext.costUnsafeModels`).
 * An incomplete snapshot keeps every stored cost as a floor: the CLI builds
 * the proof before submission zeroes estimates it cannot price safely, and an
 * unpriced survivor can make the corrected cost far too low. On those terms a
 * model corrected at the transition drops to the corrected cost, never
 * raising it. A cell whose stored cost is a floor takes the corrected cost
 * whenever its counts are the corrected ones, at the transition or after it,
 * which raises an underpriced floor as well; that is how a cost floored by an
 * incomplete transition is corrected later.
 *
 * Models the proof does not name are untouched; proof models the cell does
 * not hold have nothing to correct.
 */
export function applyRecountProofToCell(
  stored: ClientBreakdownData,
  proofDay: Record<string, RecountProofEntry>,
  submitted: ClientBreakdownData | undefined,
  context: RecountProofContext
): RecountCorrection {
  const models = modelsForHighWater(stored);
  const submittedModels = submittedModelsOf(submitted);
  const removed = { tokens: 0, cost: 0, messages: 0 };
  const recounted: string[] = [];
  let changed = false;
  let costDeferred = false;
  for (const [modelId, entry] of Object.entries(proofDay)) {
    const model = ownValue(models, modelId);
    if (!model || !proofCellIsOrdered(entry)) continue;
    const recount =
      modelCountsEqual(model, entry.previous) &&
      (context.transition || ownValue(context.residueModels ?? {}, modelId) === true);
    const pricingVouches =
      context.costIsComplete &&
      !context.costUnsafeModels.has(modelId) &&
      proofCostWasSubmitted(entry, submittedModels, modelId);
    // A cost kept for want of complete pricing is a floor a later complete
    // proof restates. One kept because it exceeds what the previous
    // generation priced is spend the proof does not explain, and stays.
    costDeferred ||= recount && !pricingVouches;
    // Stored and proof costs are sums of the same per-message prices and can
    // differ by floating-point noise, so a stored cost up to one stored cost
    // unit above `previous` still counts as accounted for.
    const costAccountedFor =
      pricingVouches && model.cost <= entry.previous.cost + PROOF_COST_TOLERANCE;
    const restatesFloor =
      context.storedCostIsFloor &&
      costAccountedFor &&
      (recount || modelCountsEqual(model, entry.corrected));
    if (!recount && !restatesFloor) continue;
    const next = { ...model };
    if (recount) {
      recounted.push(modelId);
      const priorInclusiveInput = positive(
        model.inputIncludingCacheRead ?? model.input + model.cacheRead
      );
      for (const field of PROOF_TOKEN_FIELDS) {
        next[field] = entry.corrected.tokens[field];
        removed.tokens += model[field] - next[field];
      }
      next.inputIncludingCacheRead = positive(
        priorInclusiveInput -
          (model.input - next.input) -
          (model.cacheRead - next.cacheRead)
      );
      next.tokens = PROOF_TOKEN_FIELDS.reduce((sum, field) => sum + next[field], 0);
      next.messages = entry.corrected.messages;
      removed.messages += model.messages - next.messages;
    }
    if (restatesFloor) {
      next.cost = quantizeCost(positive(entry.corrected.cost));
    } else if (costAccountedFor) {
      // In the slack of the tolerance the cost drops to the corrected cost,
      // which can remove up to one unit more than subtracting the proven
      // over-count would.
      next.cost = quantizeCost(Math.min(model.cost, positive(entry.corrected.cost)));
    }
    removed.cost += model.cost - next.cost;
    changed ||=
      next.cost !== model.cost ||
      next.messages !== model.messages ||
      PROOF_TOKEN_FIELDS.some((field) => next[field] !== model[field]);
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
  return {
    cell,
    removed,
    changed,
    costProven: recountProofProvesCost(cell, proofDay, submitted),
    costDeferred,
    recounted,
  };
}

function modelCountsEqual(model: ModelBreakdownData, side: RecountProofCell): boolean {
  return (
    PROOF_TOKEN_FIELDS.every((field) => model[field] === side.tokens[field]) &&
    model.messages === side.messages
  );
}

function modelCountsMatch(a: ModelBreakdownData, b: ModelBreakdownData): boolean {
  return (
    PROOF_TOKEN_FIELDS.every((field) => a[field] === b[field]) &&
    a.messages === b.messages
  );
}

function submittedModelsOf(
  submitted: ClientBreakdownData | undefined
): Record<string, ModelBreakdownData> {
  return submitted ? modelsForHighWater(submitted) : createSafeRecord<ModelBreakdownData>();
}

/**
 * Whether a proof's corrected cost for one model is the cost the snapshot
 * submitted for it (within one stored cost unit). A model the snapshot did not
 * submit on the day (a day it does not date at all, or a model whose rows
 * there are all duplicates) submitted nothing.
 */
function proofCostWasSubmitted(
  entry: RecountProofEntry,
  submittedModels: Record<string, ModelBreakdownData>,
  modelId: string
): boolean {
  const submittedCost = ownValue(submittedModels, modelId)?.cost ?? 0;
  return Math.abs(submittedCost - entry.corrected.cost) <= PROOF_COST_TOLERANCE;
}

/**
 * Whether a recount proof accounts for a stored cell's whole cost: every model
 * of the cell is named by the proof, with exactly its corrected counts and its
 * corrected cost (within one stored cost unit), and that cost is what the
 * snapshot submitted (`submitted`).
 *
 * The route clears a floor tag only when this holds on a complete snapshot.
 * Equal counts alone would not be enough: another source's cell can hold the
 * same counts at a different cost.
 */
export function recountProofProvesCost(
  stored: ClientBreakdownData,
  proofDay: Record<string, RecountProofEntry>,
  submitted: ClientBreakdownData | undefined
): boolean {
  const models = Object.entries(modelsForHighWater(stored));
  const submittedModels = submittedModelsOf(submitted);
  return (
    models.length > 0 &&
    models.every(([modelId, model]) => {
      const entry = ownValue(proofDay, modelId);
      return (
        entry != null &&
        proofCellIsOrdered(entry) &&
        modelCountsEqual(model, entry.corrected) &&
        Math.abs(model.cost - entry.corrected.cost) <= PROOF_COST_TOLERANCE &&
        proofCostWasSubmitted(entry, submittedModels, modelId)
      );
    })
  );
}

/** The days of a submit that `applyRecountProofToDays` corrects, and how. */
interface RecountDaysContext {
  transition: boolean;
  /** The snapshot's own cells, which the proof's corrected side matched. */
  submittedDays: Record<string, ClientBreakdownData>;
  /** Days whose submitted cost the snapshot could not price completely. */
  costIncompleteDates: ReadonlySet<string>;
  /** The stored rows, whose floor tags decide `storedCostIsFloor`. */
  storedDays: Record<string, ClientBreakdownData>;
  /**
   * Stored models still in the previous generation's units; post-transition
   * recounts may correct only these.
   */
  residueDays?: Record<string, Record<string, true>>;
  costUnsafeModels: ReadonlySet<string>;
}

function applyRecountProofToDays(
  days: Record<string, ClientBreakdownData>,
  proof: ClientRecountProof,
  dates: readonly string[],
  context: RecountDaysContext
): { days: Record<string, ClientBreakdownData>; recounted: RecountedCells } {
  const next = normalizeStateDays(days);
  const recounted = createSafeRecord<Record<string, true>>();
  for (const date of dates) {
    const stored = ownValue(next, date);
    const proofDay = ownValue(proof, date);
    if (!stored || !proofDay) continue;
    const result = applyRecountProofToCell(
      stored,
      proofDay,
      ownValue(context.submittedDays, date),
      {
        transition: context.transition,
        residueModels: ownValue(context.residueDays ?? {}, date),
        costIsComplete: !context.costIncompleteDates.has(date),
        storedCostIsFloor:
          ownValue(context.storedDays, date)?.provenance?.costIsComplete === false,
        costUnsafeModels: context.costUnsafeModels,
      }
    );
    next[date] = result.cell;
    if (result.recounted.length > 0) {
      recounted[date] = Object.fromEntries(
        result.recounted.map((modelId) => [modelId, true as const])
      );
    }
  }
  return { days: next, recounted };
}

/** `recounted` is a `ResidueDays`-shaped set of cells a proof just corrected. */
type RecountedCells = ResidueDays;

/** Every model of every stored cell (a pre-`stateVersion` ledger is all residue). */
function allModelsResidue(days: Record<string, ClientBreakdownData>): ResidueDays {
  const residue = createSafeRecord<Record<string, true>>();
  for (const [date, cell] of Object.entries(days)) {
    residue[date] = Object.fromEntries(
      Object.keys(modelsForHighWater(cell)).map((modelId) => [modelId, true as const])
    );
  }
  return residue;
}

/**
 * Marks for a v2 state written before `residueDays` existed: a stored cell
 * equal to the last snapshot's cell was credited in this generation's units;
 * one that diverges was never corrected and is still previous-generation
 * residue. A cell the last snapshot does not date is kept unmarked rather
 * than risk deleting usage whose source simply went unread.
 */
function inferResidueDays(
  days: Record<string, ClientBreakdownData>,
  observedDays: Record<string, ClientBreakdownData> | undefined
): ResidueDays {
  const residue = createSafeRecord<Record<string, true>>();
  for (const [date, cell] of Object.entries(days)) {
    const observed = ownValue(observedDays ?? {}, date);
    if (!observed) continue;
    const observedModels = modelsForHighWater(observed);
    const marked = Object.fromEntries(
      Object.entries(modelsForHighWater(cell))
        .filter(([modelId, model]) => {
          const observedModel = ownValue(observedModels, modelId);
          return observedModel !== undefined && !modelCountsMatch(model, observedModel);
        })
        .map(([modelId]) => [modelId, true as const])
    );
    if (Object.keys(marked).length > 0) residue[date] = marked;
  }
  return residue;
}

/**
 * The cells a transition's proof could not prove: every stored model the
 * proof does not name, whose counts match neither `previous` nor
 * `corrected`. Those stay in the previous generation's units and remain
 * correctable by later proofs; proven cells never are again.
 */
function unprovenResidue(
  days: Record<string, ClientBreakdownData>,
  proof: ClientRecountProof | undefined,
  acceptedDays: ReadonlySet<string>
): ResidueDays {
  const residue = createSafeRecord<Record<string, true>>();
  for (const [date, cell] of Object.entries(days)) {
    const proofDay = ownValue(proof ?? {}, date);
    const marked = Object.fromEntries(
      Object.entries(modelsForHighWater(cell))
        .filter(([modelId, model]) => {
          const entry = ownValue(proofDay ?? {}, modelId);
          return !(
            acceptedDays.has(date) &&
            entry !== undefined &&
            proofCellIsOrdered(entry) &&
            (modelCountsEqual(model, entry.previous) ||
              modelCountsEqual(model, entry.corrected))
          );
        })
        .map(([modelId]) => [modelId, true as const])
    );
    if (Object.keys(marked).length > 0) residue[date] = marked;
  }
  return residue;
}

/** Remove the cells a recount just corrected from the residue set. */
function residueMinus(residue: ResidueDays, recounted: RecountedCells): ResidueDays {
  const next = createSafeRecord<Record<string, true>>();
  for (const [date, models] of Object.entries(residue)) {
    const kept = Object.fromEntries(
      Object.keys(models)
        .filter((modelId) => ownValue(recounted[date] ?? {}, modelId) !== true)
        .map((modelId) => [modelId, true as const])
    );
    if (Object.keys(kept).length > 0) next[date] = kept;
  }
  return next;
}

/**
 * Models whose cost no day's recount proof may move on this submit.
 *
 * - Every model the proof names once any of its days was rejected.
 * - Models the proof names on a day the snapshot could not price completely.
 *   A duplicate usually shares its survivor's model, and every day holding
 *   either is then a proof day for that model. A client whose duplicates
 *   share their survivor's day (`survivorsShareTheirDay`) skips this: the
 *   day's own pricing vouches for the cost it moves.
 * - While any proof day is unpriced, models the proof shows with no survivor
 *   on some day. Their duplicates' survivors lie on other days, possibly
 *   under another model id (a Devin CLI copy without its generation model, or
 *   a moved Cursor snapshot), so no day's pricing can vouch for them alone.
 *
 * A model with survivors whose duplicates also have survivors on another,
 * unpriced day under another model id cannot be told apart from the proof.
 */
export function recountCostUnsafeModels(
  proof: ClientRecountProof,
  recount: { accepted: readonly string[]; rejected: readonly string[] },
  costIncompleteDates: ReadonlySet<string>,
  survivorsShareTheirDay = false
): Set<string> {
  const unsafe = new Set<string>();
  if (recount.rejected.length > 0) {
    for (const proofDay of Object.values(proof)) {
      for (const modelId of Object.keys(proofDay)) unsafe.add(modelId);
    }
    return unsafe;
  }
  const unpricedDates = recount.accepted.filter((date) =>
    costIncompleteDates.has(date)
  );
  if (unpricedDates.length === 0) return unsafe;
  if (!survivorsShareTheirDay) {
    for (const date of unpricedDates) {
      for (const modelId of Object.keys(ownValue(proof, date) ?? {})) {
        unsafe.add(modelId);
      }
    }
  }
  for (const modelId of modelsWithoutSurvivors(proof, recount.accepted)) {
    unsafe.add(modelId);
  }
  return unsafe;
}

/** Models an accepted proof day lists with an empty corrected side. */
function modelsWithoutSurvivors(
  proof: ClientRecountProof,
  dates: readonly string[]
): Set<string> {
  const models = new Set<string>();
  for (const date of dates) {
    for (const [modelId, entry] of Object.entries(ownValue(proof, date) ?? {})) {
      const survives =
        entry.corrected.messages > 0 ||
        PROOF_TOKEN_FIELDS.some((field) => entry.corrected.tokens[field] > 0);
      if (!survives) models.add(modelId);
    }
  }
  return models;
}


function stateAfterCreditedIncrements(
  version: number,
  previousDays: Record<string, ClientBreakdownData>,
  increments: Record<string, ClientBreakdownData>,
  observedDays: Record<string, ClientBreakdownData>,
  residueDays?: ResidueDays
): ParserClientHighWaterState {
  const days = applyCreditedIncrements(previousDays, increments);
  return {
    stateVersion: PARSER_HIGH_WATER_STATE_VERSION,
    version,
    baselineEstablished: true,
    aggregate: aggregateSnapshot(days),
    days,
    observedDays: normalizeStateDays(observedDays),
    // Persist even when empty: an absent field marks a pre-residue state and
    // triggers inference on the next submit.
    ...(residueDays !== undefined ? { residueDays } : {}),
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

/** A day's token and message totals as the ledger and the route normalize it. */
function normalizedDayTotals(day: ClientBreakdownData | undefined): {
  tokens: number;
  messages: number;
} {
  let tokens = 0;
  let messages = 0;
  for (const model of Object.values(modelsForHighWater(day))) {
    tokens += model.tokens;
    messages += model.messages;
  }
  return { tokens, messages };
}

function allocateIncrements(
  client: string,
  previousCreditedDays: Record<string, ClientBreakdownData>,
  previousObservedDays: Record<string, ClientBreakdownData>,
  incomingDays: Record<string, ClientBreakdownData>,
  previousAggregate: ParserAggregateHighWater,
  incomingAggregate: ParserAggregateHighWater,
  growth: "lifetime" | "per-day" = "lifetime"
): Record<string, ClientBreakdownData> {
  const perDay = growth === "per-day";
  // Keep the complete credited lifetime in the budget. A client-reported
  // retention boundary cannot prove that a missing old cell was pruned rather
  // than re-dated into this snapshot, so excluding it could credit it twice.
  // PER_DAY_GROWTH_CLIENTS budget per day instead: their generations only drop
  // rows, and each event is assumed to keep its day (see the set's comment).
  let tokenBudget = perDay
    ? Number.POSITIVE_INFINITY
    : positive(incomingAggregate.tokens - previousAggregate.tokens);
  let messageBudget = perDay
    ? Number.POSITIVE_INFINITY
    : positive(incomingAggregate.messages - previousAggregate.messages);
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
  // Per-day growth: a day's token and message totals rise at most to the
  // snapshot's totals for it. Bounding the day, not each model, also keeps
  // usage that a later scan names under another model id from counting twice.
  // Totals come from the normalized models the route adds increments to, so a
  // legacy cell whose nested models exceed its scalar cannot be overfilled.
  const dayTokenBudgets = createSafeRecord<number>();
  const dayMessageBudgets = createSafeRecord<number>();
  for (const date of perDay ? dates : []) {
    const credited = normalizedDayTotals(ownValue(previousCreditedDays, date));
    const incoming = normalizedDayTotals(incomingDays[date]);
    dayTokenBudgets[date] = positive(incoming.tokens - credited.tokens);
    dayMessageBudgets[date] = positive(incoming.messages - credited.messages);
  }
  const dayBudget = (budgets: Record<string, number>, date: string) =>
    perDay ? ownValue(budgets, date) ?? 0 : Number.POSITIVE_INFINITY;
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
          respectBucketBudgets ? bucketBudgets[field] : Number.POSITIVE_INFINITY,
          dayBudget(dayTokenBudgets, cell.date)
        );
        cell.increment[field] += accepted;
        cell.increment.tokens += accepted;
        tokenBudget -= accepted;
        cellBudget -= accepted;
        if (respectBucketBudgets) bucketBudgets[field] -= accepted;
        if (perDay) dayTokenBudgets[cell.date] -= accepted;
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
        cellBudget,
        dayBudget(dayMessageBudgets, cell.date)
      );
      cell.increment.messages += accepted;
      messageBudget -= accepted;
      if (perDay) dayMessageBudgets[cell.date] -= accepted;
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
    let acceptedFraction =
      cell.increment.tokens > 0 && cell.tokenCapacity > 0
        ? cell.increment.tokens / cell.tokenCapacity
        : 0;
    // Cursor's legacy premium-tool-call rows are billed per invocation, not
    // per token, and are explicitly admitted by submission validation. Their
    // message growth authorizes the same fraction of marginal cost that token
    // growth does for normal rows. Keep this narrow: repricing with no new
    // messages, another tokenless model, or a formerly token-bearing cell
    // must not turn into newly credited spend.
    if (
      client === "cursor" &&
      cell.modelId === "premium-tool-call" &&
      cell.model.tokens === 0 &&
      (cell.credited?.tokens ?? 0) === 0 &&
      cell.messageCapacity > 0
    ) {
      acceptedFraction = cell.increment.messages / cell.messageCapacity;
    }
    if (acceptedFraction > 0) {
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
 * PER_DAY_GROWTH_CLIENTS bound that growth per day instead of by the lifetime
 * high-water, because their generations only drop rows and each event is
 * assumed to keep its day.
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
   * The snapshot reads every day it dates in full and omits the rest: a
   * date-bounded scan (`scanScope.completeDayClients`). Honored only for
   * PER_DAY_GROWTH_CLIENTS whose baseline a full snapshot already set.
   */
  completeDays?: boolean;
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
  /**
   * Days whose submitted cost the snapshot could not price completely
   * (`totals.costIsComplete === false`). A recount proof changes no cost on
   * them.
   */
  costIncompleteDates?: ReadonlySet<string>;
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
  // Amp, Devin CLI, and Cursor v1 are already released when their receivers
  // deploy. Keep accepting them for devices that have not transitioned yet,
  // but never let an older CLI rewrite a device after the receiver has started
  // the new generation. This preserves the server-before-CLI rollout without
  // reopening a completed (or pending) transition to the old parser.
  if (
    (args.client === "amp" ||
      args.client === "cursor" ||
      args.client === "devin-cli") &&
    args.incomingVersion === 1
  ) {
    return args.state
      ? { mode: "freeze", increments: {} }
      : { mode: "status-quo", increments: {} };
  }
  if (args.incomingVersion !== supportedVersion) {
    return { mode: "freeze", increments: {} };
  }
  // Per-day growth reads only the days a snapshot dates, so a date-bounded
  // scan credits them exactly as a full snapshot whose other days did not
  // change. It cannot set the baseline or apply a recount proof, and the days
  // it does not date keep their credited and observed cells.
  const dayScoped =
    !args.fullHistory &&
    args.completeDays === true &&
    PER_DAY_GROWTH_CLIENTS.has(args.client) &&
    args.state != null &&
    args.state.baselineEstablished !== false;
  if (!args.fullHistory && !dayScoped) {
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
  // A recount proof is applied to the credited ledger BEFORE growth is
  // allocated against it. At the transition it corrects counts; the snapshot
  // then raises any cell the correction left below it, bounded as ever by
  // provable growth, so a corrected cell converges on the truth from either
  // side. After the transition it can only restate a floored cost (see
  // `applyRecountProofToCell`).
  const growth = PER_DAY_GROWTH_CLIENTS.has(args.client) ? "per-day" : "lifetime";
  const proof =
    RECOUNT_PROOF_CLIENTS.has(args.client) && !dayScoped
      ? args.recountProof
      : undefined;
  const recount = proof
    ? partitionRecountProof(proof, args.incomingDays)
    : { accepted: [], rejected: [] };
  const costIncompleteDates = args.costIncompleteDates ?? new Set<string>();
  const costUnsafeModels = proof
    ? recountCostUnsafeModels(
        proof,
        recount,
        costIncompleteDates,
        SAME_DAY_DUPLICATE_CLIENTS.has(args.client)
      )
    : new Set<string>();
  const correctLedger = (
    days: Record<string, ClientBreakdownData>,
    transition: boolean,
    residueDays?: ResidueDays
  ): { days: Record<string, ClientBreakdownData>; recounted: RecountedCells } =>
    proof && recount.accepted.length > 0
      ? applyRecountProofToDays(days, proof, recount.accepted, {
          transition,
          submittedDays: args.incomingDays,
          costIncompleteDates,
          storedDays: args.existingLegacyDays,
          residueDays,
          costUnsafeModels,
        })
      : { days, recounted: createSafeRecord<Record<string, true>>() };
  const recountFields = {
    ...(recount.accepted.length > 0 ? { recountDays: recount.accepted } : {}),
    ...(recount.rejected.length > 0
      ? { rejectedRecountDays: recount.rejected }
      : {}),
    ...(recount.accepted.length > 0 && costUnsafeModels.size > 0
      ? { recountCostUnsafeModels: [...costUnsafeModels].sort() }
      : {}),
  };
  if (args.state == null || args.state.baselineEstablished === false) {
    // Every stored cell still holds what the previous generation counted, so
    // this is the one submit whose proof may correct counts.
    const { days: existingLegacyDays } = correctLedger(
      args.existingLegacyDays,
      true
    );
    const residueDays = RECOUNT_PROOF_CLIENTS.has(args.client)
      ? unprovenResidue(args.existingLegacyDays, proof, new Set(recount.accepted))
      : undefined;
    const legacyAggregate = aggregateSnapshot(existingLegacyDays);
    // Whether the device stored anything is judged before the proof: a stored
    // row the proof empties entirely still has to be corrected by the route,
    // which only a legacy baseline does.
    const storedAggregate = aggregateSnapshot(args.existingLegacyDays);
    const hasLegacy =
      storedAggregate.tokens > 0 || storedAggregate.messages > 0;
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
          args.client,
          existingLegacyDays,
          existingLegacyDays,
          args.incomingDays,
          legacyAggregate,
          incomingAggregate,
          growth
        )
      : createSafeRecord<ClientBreakdownData>();
    const nextState = hasLegacy
      ? stateAfterCreditedIncrements(
          supportedVersion,
          existingLegacyDays,
          increments,
          args.incomingDays,
          residueDays
        )
      : {
          stateVersion: PARSER_HIGH_WATER_STATE_VERSION,
          version: args.incomingVersion,
          baselineEstablished: true,
          aggregate: incomingAggregate,
          days: normalizeStateDays(args.incomingDays),
          observedDays: normalizeStateDays(args.incomingDays),
          ...(residueDays !== undefined ? { residueDays } : {}),
        };
    return {
      mode: hasLegacy ? "baseline-legacy" : "baseline-new",
      increments,
      nextState,
      ...recountFields,
      ...(recount.accepted.length > 0 ? { recountTransition: true } : {}),
    };
  }
  // State written before allocation schema v2 stored every observed cell
  // maximum and advanced its aggregate even when no cell could accept that
  // growth. Rebuild that one-time migration baseline from the rows that were
  // actually credited, otherwise already-suppressed usage remains lost.
  // Past the transition a proof may still correct the cells the transition
  // left unproven (`residueDays`); every other stored cell is in this
  // generation's units, where a `previous` match proves nothing (see
  // `applyRecountProofToCell`). A pre-v2 state predates recounts entirely —
  // every credited cell is residue. A v2 state written before the set was
  // tracked infers it once from the cells its last snapshot disagrees with.
  const residueDays = !RECOUNT_PROOF_CLIENTS.has(args.client)
    ? undefined
    : args.state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION
      ? args.state.residueDays ??
        inferResidueDays(args.state.days, args.state.observedDays)
      : allModelsResidue(args.existingLegacyDays);
  const corrected = correctLedger(
    args.state.stateVersion === PARSER_HIGH_WATER_STATE_VERSION
      ? args.state.days
      : args.existingLegacyDays,
    false,
    residueDays
  );
  const previousCreditedDays = corrected.days;
  const remainingResidue = residueDays
    ? residueMinus(residueDays, corrected.recounted)
    : undefined;
  // The corrected ledger, not the persisted aggregate: a residue correction
  // lowers it, and the deficit and growth budget must see the same baseline
  // `nextState` recomputes.
  const previousAggregate = aggregateSnapshot(previousCreditedDays);
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
  // Budget a date-bounded scan against the observed cells of its own days, so
  // it allocates what a full snapshot with the same other days would.
  const windowObservedDays = createSafeRecord<ClientBreakdownData>();
  for (const date of dayScoped ? Object.keys(args.incomingDays) : []) {
    const observed = ownValue(previousObservedDays, date);
    if (observed) windowObservedDays[date] = observed;
  }
  const increments = allocateIncrements(
    args.client,
    previousCreditedDays,
    dayScoped ? windowObservedDays : previousObservedDays,
    args.incomingDays,
    previousAggregate,
    incomingAggregate,
    growth
  );

  // Only a lifetime budget withholds growth while the scan sits below the
  // credited baseline; per-day growth has no such deficit to report.
  const highWaterDeficit =
    growth === "lifetime"
      ? positive(previousAggregate.tokens - incomingAggregate.tokens)
      : 0;
  return {
    mode: "incremental",
    increments,
    ...(highWaterDeficit > 0 ? { highWaterDeficit } : {}),
    nextState: stateAfterCreditedIncrements(
      supportedVersion,
      previousCreditedDays,
      increments,
      dayScoped
        ? { ...previousObservedDays, ...args.incomingDays }
        : args.incomingDays,
      remainingResidue
    ),
    // The residue set in effect for this submit's corrections, so the route
    // applies `applyRecountProofToCell` to the stored cells the same way.
    ...(residueDays !== undefined &&
    recount.accepted.length > 0 &&
    Object.keys(residueDays).length > 0
      ? { residueDays }
      : {}),
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
