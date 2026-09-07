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
   * Sum of tokens booked under the matching `slopModels` in
   * daily_breakdown.source_breakdown, or null when this account's tokens are
   * not fully attributed to named models — no daily rows at all, a row with no
   * breakdown, a per-model map that leaves a remainder no `modelId` claims, a
   * client-level `modelId` that names nothing, tokens parked under a map key
   * that names nothing (see `UNNAMED_MODEL_REGEX`), any client entry whose
   * per-model map sums past the entry's own scalar so the entry contradicts
   * itself, or daily rows that do not cover the stored total. Null means the
   * share is unknown, not that it is small: the signal then keeps its full
   * fixed weight instead of being scaled by a share computed from partial
   * attribution.
   *
   * The over-nesting condition is stated as a property of the ENTRY, not of
   * the account's arithmetic, and that distinction is the whole point: the
   * query carries it as its own flag rather than inferring it from a shortfall
   * in the attributed sum. A contradictory entry whose own `tokens` scalar is
   * 0 or absent subtracts nothing from that sum and adds nothing to the total
   * it is compared against, so before the flag existed this field came back 2
   * for a map holding 2 slop tokens against a scalar of 0 — measured, not
   * theorised — in flat contradiction of this doc. Do not re-express the
   * condition as a subtraction.
   */
  slopTokens: number | null;
  /**
   * The completeness gate's own clause values, re-exported from the
   * candidates query so the unknowable-reason classification
   * (aggregateUnknowableStats) reports the gate's actual inputs rather than
   * re-deriving them. Only meaningful on rows with a slop model match — the
   * attribution CTEs are scoped to those in SQL; the values below are the
   * NULL-coalesced defaults anywhere else and must not be read as a verdict
   * for non-slop accounts.
   */
  hasOverNestedEntry: boolean;
  everyDayAttributed: boolean;
  /** Tokens the daily rows attribute to SOME named model. */
  attributedTokens: number;
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
 * Why a slop-matched candidate's token attribution came back unknowable
 * (`slopTokens === null`). Exists for observability, not scoring: the
 * fail-closed decision lives in the candidates query's completeness gate, and
 * this classification re-derives which clause of that gate failed so an
 * operator can tell how often the fail-closed path fires and why.
 */
export type UnknowableReason =
  /**
   * At least one daily_breakdown row carries no source_breakdown at all
   * (legacy pre-breakdown shape). every_day_attributed failed.
   */
  | "missing_breakdown"
  /**
   * At least one client entry's per-model map sums past the entry's own
   * scalar, so the entry contradicts itself (over_nested). This dominates the
   * other classes: a contradictory entry is never repaired by better
   * arithmetic elsewhere in the same submission.
   */
  | "over_nested"
  /**
   * Every row has a breakdown and no entry contradicts itself, but the
   * attributed sum still falls short of the stored total: a scalar remainder
   * no modelId claims, or tokens parked under a key that names nothing
   * (unknown / parser debris). attributed_tokens < total_tokens.
   */
  | "unattributed_tokens"
  /**
   * The completeness gate failed but none of the three known clauses can be
   * the cause — the SQL drifted ahead of this classifier. Distinct so a
   * drifted gate surfaces as its own bucket in the telemetry instead of being
   * misattributed to one of the measured classes.
   */
  | "unknown";

/**
 * One invocation's breadth measurement: how many slop-matched candidates the
 * fail-closed path swallowed, by reason, and the share of those candidates'
 * tokens that no named model accounts for. Percentages are computed by the
 * consumer (log pipeline, dashboard) over whatever window it aggregates.
 */
