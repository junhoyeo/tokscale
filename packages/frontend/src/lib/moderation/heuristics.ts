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
    | "impliedRate";
  /** Shown verbatim in the review UI. */
  label: string;
  weight: number;
}

export interface ScoredCandidate extends CandidateRow {
  score: number;
  signals: CandidateSignal[];
}

/** A user holding more than this share of all tokens is worth a look. */
const SITE_SHARE_THRESHOLD = 0.05;
/** Multiples of the median that stop being explainable as heavy usage. */
const MEDIAN_RATIO_THRESHOLD = 500;
/**
 * Published provider pricing sits well inside this band per token. Outside it,
 * either the cost or the token count is not what it claims to be.
 */
const MIN_IMPLIED_RATE = 0.0000001;
const MAX_IMPLIED_RATE = 0.001;
/**
 * Daily rows should sum to roughly the stored total. A large gap is the
 * fingerprint of the ratchet, not of heavy usage.
 */
const DAILY_MISMATCH_THRESHOLD = 1.5;

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
    if (impliedRate > MAX_IMPLIED_RATE || impliedRate < MIN_IMPLIED_RATE) {
      signals.push({
        key: "impliedRate",
        label: `Implied $${impliedRate.toPrecision(3)}/token is outside plausible provider pricing`,
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
