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
  mode: "status-quo" | "freeze" | "replace";
  parserVersions?: Record<string, number>;
  layouts?: FamilyLayouts;
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

  if (
    args.contributions.some(
      (day) =>
        day.clients.some((cell) =>
          ANTIGRAVITY_FAMILY.some((client) => cell.client === client)
        ) && day.totals?.costIsComplete === false
    )
  ) {
    return freeze("the Antigravity family snapshot has incomplete pricing");
  }

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
  if (
    !covers(previous.total, incoming.total) ||
    [...previous.models].some(([modelId, coverage]) =>
      !covers(coverage, incoming.models.get(modelId))
    )
  ) {
    return freeze("the full snapshot does not cover this device's credited Antigravity family usage");
  }

  return {
    mode: "replace",
    parserVersions,
    layouts,
    warning: "Reconciled Antigravity desktop, CLI, and IDE extension usage together from one family snapshot; source changes were transferred, not added again.",
  };
}
