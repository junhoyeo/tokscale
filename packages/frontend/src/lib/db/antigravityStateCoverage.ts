import type { ClientBreakdownData } from "./helpers";
import {
  modelsForHighWater,
  PARSER_HIGH_WATER_STATE_VERSION,
  type DeviceParserStates,
  type ParserAggregateHighWater,
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

function sameCoverage(left: Coverage, right: Coverage): boolean {
  return COVERAGE_FIELDS.every((field) => left[field] === right[field]);
}

/**
 * Combine the legacy per-client parser ledgers of the family.
 *
 * Legacy ledgers can overlap or be disjoint. A device that submitted the same
 * response from two surfaces before the family existed holds it in both
 * ledgers, while separate sessions are distinct credited usage. Summing
 * freezes the overlap case forever; a per-field max silently drops the
 * disjoint case on replace. Neither is sound alone, so the overlap has to be
 * proven: an identical (date, model) cell in more than one ledger is the
 * signature of one response credited twice and counts once. Every other cell
 * is distinct usage and is summed. A ledger's aggregate excess beyond its own
 * day cells cannot be attributed to a cell, so it is also summed. Anything
 * unproven therefore stays conservative and freezes rather than replaces.
 */
function legacyLedgerCoverage(
  ledgers: Array<{
    days: Record<string, ClientBreakdownData>;
    aggregate: ParserAggregateHighWater;
  }>,
): { summary: CoverageSummary; unverifiable: boolean } {
  const cellsByDateModel = new Map<string, Coverage[]>();
  const summary: CoverageSummary = { total: emptyCoverage(), models: new Map() };
  let unverifiable = false;

  for (const ledger of ledgers) {
    for (const [date, cell] of Object.entries(ledger.days)) {
      for (const [modelId, model] of Object.entries(modelsForHighWater(cell))) {
        const vector = emptyCoverage();
        addCoverage(vector, model);
        const key = `${date}\u0000${modelId}`;
        const seen = cellsByDateModel.get(key) ?? [];
        if (seen.some((existing) => sameCoverage(existing, vector))) continue;
        seen.push(vector);
        cellsByDateModel.set(key, seen);
        addCoverage(summary.total, vector);
        const modelCoverage = summary.models.get(modelId) ?? emptyCoverage();
        addCoverage(modelCoverage, vector);
        summary.models.set(modelId, modelCoverage);
      }
    }

    const daysTotal = coverage(Object.values(ledger.days)).total;
    for (const field of COVERAGE_FIELDS) {
      const aggregate = ledger.aggregate[field];
      if (!Number.isSafeInteger(aggregate) || aggregate < 0) {
        unverifiable = true;
        continue;
      }
      summary.total[field] += Math.max(0, aggregate - daysTotal[field]);
    }
  }
  return { summary, unverifiable };
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
  const ledgers: Parameters<typeof legacyLedgerCoverage>[0] = [];
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
    ledgers.push({ days: state.days, aggregate: state.aggregate });
  }

  const legacy = legacyLedgerCoverage(ledgers);
  const combined = maximum(stored, legacy.summary);
  return { ...combined, unverifiable: unverifiable || legacy.unverifiable };
}
