/**
 * Scoring for the moderation review queue.
 *
 * Deliberately pure and DB-free so the judgement calls are unit-testable, and
 * deliberately advisory: nothing here ever hides anyone. It only decides what a
 * human looks at first, and every signal is surfaced with a human-readable
 * reason so the reviewer can disagree with it.
 *
 * The signals are chosen to separate two very different situations that look
 * identical in the totals:
 *   - someone submitting fabricated usage, and
 *   - our own inflation bug (#960: daily active_time_ms is not
 *     timezone-invariant, so re-scanning under another TZ re-splits intervals
 *     and the monotonic per-device merge ratchets the total upward).
 * `dailyMismatch` is the signal that distinguishes them, which is why a high
 * score is a prompt to investigate rather than a verdict.
 */

export interface CandidateRow {
  userId: string;
  username: string;
  avatarUrl: string | null;
  leaderboardHidden: boolean;
  totalTokens: number;
  totalCost: number;
  submitCount: number;
  hasBackfill: boolean;
  /** Sum of this user's daily_breakdown rows. */
  dailyTokens: number;
  /** How many OTHER users report a near-identical token total. */
  nearDuplicateCount: number;
  /**
   * This user's model names that match SLOP_MODEL_PATTERNS. Pre-filtered in SQL
   * rather than sent whole: the busiest account reports 141 models, and only
   * the matches are of any interest.
   */
  slopModels: string[];
  /**
   * Sum of tokens attributed to matching slopModels from daily_breakdown.source_breakdown.
   * null if breakdown data is unavailable (legacy submissions or submissions with no
   * daily breakdown rows), in which case token share cannot be computed and full fixed weight is retained.
   */
  slopTokens: number | null;
}

export interface CandidateContext {
  /** Total tokens across all users, used for the share-of-site signal. */
  siteTokens: number;
  /** Median user's tokens, used as the "normal person" baseline. */
  medianTokens: number;
}

export interface CandidateSignal {
  key:
    | "siteShare"
    | "medianRatio"
    | "duplicateTotal"
    | "dailyMismatch"
    | "impliedRate"
    | "slopModelName";
  /** Shown verbatim in the review UI. */
  label: string;
  weight: number;
}

export interface ScoredCandidate extends CandidateRow {
  score: number;
  signals: CandidateSignal[];
}

/**
 * Substrings that only appear in a model name someone invented.
 *
 * Deliberately tiny, and every entry was checked against production before
 * being included. Two things are NOT here on purpose:
 *
 * - `test` — `test-model` is reported by 4 separate accounts, so it is someone
 *   genuinely testing rather than a fabrication.
 * - `hack` — the only hit was a tool name, and the word appears in enough
 *   legitimate contexts to be a false-positive risk.
 *
 * Statistical alternatives were measured and rejected. Model *count* looked
 * promising until the distribution came back at p50=20, p99=140, max=206 with
 * 51 accounts above 100 models — the 141-model account is unremarkable on that
 * axis. Counting models nobody else reports fails too, because the
 * one-user-only set is mostly parser debris (`*`, `{`, `│`, bare UUIDs).
 *
 * So this is a content signal, not a statistical one: a name that declares
 * itself fake is evidence in a way that an unusual count is not.
 */
export const SLOP_MODEL_PATTERNS = [
  "slop",
  "fake",
  "dummy",
  "bogus",
  "notreal",
  "madeup",
] as const;

/**
 * Case-insensitive alternation for the SQL-side pre-filter, anchored to the
 * start of a name or of a segment within it.
 *
 * Unanchored, the pattern matched anywhere inside an id, so any future
 * legitimate name that merely contains one of these words would be flagged.
 * Anchoring only the left side is deliberate: requiring a delimiter on BOTH
 * sides would stop matching `slopllm`, which is the exact shape the list is
 * written to catch. `slop-llm`, `slop/llm` and `slopllm` all still match;
 * `notaslopname` no longer does.
 */
export const SLOP_MODEL_REGEX = `(^|[^a-z0-9])(${SLOP_MODEL_PATTERNS.join("|")})`;

/** A user holding more than this share of all tokens is worth a look. */
export const SITE_SHARE_THRESHOLD = 0.05;
/** Multiples of the median that stop being explainable as heavy usage. */
export const MEDIAN_RATIO_THRESHOLD = 500;
/**
 * Only an upper bound. There is deliberately no floor.
 *
 * A low implied rate carries no signal: local models via Ollama or LM Studio
 * cost nothing, free tiers cost nothing, and cache reads are an order of
 * magnitude cheaper than input tokens — so ordinary heavy users legitimately
 * land far below any floor worth setting. Measured against real data, a
 * 1e-7 floor flagged 38 innocent accounts against 3 genuine ones, which is a
 * queue nobody would keep reading.
 *
 * The ceiling still means something: nobody pays above list price.
 */
