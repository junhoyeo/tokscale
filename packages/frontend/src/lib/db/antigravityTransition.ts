import type { ClientBreakdownData } from "./helpers";
import {
  ANTIGRAVITY_FAMILY,
  antigravityPriorCoverage,
  validateAntigravityRecountProof,
  type AntigravityRecountProofs,
} from "./antigravityStateCoverage";
import {
  foldParserClientSnapshot,
  SUPPORTED_VERSIONED_PARSERS,
  type ClientRecountProof,
  type DeviceParserStates,
  type IncomingParserContribution,
} from "./parserHighWater";
import { ownValue } from "../safeRecord";

export { ANTIGRAVITY_FAMILY } from "./antigravityStateCoverage";

type AntigravityClient = (typeof ANTIGRAVITY_FAMILY)[number];
type FamilyLayouts = Record<
  AntigravityClient,
  Record<string, ClientBreakdownData>
>;
type Coverage = Record<
  "tokens" | "input" | "output" | "cacheRead" | "cacheWrite" | "reasoning" | "messages",
  number
>;
type StoredDay = { date: string; sourceBreakdown: unknown };

const COVERAGE_FIELDS = [
  "tokens",
  "input",
  "output",
  "cacheRead",
  "cacheWrite",
  "reasoning",
  "messages",
] as const;

/**
 * The generation the family declared before `SUPPORTED_VERSIONED_PARSERS`
 * registered the corrected usage-field mapping. Every CLI released before
 * then declares it, and the receiver deploys first, so it stays accepted from
 * a device until a snapshot of the registered generation has replaced that
 * device's stored layout.
 */
const PREVIOUS_GENERATION = 1;

export interface AntigravityTransitionPlan {
  mode: "status-quo" | "freeze" | "replace";
  /**
   * The generation of the snapshot that last replaced this device's family
   * layout, so it also names the units of the stored cells. A freeze never
   * advances it.
   */
  parserVersions?: Record<string, number>;
  layouts?: FamilyLayouts;
  recountCostReduction?: number;
  warning?: string;
}

function emptyCoverage(): Coverage {
  return {
    tokens: 0,
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages: 0,
  };
}

function addCoverage(target: Coverage, source: Partial<Coverage>): void {
  for (const field of COVERAGE_FIELDS) target[field] += source[field] ?? 0;
}

function familyCoverage(cells: Array<ClientBreakdownData | undefined>) {
  const total = emptyCoverage();
  for (const cell of cells) {
    if (!cell) continue;
    addCoverage(total, cell);
  }
  return { total };
}

function covers(previous: Coverage, incoming?: Coverage): boolean {
  return COVERAGE_FIELDS.every((field) => {
    const before = previous[field];
    const after = incoming?.[field] ?? 0;
    return Number.isSafeInteger(before) && Number.isSafeInteger(after)
      && before >= 0 && after >= before;
  });
}

/**
 * Antigravity's desktop cache, CLI, and IDE extension can contain the same
 * provider response. Their source labels are presentation surfaces, not
 * independent lifetime ledgers. Admit or replace all three together from one
 * complete family snapshot so a response moving between clients cannot pass
 * three separate high-water checks and be credited more than once.
 */
