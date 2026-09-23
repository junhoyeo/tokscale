import type { ClientBreakdownData } from "./helpers";
import {
  modelsForHighWater,
  PARSER_HIGH_WATER_STATE_VERSION,
  type DeviceParserStates,
} from "./parserHighWater";
import { ownValue } from "../safeRecord";

export const ANTIGRAVITY_FAMILY = [
  "antigravity",
  "antigravity-cli",
  "antigravity-extension",
] as const;

const COVERAGE_FIELDS = [
  "tokens",
  "input",
  "output",
  "cacheRead",
  "cacheWrite",
  "reasoning",
  "messages",
] as const;

type Coverage = Record<(typeof COVERAGE_FIELDS)[number], number>;
type CoverageSummary = { total: Coverage; models: Map<string, Coverage> };
type PriorDay = { sourceBreakdown: unknown };

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

function coverage(cells: Array<ClientBreakdownData | undefined>): CoverageSummary {
  const total = emptyCoverage();
  const models = new Map<string, Coverage>();
  for (const cell of cells) {
    if (!cell) continue;
    addCoverage(total, cell);
    for (const [modelId, model] of Object.entries(modelsForHighWater(cell))) {
      const modelCoverage = models.get(modelId) ?? emptyCoverage();
      addCoverage(modelCoverage, model);
      models.set(modelId, modelCoverage);
    }
  }
  return { total, models };
}

function maximum(left: CoverageSummary, right: CoverageSummary): CoverageSummary {
  const summary: CoverageSummary = { total: emptyCoverage(), models: new Map() };
  for (const field of COVERAGE_FIELDS) {
    summary.total[field] = Math.max(left.total[field], right.total[field]);
  }
  for (const modelId of new Set([...left.models.keys(), ...right.models.keys()])) {
    const previous = left.models.get(modelId);
    const parserState = right.models.get(modelId);
    const modelCoverage = emptyCoverage();
    for (const field of COVERAGE_FIELDS) {
      modelCoverage[field] = Math.max(
        previous?.[field] ?? 0,
        parserState?.[field] ?? 0,
      );
    }
    summary.models.set(modelId, modelCoverage);
  }
  return summary;
}

/** Combine the stored family ledger and legacy per-client parser ledgers. */
export function antigravityPriorCoverage(
  existingDays: PriorDay[],
  parserStates?: DeviceParserStates,
): { total: Coverage; models: Map<string, Coverage>; unverifiable: boolean } {
  const existingBreakdowns = existingDays.map((day) =>
    (day.sourceBreakdown ?? {}) as Record<string, ClientBreakdownData>,
  );
  const stored = coverage(
    existingBreakdowns.flatMap((breakdown) =>
      ANTIGRAVITY_FAMILY.map((client) => ownValue(breakdown, client)),
    ),
  );
  // Legacy per-client parser ledgers are interchangeable evidence of the same
  // credited lifetime: devices that submitted the same response from two
  // family surfaces pre-transition hold that response in BOTH ledgers, so
  // summing the ledgers would demand 2x coverage and freeze the family
  // channel forever (or double-credit on replace). Take the per-field max
  // across them instead -- the same reconciliation maximum() applies to
  // stored-vs-ledger.
  let parserLedger: CoverageSummary | null = null;
  let unverifiable = false;

  for (const client of ANTIGRAVITY_FAMILY) {
    const state = ownValue(parserStates, client);
    if (!state) continue;
    if (
      state.stateVersion != null &&
      state.stateVersion !== PARSER_HIGH_WATER_STATE_VERSION
    ) {
      unverifiable = true;
    }
    if (!state.days || !state.aggregate) {
      unverifiable = true;
      continue;
    }

    const clientLedger = coverage(Object.values(state.days));
    for (const field of COVERAGE_FIELDS) {
      const aggregate = state.aggregate[field];
      if (!Number.isSafeInteger(aggregate) || aggregate < 0) {
        unverifiable = true;
        continue;
      }
      clientLedger.total[field] = Math.max(clientLedger.total[field], aggregate);
    }
    parserLedger = parserLedger
      ? maximum(parserLedger, clientLedger)
      : clientLedger;
  }

  const combined = maximum(
    stored,
    parserLedger ?? { total: emptyCoverage(), models: new Map() },
  );
  return { ...combined, unverifiable };
}