export const MAX_IMPLIED_RATE = 0.001;
/**
 * Daily rows should sum to roughly the stored total. A large gap is the
 * fingerprint of the ratchet, not of heavy usage.
 */
export const DAILY_MISMATCH_THRESHOLD = 1.5;

function formatMultiple(value: number): string {
  return value >= 100 ? `${Math.round(value).toLocaleString("en-US")}x` : `${value.toFixed(1)}x`;
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

/**
 * Scores one candidate. Higher means "look at this sooner", nothing more.
 *
 * Signal weights are ordinal, not probabilistic — they exist to order the
 * queue. Do not read a score as a confidence that someone cheated.
 */
export function scoreCandidate(
  row: CandidateRow,
  context: CandidateContext
): ScoredCandidate {
  const signals: CandidateSignal[] = [];

  if (context.siteTokens > 0) {
    const share = row.totalTokens / context.siteTokens;
    if (share >= SITE_SHARE_THRESHOLD) {
      signals.push({
        key: "siteShare",
        label: `Holds ${formatPercent(share)} of all tokens on the site`,
        // Scaled by share so a 99% account outranks a 6% one.
        weight: 40 * share,
      });
    }
  }

  if (context.medianTokens > 0) {
    const ratio = row.totalTokens / context.medianTokens;
    if (ratio >= MEDIAN_RATIO_THRESHOLD) {
      signals.push({
        key: "medianRatio",
        label: `${formatMultiple(ratio)} the median user's tokens`,
        // Log-scaled: the gap between 500x and 5000x matters less than the
        // fact that both are far outside normal.
        weight: Math.min(25, Math.log10(ratio) * 6),
      });
    }
  }

  if (row.slopModels.length > 0) {
    // Scaled by the share of the account's tokens carried by the matching
    // models: the name speaks for itself, but only when used to book real
    // usage. Config artifacts carrying zero or negligible tokens scale down
    // to 0 and drop out of the review queue (#1265).
    //
    // When per-model breakdown data is unavailable (null slopTokens from legacy
    // rows or submissions with no daily breakdown rows), retain the original
    // full fixed weight (35) so genuine fabrications on older submissions are not lost.
    let weight = 35;
    if (row.slopTokens !== null) {
      const slopShare =
        row.totalTokens > 0
          ? Math.min(1, Math.max(0, row.slopTokens) / row.totalTokens)
          : 0;
      weight = 35 * slopShare;
    }

    if (Math.round(weight) > 0) {
      // Quoted verbatim so the reviewer judges the actual string rather than
      // trusting the match — the whole point is that the name speaks for itself.
      const shown = row.slopModels.slice(0, 3).map((name) => `"${name}"`).join(", ");
      const extra = row.slopModels.length - 3;

      signals.push({
        key: "slopModelName",
        label: `Reports invented model names: ${shown}${extra > 0 ? ` and ${extra} more` : ""}`,
        weight,
      });
    }
  }

  if (row.nearDuplicateCount > 0) {
    signals.push({
      key: "duplicateTotal",
      label:
        row.nearDuplicateCount === 1
          ? "Token total matches another account almost exactly"
          : `Token total matches ${row.nearDuplicateCount} other accounts almost exactly`,
      // Two people cannot independently land on the same total, so this is the
      // strongest single signal that something was copied.
      weight: 30,
    });
  }

  // Only meaningful when daily rows exist at all; a user with none is simply
  // an older submission shape, not evidence of anything.
  if (row.dailyTokens > 0) {
    const ratio = row.totalTokens / row.dailyTokens;
    if (ratio >= DAILY_MISMATCH_THRESHOLD) {
      signals.push({
        key: "dailyMismatch",
        label: `Stored total is ${formatMultiple(ratio)} the sum of daily rows — possible ratchet inflation (#960), not necessarily the user's doing`,
        weight: 20,
      });
    }
  }

  if (row.totalTokens > 0) {
    const impliedRate = row.totalCost / row.totalTokens;
    if (impliedRate > MAX_IMPLIED_RATE) {
      signals.push({
        key: "impliedRate",
        label: `Implied $${impliedRate.toPrecision(3)}/token is above any provider's list price`,
        weight: 15,
      });
    }
  }

  return {
    ...row,
    score: signals.reduce((sum, signal) => sum + signal.weight, 0),
    signals,
  };
}

/**
 * Scores every row and returns those with at least one signal, worst first.
 *
 * Already-hidden users are kept so the reviewer can see and reverse previous
 * decisions rather than losing track of them.
 */
export function rankCandidates(
  rows: readonly CandidateRow[],
  context: CandidateContext
): ScoredCandidate[] {
  return rows
    .map((row) => scoreCandidate(row, context))
    .filter((candidate) => candidate.signals.length > 0 || candidate.leaderboardHidden)
    .sort((left, right) => {
      if (right.score !== left.score) {
        return right.score - left.score;
      }
      return left.username.localeCompare(right.username);
    });
}
