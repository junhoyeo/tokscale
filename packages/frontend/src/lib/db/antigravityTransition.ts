import type { ClientBreakdownData } from "./helpers";
import {
  ANTIGRAVITY_FAMILY,
  antigravityPriorCoverage,
} from "./antigravityStateCoverage";
import {
  foldParserClientSnapshot,
  modelsForHighWater,
  SUPPORTED_VERSIONED_PARSERS,
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

export interface AntigravityTransitionPlan {
  mode: "status-quo" | "freeze" | "replace" | "incremental";
  parserVersions?: Record<string, number>;
  layouts?: FamilyLayouts;
  increments?: Record<AntigravityClient, Record<string, ClientBreakdownData>>;
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
  const models = new Map<string, Coverage>();
  for (const cell of cells) {
    if (!cell) continue;
    addCoverage(total, cell);
    for (const [modelId, model] of Object.entries(modelsForHighWater(cell))) {
      const coverage = models.get(modelId) ?? emptyCoverage();
      addCoverage(coverage, model);
      models.set(modelId, coverage);
    }
  }
  return { total, models };
}

function covers(previous: Coverage, incoming?: Coverage): boolean {
  return COVERAGE_FIELDS.every((field) => {
    const before = previous[field];
    const after = incoming?.[field] ?? 0;
    return Number.isSafeInteger(before) && Number.isSafeInteger(after)
      && before >= 0 && after >= before;
  });
}

function isAdjacentDay(dateA: string, dateB: string): boolean {
  const [y1, m1, d1] = dateA.split("-").map(Number);
  const [y2, m2, d2] = dateB.split("-").map(Number);
  const t1 = Date.UTC(y1, m1 - 1, d1);
  const t2 = Date.UTC(y2, m2 - 1, d2);
  const diffDays = Math.round(Math.abs(t2 - t1) / (24 * 60 * 60 * 1000));
  return diffDays <= 1;
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
  const unsupportedIncoming = ANTIGRAVITY_FAMILY.some((client) => {
    const incoming = ownValue(args.incomingVersions, client);
    return incoming !== undefined && incoming !== SUPPORTED_VERSIONED_PARSERS[client];
  });
  const incomingFutureVersions = Object.fromEntries(
    ANTIGRAVITY_FAMILY.flatMap((client) => {
      const incoming = ownValue(args.incomingVersions, client);
      return incoming !== undefined && incoming > SUPPORTED_VERSIONED_PARSERS[client]
        ? [[client, incoming]]
        : [];
    }),
  );
  const parserVersions = unknownStoredGeneration
    ? undefined
    : unsupportedIncoming
      ? Object.keys(incomingFutureVersions).length > 0
        ? incomingFutureVersions
        : undefined
      : Object.fromEntries(
          ANTIGRAVITY_FAMILY.map((client) => [
            client,
            SUPPORTED_VERSIONED_PARSERS[client],
          ]),
        );
  const freeze = (reason: string): AntigravityTransitionPlan => ({
    mode: "freeze",
    parserVersions,
    warning: `Preserved Antigravity sources together: ${reason}. No Antigravity token or cost changes were applied. Submit antigravity, antigravity-cli, and antigravity-extension together with a full-history scan to update them.`,
  });

  if (
    unknownStoredGeneration ||
    unsupportedIncoming ||
    args.isBackfill ||
    !args.fullHistory ||
    !ANTIGRAVITY_FAMILY.every((client) =>
      ownValue(args.incomingVersions, client) === SUPPORTED_VERSIONED_PARSERS[client]
    )
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

  const previous = antigravityPriorCoverage(args.existingDays, args.parserStates);
  if (previous.unverifiable) {
    return freeze("a prior Antigravity parser high-water state cannot be verified");
  }
  // Date changes are expected as providers expose per-generation timestamps.
  // Compare family lifetime/model coverage across dates, then replace all
  // source layouts atomically. This retains the credited total while allowing
  // the latest source label and day layout to follow the parser.
  const incoming = familyCoverage(
    ANTIGRAVITY_FAMILY.flatMap((client) => Object.values(layouts[client]))
  );
  const hasCoverageDeficit =
    !covers(previous.total, incoming.total) ||
    [...previous.models].some(([modelId, coverage]) =>
      !covers(coverage, incoming.models.get(modelId))
    );

  if (hasCoverageDeficit) {
    const hasLegacyUnmigratedState = ANTIGRAVITY_FAMILY.some(
      (client) => ownValue(args.parserStates, client) !== undefined
    );

    if (!hasLegacyUnmigratedState) {
      const existingFamilyDates = new Set(
        args.existingDays
          .filter((day) => {
            const breakdown = (day.sourceBreakdown ?? {}) as Record<
              string,
              ClientBreakdownData
            >;
            return ANTIGRAVITY_FAMILY.some(
              (client) => ownValue(breakdown, client) !== undefined
            );
          })
          .map((day) => day.date)
      );

      const lastCreditedDate =
        existingFamilyDates.size > 0
          ? [...existingFamilyDates].reduce((max, d) => (d > max ? d : max))
          : undefined;

      const incomingFamilyDates = new Set<string>();
      for (const client of ANTIGRAVITY_FAMILY) {
        for (const date of Object.keys(layouts[client])) {
          incomingFamilyDates.add(date);
        }
      }

      // Server-verifiable continuity:
      // 1. The device must already have completed parser generation migration (persistedVersions >= 1),
      //    guaranteeing that per-turn event dating is already established and no generations can be re-dated
      //    from older dates to newer dates across versions.
      // 2. The incoming snapshot's retained history must anchor to credited history:
      //    either overlapping on or before lastCreditedDate, or directly adjacent to the stored tail,
      //    proving historical continuity without depending on a single boundary day surviving rolling retention.
      // 3. Activity extends strictly beyond lastCreditedDate into genuinely new dates.
      const isAlreadyMigrated = ANTIGRAVITY_FAMILY.every((client) => {
        const persisted = ownValue(args.persistedVersions ?? {}, client);
        return persisted !== undefined && persisted >= 1;
      });

      const minIncomingDate =
        incomingFamilyDates.size > 0
          ? [...incomingFamilyDates].reduce((min, d) => (d < min ? d : min))
          : undefined;

      const anchorsOnRetainedHistory =
        lastCreditedDate !== undefined &&
        minIncomingDate !== undefined &&
        (minIncomingDate <= lastCreditedDate ||
          isAdjacentDay(lastCreditedDate, minIncomingDate));

      const newDates = new Set<string>();
      if (lastCreditedDate && anchorsOnRetainedHistory && isAlreadyMigrated) {
        for (const date of incomingFamilyDates) {
          if (date > lastCreditedDate) {
            newDates.add(date);
          }
        }
      }

      if (newDates.size > 0 && lastCreditedDate) {
        const increments = Object.fromEntries(
          ANTIGRAVITY_FAMILY.map((client) => [
            client,
            Object.fromEntries(
              Object.entries(layouts[client]).filter(([date]) => newDates.has(date))
            ),
          ])
        ) as Record<AntigravityClient, Record<string, ClientBreakdownData>>;

        const tokenDeficit = Math.max(0, previous.total.tokens - incoming.total.tokens);
        const deficitMsg = tokenDeficit > 0 ? ` (${tokenDeficit.toLocaleString()} token shortfall)` : "";

        return {
          mode: "incremental",
          parserVersions,
          increments,
          layouts,
          warning: `Preserved Antigravity sources prior to ${lastCreditedDate}${deficitMsg} because older local history was pruned under the credited high-water. Genuinely new activity after ${lastCreditedDate} was credited.`,
        };
      }
    }

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
    parserVersions,
    layouts,
    warning: pricingIncomplete
      ? "Reconciled Antigravity desktop, CLI, and IDE extension usage together from one family snapshot; source changes were transferred, not added again. Some models could not be priced: tokens are credited, costs stay partial and floored at the credited lifetime until a fully priced snapshot replaces them."
      : "Reconciled Antigravity desktop, CLI, and IDE extension usage together from one family snapshot; source changes were transferred, not added again.",
  };
}