export interface UnknowableStats {
  /** Candidates naming a slop model whose slopTokens could be computed. */
  knowable: number;
  /** Candidates naming a slop model whose slopTokens came back null. */
  unknowable: number;
  byReason: Record<UnknowableReason, number>;
  /**
   * Sum over unknowable candidates of GREATEST(total - attributed, 0) — the
   * tokens no named model accounts for. The clamp is defence against the
   * attributed sum overshooting the stored total on drifted data; on
   * well-formed rows the gate itself guarantees attributed < total.
   */
  unattributedTokens: number;
  /** Sum of unknowable candidates' stored totals. */
  unknowableTotalTokens: number;
  /**
   * Log-scale histogram of per-candidate unattributed tokens over the
   * unknowable set: key n counts candidates with at least
   * 2**n * UNKNOWABLE_BUCKET_WIDTH unattributed tokens. The largest key is
   * open-ended — a gap beyond it lands there rather than under a key the
   * allocation never made. The flat
   * unattributedTokens / unknowableTotalTokens pair cannot say whether the
   * missing share is a rounding error or the whole submission — one account
   * missing 99% and nine missing 1% aggregate identically — and whether it
   * is one large account or many small ones decides whether the fail-closed
   * path is a rare safety net or a routine outcome.
   */
  unattributedHistogram: Record<string, number>;
}

/** Bucket boundaries for the unattributed-tokens histogram. */
const UNKNOWABLE_HISTOGRAM_BUCKETS = 8;

/**
 * Classifies one candidate's NULL slopTokens into the gate clause that
 * produced it. Checked in dominance order: an over-nested entry makes the
 * whole submission unknowable whatever else is true of it, and a missing
 * breakdown makes the attributed sum unobservable rather than merely short.
 */
export function classifyUnknowableReason(row: CandidateRow): UnknowableReason {
  if (row.hasOverNestedEntry) {
    return "over_nested";
  }
  if (!row.everyDayAttributed) {
    return "missing_breakdown";
  }
  if (row.attributedTokens < row.totalTokens) {
    return "unattributed_tokens";
  }
  return "unknown";
}

/**
 * Aggregates one getModerationCandidates() result into the fail-closed
 * breadth measurement. Only candidates with at least one slop model name are
 * counted at all: the attribution work is scoped to them in SQL, so a
 * non-slop account says nothing about how often the path fires.
 */
