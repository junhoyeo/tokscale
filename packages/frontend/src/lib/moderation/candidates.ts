import { sql } from "drizzle-orm";
import { db } from "@/lib/db";
import {
  rankCandidates,
  type CandidateRow,
  type ScoredCandidate,
} from "./heuristics";

/**
 * How close two token totals must be to count as "the same data in two
 * accounts". The observed case differed by exactly 1 token, so this only has
 * to tolerate rounding, not genuine coincidence.
 */
const NEAR_DUPLICATE_TOKENS = 10;

/**
 * Rows returned to the reviewer. The site totals used for scoring are computed
 * across every user in the CTEs, so this cap only bounds the payload, not the
 * denominators.
 */
const CANDIDATE_LIMIT = 500;

interface CandidateDbRow extends Record<string, unknown> {
  user_id: string;
  username: string;
  avatar_url: string | null;
  leaderboard_hidden: boolean;
  total_tokens: number | string | null;
  total_cost: number | string | null;
  submit_count: number | string | null;
  has_backfill: boolean | null;
  daily_tokens: number | string | null;
  near_duplicate_count: number | string | null;
  site_tokens: number | string | null;
  median_tokens: number | string | null;
}

/**
 * Values out of db.execute() are driver-shaped, not schema-shaped: Postgres
 * bigint and numeric both arrive as strings via postgres-js. Coerce at the
 * boundary — the generic on db.execute<T>() is an unchecked assertion, and
 * trusting it is what previously made rank silently vanish from the badges.
 */
function toNumber(value: number | string | null | undefined): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

/**
 * Builds the review queue: every user with at least one suspicion signal, plus
 * everyone currently hidden so past decisions stay visible and reversible.
 *
 * Read-only. Nothing here changes state — hiding is always an explicit,
 * human-initiated action.
 */
export async function getModerationCandidates(): Promise<ScoredCandidate[]> {
  const result = await db.execute<CandidateDbRow>(sql`
    WITH per_user AS (
      SELECT
        u.id AS user_id,
        u.username,
        u.avatar_url,
        u.leaderboard_hidden,
        s.id AS submission_id,
        s.total_tokens,
        CAST(s.total_cost AS DECIMAL(18,4)) AS total_cost,
        s.submit_count,
        s.has_backfill
      FROM users u
      JOIN submissions s ON s.user_id = u.id
    ),
    daily AS (
      SELECT d.submission_id, SUM(d.tokens) AS daily_tokens
      FROM daily_breakdown d
      GROUP BY d.submission_id
    ),
    site AS (
      SELECT
        COALESCE(SUM(total_tokens), 0) AS site_tokens,
        COALESCE(
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY total_tokens::numeric),
          0
        ) AS median_tokens
      FROM per_user
    ),
    dupes AS (
      SELECT a.user_id, COUNT(*) AS near_duplicate_count
      FROM per_user a
      JOIN per_user b
        ON a.user_id <> b.user_id
       AND a.total_tokens > 0
       AND ABS(a.total_tokens - b.total_tokens) <= ${NEAR_DUPLICATE_TOKENS}
      GROUP BY a.user_id
    )
    SELECT
      p.user_id,
      p.username,
      p.avatar_url,
      p.leaderboard_hidden,
      p.total_tokens,
      p.total_cost,
      p.submit_count,
      p.has_backfill,
      COALESCE(dl.daily_tokens, 0) AS daily_tokens,
      COALESCE(dp.near_duplicate_count, 0) AS near_duplicate_count,
      site.site_tokens,
      site.median_tokens
    FROM per_user p
    LEFT JOIN daily dl ON dl.submission_id = p.submission_id
    LEFT JOIN dupes dp ON dp.user_id = p.user_id
    CROSS JOIN site
    -- Hidden users sort first so a past decision can never fall off the end of
    -- the cap and become invisible to review.
    ORDER BY p.leaderboard_hidden DESC, p.total_tokens DESC
    LIMIT ${CANDIDATE_LIMIT}
  `);

  const dbRows = (result as unknown as CandidateDbRow[]) ?? [];

  if (dbRows.length === 0) {
    return [];
  }

  const rows: CandidateRow[] = dbRows.map((row) => ({
    userId: row.user_id,
    username: row.username,
    avatarUrl: row.avatar_url,
    leaderboardHidden: row.leaderboard_hidden === true,
    totalTokens: toNumber(row.total_tokens),
    totalCost: toNumber(row.total_cost),
    submitCount: toNumber(row.submit_count),
    hasBackfill: row.has_backfill === true,
    dailyTokens: toNumber(row.daily_tokens),
    nearDuplicateCount: toNumber(row.near_duplicate_count),
  }));

  return rankCandidates(rows, {
    siteTokens: toNumber(dbRows[0].site_tokens),
    medianTokens: toNumber(dbRows[0].median_tokens),
  });
}
