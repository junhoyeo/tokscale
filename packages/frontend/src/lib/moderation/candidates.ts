import { sql } from "drizzle-orm";
import { db } from "@/lib/db";
import {
  DAILY_MISMATCH_THRESHOLD,
  MAX_IMPLIED_RATE,
  MEDIAN_RATIO_THRESHOLD,
  SLOP_MODEL_REGEX,
  rankCandidates,
  SITE_SHARE_THRESHOLD,
  type CandidateRow,
  type ScoredCandidate,
} from "./heuristics";

/**
 * How close two token totals must be to count as "the same data in two
 * accounts". The observed case differed by exactly 1 token, so this only has
 * to tolerate rounding, not genuine coincidence.
 */
const NEAR_DUPLICATE_TOKENS = 10;

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
  slop_models: string[] | null;
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
        s.has_backfill,
        -- Pre-filtered here rather than shipping the whole array: the busiest
        -- account reports 141 models and only the matches are of interest.
        COALESCE(
          (
            SELECT array_agg(DISTINCT m)
            FROM unnest(s.models_used) AS m
            WHERE m ~* ${SLOP_MODEL_REGEX}
          ),
          ARRAY[]::text[]
        ) AS slop_models
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
    enriched AS (
      SELECT
        p.*,
        COALESCE(dl.daily_tokens, 0) AS daily_tokens,
        CASE WHEN p.total_tokens > 0 THEN
          COUNT(*) OVER (
            ORDER BY p.total_tokens
            RANGE BETWEEN ${NEAR_DUPLICATE_TOKENS} PRECEDING
              AND ${NEAR_DUPLICATE_TOKENS} FOLLOWING
          ) - 1
        ELSE 0 END AS near_duplicate_count,
        site.site_tokens,
        site.median_tokens
      FROM per_user p
      LEFT JOIN daily dl ON dl.submission_id = p.submission_id
      CROSS JOIN site
    ),
    eligible AS (
      SELECT *
      FROM enriched
      WHERE leaderboard_hidden = true
        OR (site_tokens > 0 AND total_tokens::numeric / site_tokens >= ${SITE_SHARE_THRESHOLD})
        OR (median_tokens > 0 AND total_tokens::numeric / median_tokens >= ${MEDIAN_RATIO_THRESHOLD})
        OR near_duplicate_count > 0
        OR cardinality(slop_models) > 0
        OR (daily_tokens > 0 AND total_tokens::numeric / daily_tokens >= ${DAILY_MISMATCH_THRESHOLD})
        OR (
          total_tokens > 0
          AND total_cost / total_tokens::numeric > ${MAX_IMPLIED_RATE}
        )
    )
    SELECT
      user_id, username, avatar_url, leaderboard_hidden, total_tokens,
      total_cost, submit_count, has_backfill, daily_tokens,
      near_duplicate_count, slop_models, site_tokens, median_tokens
    FROM eligible
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
    slopModels: Array.isArray(row.slop_models) ? row.slop_models : [],
  }));

  return rankCandidates(rows, {
    siteTokens: toNumber(dbRows[0].site_tokens),
    medianTokens: toNumber(dbRows[0].median_tokens),
  });
}
