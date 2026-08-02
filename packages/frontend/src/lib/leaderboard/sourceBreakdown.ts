import type { ParsedSearchDirectives } from "@/lib/leaderboard/searchDirectives";

/**
 * The part of `daily_breakdown.source_breakdown` the leaderboards read. Every
 * field is optional because the stored blob has grown over time and rows
 * written by older CLI versions are still on the board — see db/schema.ts for
 * the full shape.
 */
export interface PeriodClientBreakdown {
  tokens?: number;
  cost?: number;
  models?: Record<string, { tokens?: number; cost?: number }>;
}

export type PeriodSourceBreakdown = Record<string, PeriodClientBreakdown>;

/**
 * Narrows one daily row's breakdown to the tokens and cost a `client:`/`model:`
 * search actually asked for, or null when nothing in it was selected.
 *
 * A daily row's own `tokens`/`cost` are the total across every client and
 * model that shared that day and device, so a filtered board must not sum
 * them: crediting the whole row to a `client:codex` search hands the user
 * every other client they happened to run that day. The per-client and
 * per-model figures inside `sourceBreakdown` are the only ones narrow enough
 * to add up.
 *
 * `client:x model:y` is read as an intersection within one client — a model's
 * tokens count only where they were spent under a matching client. A union
 * reading would let `client:codex model:opus` re-credit Codex work that never
 * touched Opus, which is the same over-count wearing a different hat.
 *
 * Takes the breakdown rather than a row because the global and group period
 * boards carry different row shapes (a group row also has `role`) over the
 * same JSON. The duplicate that this replaced is how the group board inherited
 * the over-count in the first place.
 */
export function scopeBreakdownToDirectives(
  sourceBreakdown: PeriodSourceBreakdown | null,
  parsed: ParsedSearchDirectives
): { tokens: number; cost: number } | null {
  if (!sourceBreakdown) {
    return null;
  }

  let tokens = 0;
  let cost = 0;
  let matched = false;

  for (const [clientId, client] of Object.entries(sourceBreakdown)) {
    // Case-insensitive substring, carried over verbatim from the row filter
    // this replaced: `client:claude` has always matched `claude-code`, and
    // tightening that to equality here would quietly drop usage from every
    // saved search built against the old behaviour.
    const clientMatches =
      parsed.clients.length === 0 ||
      parsed.clients.some((candidate) => clientId.toLowerCase().includes(candidate));

    if (!clientMatches) {
      continue;
    }

    if (parsed.models.length === 0) {
      matched = true;
      tokens += Number(client.tokens) || 0;
      cost += Number(client.cost) || 0;
      continue;
    }

    for (const [modelId, model] of Object.entries(client.models ?? {})) {
      if (!parsed.models.some((candidate) => modelId.toLowerCase().includes(candidate))) {
        continue;
      }
      matched = true;
      tokens += Number(model.tokens) || 0;
      cost += Number(model.cost) || 0;
    }
  }

  // Tracked separately from a zero sum: a client that genuinely burned no
  // tokens still puts its owner on the filtered board and into the user
  // counts, whereas a row nothing matched must leave no trace at all.
  return matched ? { tokens, cost } : null;
}