export function aggregateUnknowableStats(
  candidates: readonly CandidateRow[]
): UnknowableStats {
  const stats: UnknowableStats = {
    knowable: 0,
    unknowable: 0,
    byReason: {
      missing_breakdown: 0,
      over_nested: 0,
      unattributed_tokens: 0,
      unknown: 0,
    },
    unattributedTokens: 0,
    unknowableTotalTokens: 0,
    unattributedHistogram: Object.fromEntries(
      Array.from({ length: UNKNOWABLE_HISTOGRAM_BUCKETS }, (_, i) => [
        String(2 ** i * UNKNOWABLE_BUCKET_WIDTH),
        0,
      ])
    ),
  };

  for (const candidate of candidates) {
    if (candidate.slopModels.length === 0) {
      continue;
    }
    if (candidate.slopTokens !== null) {
      stats.knowable += 1;
      continue;
    }
    stats.unknowable += 1;
    stats.byReason[classifyUnknowableReason(candidate)] += 1;
    const unattributed = Math.max(
      0,
      candidate.totalTokens - candidate.attributedTokens
    );
    stats.unattributedTokens += unattributed;
    stats.unknowableTotalTokens += candidate.totalTokens;
    // Cap the walk at the largest allocated key: a gap past it still counts
    // in every bucket up to and including the top one, instead of minting a
    // key the allocation above never made — `undefined + 1` is NaN, which
    // JSON.stringify then serializes as null, corrupting the telemetry on
    // exactly the largest accounts.
    const cap = Math.min(
      unattributed,
      2 ** (UNKNOWABLE_HISTOGRAM_BUCKETS - 1) * UNKNOWABLE_BUCKET_WIDTH
    );
    for (
      let boundary = UNKNOWABLE_BUCKET_WIDTH;
      boundary <= cap;
      boundary *= 2
    ) {
      stats.unattributedHistogram[String(boundary)] += 1;
    }
  }

  return stats;
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

/**
 * Keys of a `source_breakdown` per-model map that identify no model, so the
 * tokens under them are unattributed however complete the map looks.
 *
 * `unknown` is routine modern data, not a legacy artifact, and it arrives by
 * three independent routes:
 *
 *   - Parsers emit it as the model id of a token-bearing message whose model
 *     is missing or blank. `model_id()` in sessions/augment.rs and in
 *     sessions/jcode.rs both return "unknown" for a blank id — augment.rs has
 *     a test asserting exactly that for a message carrying 7 input and 1
 *     output tokens — and sessions/claudecode.rs and sessions/gemini.rs fall
 *     back to the same literal.
 *   - normalizeSubmissionData() in app/api/submit/route.ts rewrites any null,
 *     non-string or whitespace-only `modelId` to the literal "unknown" on
 *     every POST /api/submit, for every client, before validation. The map key
 *     is that value verbatim.
 *   - modelsForHighWater() in lib/db/parserHighWater.ts parks an entry's
 *     unclaimed scalar remainder under `breakdown.modelId || "unknown"`, and
 *     breakdownFromModels() then rewrites the entry's scalar as the sum of
 *     that map. A remainder that used to be visible as `tokens` > Σ`models`
 *     therefore comes back as an explicit cell whose key names nothing, with
 *     the scalar and the nested sum in agreement — so checking only for a
 *     scalar remainder no longer sees it.
 *
 * Treating those tokens as unattributed is correct in every one of the three:
 * a token whose model is the string "unknown" is a token no model claims. But
 * the blast radius is wide and it is not confined to legacy rows — a single
 * such token anywhere in an account's daily rows makes `slopTokens` NULL, so
 * the slopModelName signal keeps its full fixed weight and the #1265 share
 * scaling never applies to that account. That is the fail-closed direction (a
 * share computed from partial attribution can only be too small), and it is
 * chosen deliberately over an upper bound like (slop + unattributed) / total,
 * which puts the fabrication case back on an estimate. Narrowing it needs
 * measured evidence about how `unknown` tokens are distributed across real
 * source_breakdown rows; nobody has run that query, so do not narrow it on the
 * assumption that these cells are rare.
 *
 * Keys holding no alphanumeric character at all are the parser debris
 * documented above (`*`, `{`, `│`), which cannot be a model id either.
 *
 * Bare UUIDs are deliberately NOT matched. They are common debris, but a UUID
 * is also a plausible fine-tune or deployment id, and unlike `unknown` no
 * code path here parks a remainder under one — so treating them as unnamed
 * would pin accounts at full weight on a guess.
 */
export const UNNAMED_MODEL_REGEX = `^([^a-zA-Z0-9]*|unknown)$`;

/**
 * Width of one measurement bucket for the unknowable-breadth telemetry, in
 * tokens. Each bucket boundary is 2**n * UNKNOWABLE_BUCKET_WIDTH, so bucket
 * n reads as "[2**n * width, 2**(n+1) * width) tokens unattributed". Fixed by
 * the emitted log schema: widening it re-baselines every aggregate built on
 * the bucket keys, so it is a constant, not a config knob.
 */
export const UNKNOWABLE_BUCKET_WIDTH = 1_000_000;

/**
 * The single structured-log event name the fail-closed breadth telemetry is
 * emitted under. One line per getModerationCandidates() invocation,
 * unconditionally: a window where every slop-matched candidate was knowable
 * still contributes its denominator (knowable > 0, unknowable = 0) instead of
 * silence, so the rate is derivable rather than only its failures. The JSON
 * payload carries the counts, per-reason breakdown, and unattributed-token
 * histogram. Operators aggregate by `event` over any log window to answer
 * "what fraction of submissions went unknowable in window W".
 */
export const UNKNOWABLE_EVENT = "moderation_unknowable_submissions";

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
    // When the account's tokens are not fully attributed to named models
    // (null slopTokens), retain the original full fixed weight (35): a partial
    // attribution divided by the full total understates the share, and
    // understating it here is how a genuine fabrication leaves the queue.
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
