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
type PriorDay = { date?: string; sourceBreakdown: unknown };

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

type LegacyLedger = {
  days: Record<string, ClientBreakdownData>;
  aggregate: Coverage;
};

/** Per-field lifetime of one ledger: its aggregate or its day cells, whichever is larger. */
function ledgerLifetime(ledger: LegacyLedger): Coverage {
  const lifetime = coverage(Object.values(ledger.days)).total;
  for (const field of COVERAGE_FIELDS) {
    const aggregate = ledger.aggregate[field];
    if (Number.isSafeInteger(aggregate) && aggregate > lifetime[field]) {
      lifetime[field] = aggregate;
    }
  }
  return lifetime;
}

/**
 * Normalize legacy Antigravity model IDs to their canonical family key when
 * comparing pre-transition per-surface parser ledgers. This lets surfaces that
 * recorded the same response before and after model canonicalization
 * (`gemini-default` / `gemini-3.5-flash-*`, `claude-*-thinking` -> `claude-*`)
 * fold as one lifetime while keeping distinct models (e.g. `gemini-3-pro` vs.
 * `claude-sonnet-4-6`) separate even when their seven-field totals match.
 */
function canonicalLedgerModelId(modelId: string): string {
  const trimmed = modelId.trim().toLowerCase();
  if (
    trimmed === "gemini-default" ||
    trimmed === "gemini-3-flash-a" ||
    trimmed === "gemini-3-flash-preview" ||
    trimmed === "gemini-3.5-flash" ||
    trimmed.startsWith("gemini-3.5-flash-")
  ) {
    return "gemini-3.5-flash";
  }
  if (trimmed.startsWith("claude-") && trimmed.endsWith("-thinking")) {
    return trimmed.slice(0, -"-thinking".length);
  }
  return trimmed;
}

function ledgerModels(ledger: LegacyLedger): Set<string> {
  const models = new Set<string>();
  for (const cell of Object.values(ledger.days)) {
    for (const modelId of Object.keys(modelsForHighWater(cell))) {
      models.add(canonicalLedgerModelId(modelId));
    }
  }
  return models;
}

/**
 * Ledgers that record one credited lifetime from different surfaces. Parsers
 * can date the same conversation differently (session start vs. per
 * generation) and canonicalize model IDs across releases, so their day cells
 * need not match. Identical lifetime totals on every field, the same canonical
 * models and at least one shared date are treated as the same lifetime.
 * Distinct usage with coincidentally equal totals is kept apart by requiring
 * matching canonical models and a shared date.
 */
function oneLifetime(ledgers: LegacyLedger[]): boolean {
  if (ledgers.length < 2) return false;
  const lifetime = ledgerLifetime(ledgers[0]);
  const models = [...ledgerModels(ledgers[0])].sort().join("\u0000");
  const dateSets = ledgers.map((ledger) => new Set(Object.keys(ledger.days)));
  return ledgers.every((ledger, index) =>
    sameCoverage(ledgerLifetime(ledger), lifetime) &&
    [...ledgerModels(ledger)].sort().join("\u0000") === models &&
    dateSets.every((other, otherIndex) =>
      otherIndex === index || [...dateSets[index]].some((date) => other.has(date)),
    ),
  );
}

/**
 * Combine the legacy per-client parser ledgers of the family.
 *
 * Legacy ledgers can overlap or be disjoint. A device that submitted the same
 * response from two surfaces before the family existed holds it in both
 * ledgers, while separate sessions are distinct credited usage. Summing
 * freezes the overlap case forever; a per-field max silently drops the
 * disjoint case on replace. Neither is sound alone, so a ledger is only
 * collapsed into another on a strong overlap signature, and everything else
 * stays conservative (summed, so an uncovered snapshot freezes, never drops):
 *
 * - Whole ledgers recording one lifetime (see `oneLifetime`) fold by max.
 * - Otherwise an identical (date, canonical model) cell in more than one
 *   ledger is treated as one response credited twice and counts once. Every
 *   other cell is distinct usage and is summed. A ledger's aggregate excess
 *   beyond its own day cells cannot be attributed to a cell, so it is also
 *   summed.
 *
 * Both are strong heuristics, not proofs: distinct usage whose totals match
 * on all seven fields would be undercounted.
 */
function legacyLedgerCoverage(
  ledgers: LegacyLedger[],
): { summary: CoverageSummary; unverifiable: boolean } {
  if (oneLifetime(ledgers)) {
    let summary: CoverageSummary = { total: emptyCoverage(), models: new Map() };
    let unverifiable = false;
    for (const ledger of ledgers) {
      const single = legacyLedgerCoverage([ledger]);
      summary = maximum(summary, single.summary);
      unverifiable ||= single.unverifiable;
    }
    return { summary, unverifiable };
  }

  const cellsByDateModel = new Map<string, Coverage[]>();
  const summary: CoverageSummary = { total: emptyCoverage(), models: new Map() };
  let unverifiable = false;

  for (const ledger of ledgers) {
    const seenInEarlierLedgers = new Map(
      [...cellsByDateModel.entries()].map(([key, vectors]) => [key, [...vectors]]),
    );
    for (const [date, cell] of Object.entries(ledger.days)) {
      for (const [modelId, model] of Object.entries(modelsForHighWater(cell))) {
        const vector = emptyCoverage();
        addCoverage(vector, model);
        const canonicalModelId = canonicalLedgerModelId(modelId);
        const key = `${date}\u0000${canonicalModelId}`;
        const earlier = seenInEarlierLedgers.get(key) ?? [];
        const matchIndex = earlier.findIndex((existing) =>
          sameCoverage(existing, vector),
        );
        if (matchIndex !== -1) {
          earlier.splice(matchIndex, 1);
          continue;
        }
        const seen = cellsByDateModel.get(key) ?? [];
        seen.push(vector);
        cellsByDateModel.set(key, seen);
        addCoverage(summary.total, vector);
        const modelCoverage = summary.models.get(canonicalModelId) ?? emptyCoverage();
        addCoverage(modelCoverage, vector);
        summary.models.set(canonicalModelId, modelCoverage);
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
  const ledgers: LegacyLedger[] = [];
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
    if (
      !COVERAGE_FIELDS.every(
        (field) =>
          Number.isSafeInteger(state.aggregate[field]) &&
          state.aggregate[field] >= 0,
      )
    ) {
      unverifiable = true;
      continue;
    }
    if (state.stateVersion == null) {
      // Schema v1 stored every observed cell maximum (`advanceDays`) and
      // advanced its aggregate even when no cell could accept that growth.
      // Rebuild its ledger from the rows that were actually credited in
      // `existingDays`, matching `planParserHighWaterSubmission`.
      const creditedCells = existingBreakdowns
        .map((breakdown) => ownValue(breakdown, client))
        .filter((cell): cell is ClientBreakdownData => cell !== undefined);
      const creditedDays: Record<string, ClientBreakdownData> = {};
      existingDays.forEach((day, index) => {
        const cell = ownValue(existingBreakdowns[index], client);
        if (cell) creditedDays[day.date ?? String(index)] = cell;
      });
      ledgers.push({
        days: creditedDays,
        aggregate: coverage(creditedCells).total,
      });
      continue;
    }
    ledgers.push({ days: state.days, aggregate: state.aggregate });
  }

  const legacy = legacyLedgerCoverage(ledgers);
  const combined = maximum(stored, legacy.summary);
  return { ...combined, unverifiable: unverifiable || legacy.unverifiable };
}