export function planAntigravityTransition(args: {
  submittedClients: ReadonlySet<string>;
  incomingVersions?: Record<string, number>;
  persistedVersions?: Record<string, number>;
  parserStates?: DeviceParserStates;
  recountProofs?: Record<string, ClientRecountProof>;
  fullHistory: boolean;
  isBackfill: boolean;
  contributions: Array<IncomingParserContribution & { totals?: { costIsComplete?: boolean } }>;
  existingDays: StoredDay[];
}): AntigravityTransitionPlan {
  const touchesFamily = ANTIGRAVITY_FAMILY.some((client) =>
    args.submittedClients.has(client) || ownValue(args.incomingVersions, client) !== undefined
  );
  if (!touchesFamily) return { mode: "status-quo" };

  const unknownStoredGeneration = ANTIGRAVITY_FAMILY.some((client) =>
    (ownValue(args.persistedVersions, client) ?? 0) > SUPPORTED_VERSIONED_PARSERS[client]
  );
  // Every member, because a replace pins all three at once. A single pinned
  // member is left over from a submit an earlier receiver froze as a future
  // generation: it changed no cell, so the layout is still the previous
  // generation's.
  const transitioned = ANTIGRAVITY_FAMILY.every((client) =>
    ownValue(args.persistedVersions, client) === SUPPORTED_VERSIONED_PARSERS[client]
  );
  const declaresRegistered = ANTIGRAVITY_FAMILY.every((client) =>
    ownValue(args.incomingVersions, client) === SUPPORTED_VERSIONED_PARSERS[client]
  );
  const declaresPrevious = ANTIGRAVITY_FAMILY.every((client) =>
    ownValue(args.incomingVersions, client) === PREVIOUS_GENERATION
  );
  const unsupportedIncoming = ANTIGRAVITY_FAMILY.some((client) => {
    const incoming = ownValue(args.incomingVersions, client);
    return incoming !== undefined
      && incoming !== SUPPORTED_VERSIONED_PARSERS[client]
      && incoming !== PREVIOUS_GENERATION;
  });
  const incomingFutureVersions = Object.fromEntries(
    ANTIGRAVITY_FAMILY.flatMap((client) => {
      const incoming = ownValue(args.incomingVersions, client);
      return incoming !== undefined && incoming > SUPPORTED_VERSIONED_PARSERS[client]
        ? [[client, incoming]]
        : [];
    }),
  );
  const generationPins = (registered: boolean) =>
    Object.fromEntries(
      ANTIGRAVITY_FAMILY.map((client) => [
        client,
        registered ? SUPPORTED_VERSIONED_PARSERS[client] : PREVIOUS_GENERATION,
      ]),
    );
  const parserVersions = unknownStoredGeneration
    ? undefined
    : unsupportedIncoming
      ? Object.keys(incomingFutureVersions).length > 0
        ? incomingFutureVersions
        : undefined
      : generationPins(transitioned);
  const freeze = (
    reason: string,
    remedy = "Submit antigravity, antigravity-cli, and antigravity-extension together with a full-history scan to update them.",
  ): AntigravityTransitionPlan => ({
    mode: "freeze",
    parserVersions,
    warning: `Preserved Antigravity sources together: ${reason}. No Antigravity token or cost changes were applied. ${remedy}`,
  });

  if (declaresPrevious && transitioned && !unknownStoredGeneration) {
    // The stored layout is in the registered generation's units. This CLI
    // counts the same history differently, so its snapshot must not replace
    // the layout or undo a correction. Checked first: no rescan with this
    // CLI can help, so the scan-scope remedy below would mislead.
    return freeze(
      "this device already submitted Antigravity usage with a newer Tokscale CLI, which counts it differently",
      "Update the Tokscale CLI on this device to keep submitting Antigravity usage.",
    );
  }
  if (
    unknownStoredGeneration ||
    unsupportedIncoming ||
    args.isBackfill ||
    !args.fullHistory ||
    !(declaresRegistered || declaresPrevious)
  ) {
    return freeze("all three sources must declare the supported generation in a full-history scan");
  }

  // Incomplete pricing no longer vetoes the family's tokens: a permanently
  // unpriced historic model would otherwise freeze every later full-history
  // submit, silently dropping all growth. Tokens are reconciled below and the
  // route floors the family's lifetime cost (see
  // `reapplyReplaceFamilyCostFloor`), so spend survives source moves while the
  // cells stay tagged incomplete until a fully priced snapshot replaces them.

  const layouts = Object.fromEntries(
    ANTIGRAVITY_FAMILY.map((client) => [
      client,
      foldParserClientSnapshot(args.contributions, client),
    ])
  ) as FamilyLayouts;

  // A recount proof corrects stored cells at the one submit that moves the
  // device to the registered generation, while every stored cell still holds
  // what the previous generation counted. Afterwards the stored cells are in
  // the registered generation's units, and equal counts no longer show which
  // rows they came from, so a proof must not lower them.
  const recountProofs =
    declaresRegistered && !transitioned ? args.recountProofs : undefined;
  const validatedProofs: AntigravityRecountProofs = {};
  for (const client of ANTIGRAVITY_FAMILY) {
    const validated = validateAntigravityRecountProof(
      client,
      ownValue(recountProofs, client),
      layouts[client],
    );
    if (validated) {
      validatedProofs[client] = validated;
    }
  }

  const previous = antigravityPriorCoverage(
    args.existingDays,
    args.parserStates,
    validatedProofs,
  );
  if (previous.unverifiable) {
    return freeze("a prior Antigravity parser high-water state cannot be verified");
  }
  // Date and model-ID re-attributions are expected across parser improvements
  // (per-generation timestamps, tier model resolution from gemini-default /
  // gemini-3.5-flash, SessionModels backfills for unknown turns, and Claude
  // thinking-variant canonicalization). Compare the seven-field family
  // lifetime coverage across dates and models, matching SNAPSHOT_LAYOUT_CLIENTS
  // in parserHighWater, then replace all source layouts atomically.
  const incoming = familyCoverage(
    ANTIGRAVITY_FAMILY.flatMap((client) => Object.values(layouts[client]))
  );
  if (!covers(previous.total, incoming.total)) {
    return freeze("the full snapshot does not cover this device's credited Antigravity family usage");
  }

  const pricingIncomplete = args.contributions.some(
    (day) =>
      day.clients.some((cell) =>
        ANTIGRAVITY_FAMILY.some((client) => cell.client === client)
      ) && day.totals?.costIsComplete === false
  );

  return {
    mode: "replace",
    parserVersions: generationPins(declaresRegistered),
    layouts,
    ...(previous.recountCostReduction > 0
      ? { recountCostReduction: previous.recountCostReduction }
      : {}),
    warning: pricingIncomplete
      ? "Reconciled Antigravity desktop, CLI, and IDE extension usage together from one family snapshot; source changes were transferred, not added again. Some models could not be priced: tokens are credited, costs stay partial and floored at the credited lifetime until a fully priced snapshot replaces them."
      : "Reconciled Antigravity desktop, CLI, and IDE extension usage together from one family snapshot; source changes were transferred, not added again.",
  };
}
