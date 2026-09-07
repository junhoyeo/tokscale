import { sql } from "drizzle-orm";
import { db } from "@/lib/db";
import {
  DAILY_MISMATCH_THRESHOLD,
  MAX_IMPLIED_RATE,
  MEDIAN_RATIO_THRESHOLD,
  SLOP_MODEL_REGEX,
  UNNAMED_MODEL_REGEX,
  UNKNOWABLE_EVENT,
  aggregateUnknowableStats,
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
  slop_tokens: number | string | null;
  has_over_nested_entry: boolean | null;
  every_day_attributed: boolean | null;
  attributed_tokens: number | string | null;
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
      SELECT
        d.submission_id,
        SUM(d.tokens) AS daily_tokens,
        -- COALESCE rather than a bare comparison: jsonb_typeof(NULL) is NULL
        -- and BOOL_AND skips NULL inputs, so one legacy row with no breakdown
        -- next to one attributed row would still report "all attributed".
        BOOL_AND(
          COALESCE(jsonb_typeof(d.source_breakdown) = 'object', false)
        ) AS every_day_attributed
      FROM daily_breakdown d
      GROUP BY d.submission_id
    ),
    -- Scoped to accounts that actually have a matching name: the per-model
    -- expansion below is the expensive part of this query, and slop_tokens is
    -- only ever read when slop_models is non-empty.
    slop_users AS (
      SELECT submission_id
      FROM per_user
      WHERE cardinality(slop_models) > 0
    ),
    -- One row per (daily row, client entry). nested_tokens is what the
    -- entry's per-model map accounts for; remainder is the scalar total a
    -- legacy entry carries beyond it. modelsForHighWater() in
    -- lib/db/parserHighWater.ts credits exactly that remainder to the entry's
    -- own modelId, so this reads it back the same way instead of dropping
    -- it. Without a modelId the remainder belongs to no named model, and it
    -- is deliberately left out of attributed_tokens below.
    --
    -- attributed_nested_tokens is the part of the map that a model actually
    -- claims. It is separate from nested_tokens because the two answer
    -- different questions: the remainder is what the map does not cover
    -- (so it must subtract the WHOLE map, unnamed cells included, or the
    -- same tokens get counted twice), while attribution is what the map
    -- credits to a model (so it must skip the cells that name nothing).
    --
    -- The map is not guaranteed to fit inside the entry's own scalar. When it
    -- does not, the entry contradicts itself: the scalar says N tokens were
    -- submitted and the map claims more than N are accounted for, so nothing
    -- the map holds can be reconciled against the account's total. Such an
    -- entry raises over_nested, and the completeness gate below fails the
    -- whole submission closed on that flag alone.
    --
    -- Such an entry ALSO attributes nothing, but be clear about what that
    -- second guard is worth: it is unobservable. The flag alone already forces
    -- slop_tokens to NULL whatever the attributed sum comes out at, so
    -- removing the zeroing leaves the integration fixture entirely green
    -- (measured: 17/17). It is kept as defence in depth against a future
    -- narrowing of the flag, not because any test can tell it is there — and
    -- for the same reason a clamp put in its place would also look green. The
    -- fixture header records which mutation each persona actually kills.
    --
    -- The flag is load-bearing and the zeroed attribution alone is NOT enough,
    -- because the gate is an inequality over sums and a zero-scalar entry
    -- moves neither side of it. attributed_tokens is compared against
    -- total_tokens, which is SUM(daily.tokens) and in turn SUM(client scalars)
    -- (recalculateDayTotals in lib/db/helpers.ts adds client.tokens || 0).
    -- An entry whose own scalar is 0 or absent therefore adds 0 to the
    -- threshold as well as 0 to the attributed sum, so zeroing it opens no gap
    -- and the gate passes EXACTLY, on the passing side. Measured against
    -- postgres:16 before this flag existed: {claude: scalar 1,200,000 / map
    -- {claude-sonnet-4: 1,200,000}} beside {copilot: scalar 0 / map
    -- {fake-api: 2}} returned slopTokens 2 rather than NULL, weight
    -- 35 * 2/1,200,000, which Math.round drops -- the slopModelName signal
    -- vanished and rankCandidates() dropped the account. Deleting the tokens
    -- key outright behaved identically. This is the same failure the clamp
    -- note below describes, one construction over: there the ceiling equals
    -- the scalar, here the ceiling IS zero and so is the scalar.
    --
    -- nested_slop_tokens is deliberately NOT zeroed alongside the attribution.
    -- It is the numerator, and zeroing it would round the weight to 0 on its
    -- own -- fail-open by a second route -- if this gate were ever loosened.
    -- The over-nested entry's slop tokens are only ever read when the whole
    -- submission is already NULL, so the pass-through is inert by design.
    --
    -- applyCostCompleteness() in lib/db/helpers.ts unions the stored and
    -- incoming model maps while taking the scalar from the incoming entry, so
    -- a same-device resubmit declaring costIsComplete:false (#1044) that drops
    -- a previously-seen model stores exactly that divergence.
    --
    -- CLAMPING the named sum instead is what let this fail open before the
    -- flag existed. Reading that history as current behaviour would be wrong:
    -- with over_nested standing, the clamp below decides nothing either way.
    -- It is recorded because it explains why the guard has to sit OUTSIDE the
    -- attributed sum. Every clamp of the named cells has a
    -- ceiling of the entry's OWN scalar, while the gate's threshold is
    -- total_tokens -- the sum of those same scalars. So a clamped over-nested
    -- entry does not land below the gate, it lands exactly ON it, which is the
    -- passing side. GREATEST(LEAST(named, scalar - unnamed), 0) only bit when
    -- an unnamed cell happened to subtract from that ceiling; with the excess
    -- under a real model name, or in a map holding no unnamed cell at all
    -- (stored {claude-sonnet-4: N} merged with an incoming legacy-partial
    -- {fake-api: 2}), attribution came back equal to the total while only 2
    -- tokens were backed by the submission that set the scalar.
    --
    -- On a well-formed entry, whose map fits inside its scalar, this is inert:
    -- named <= nested <= scalar, so no clamp was ever doing work there.
    client_attribution AS (
      SELECT
        d.submission_id,
        m.nested_tokens > COALESCE((client.value->>'tokens')::numeric, 0)
          AS over_nested,
        CASE
          WHEN m.nested_tokens > COALESCE((client.value->>'tokens')::numeric, 0)
            THEN 0
          ELSE m.nested_named_tokens
        END AS attributed_nested_tokens,
        m.nested_slop_tokens,
        GREATEST(
          COALESCE((client.value->>'tokens')::numeric, 0) - m.nested_tokens,
          0
        ) AS remainder,
        CASE
          WHEN jsonb_typeof(client.value->'modelId') = 'string'
            AND client.value->>'modelId' !~* ${UNNAMED_MODEL_REGEX}
          THEN client.value->>'modelId'
        END AS remainder_model
      FROM daily_breakdown d
      JOIN slop_users su ON su.submission_id = d.submission_id
      CROSS JOIN LATERAL jsonb_each(
        CASE WHEN jsonb_typeof(d.source_breakdown) = 'object' THEN d.source_breakdown ELSE '{}'::jsonb END
      ) AS client(key, value)
      CROSS JOIN LATERAL (
        SELECT
          COALESCE(SUM(COALESCE((model.value->>'tokens')::numeric, 0)), 0)
            AS nested_tokens,
          COALESCE(
            SUM(COALESCE((model.value->>'tokens')::numeric, 0))
              FILTER (WHERE model.key !~* ${UNNAMED_MODEL_REGEX}),
            0
          ) AS nested_named_tokens,
          COALESCE(
            SUM(COALESCE((model.value->>'tokens')::numeric, 0))
              FILTER (WHERE model.key ~* ${SLOP_MODEL_REGEX}),
            0
          ) AS nested_slop_tokens
        FROM jsonb_each(
          CASE WHEN jsonb_typeof(client.value->'models') = 'object' THEN client.value->'models' ELSE '{}'::jsonb END
        ) AS model(key, value)
      ) AS m
    ),
    slop_usage AS (
      SELECT
        submission_id,
        -- Any single self-contradictory entry makes the whole submission's
        -- attribution unknowable, so this is BOOL_OR and not a per-entry
        -- subtraction: see the over_nested note above for why subtraction
        -- cannot express it.
        BOOL_OR(over_nested) AS has_over_nested_entry,
        SUM(
          nested_slop_tokens
          + CASE WHEN remainder_model ~* ${SLOP_MODEL_REGEX} THEN remainder ELSE 0 END
        ) AS slop_tokens,
        -- Everything these same entries attribute to SOME named model. Adding
        -- the remainder to the nested sum cannot double count it: the
        -- remainder is by construction what the whole nested sum leaves over,
        -- and only the named part of that sum is counted here. A non-zero
        -- remainder also implies the entry was not over-nested, since a map
        -- that leaves the scalar something over cannot have outrun it.
        SUM(
          attributed_nested_tokens
          + CASE WHEN remainder_model IS NOT NULL THEN remainder ELSE 0 END
        ) AS attributed_tokens
      FROM client_attribution
      GROUP BY submission_id
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
        -- A share is only meaningful when every one of this account's tokens
        -- is attributed to a named model. Tokens nobody claims — a scalar
        -- remainder with no modelId, or a cell keyed 'unknown' or '*' — could
        -- be all slop, and dividing the attributed part by the full total
        -- scales the weight towards zero, which drops a real fabrication out
        -- of the queue. Incomplete attribution therefore yields NULL, which
        -- the scorer reads as "share unknown" and answers with the full
        -- weight.
        --
        -- has_over_nested_entry is a separate clause rather than another way
        -- of failing the sum comparison, because a self-contradictory entry
        -- whose own scalar is 0 or absent contributes nothing to EITHER side
        -- of that comparison and so cannot fail it. See the over_nested note
        -- in client_attribution for the measurement.
        CASE
          WHEN dl.every_day_attributed = true
            AND COALESCE(su.has_over_nested_entry, false) = false
            AND COALESCE(su.attributed_tokens, 0) >= p.total_tokens
          THEN COALESCE(su.slop_tokens, 0)
          ELSE NULL
        END AS slop_tokens,
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
      LEFT JOIN slop_usage su ON su.submission_id = p.submission_id
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
      near_duplicate_count, slop_models, slop_tokens,
      -- Observability for the fail-closed completeness gate above, NOT inputs
      -- to the decision: they re-export the gate's own clause values verbatim
      -- so the application can report which clause failed without
      -- re-deriving it. The attribution CTEs are scoped to submissions with a
      -- slop model match, so these come back NULL anywhere else and must not
      -- be read as a verdict for non-slop accounts.
      (SELECT su.has_over_nested_entry FROM slop_usage su
        WHERE su.submission_id = eligible.submission_id) AS has_over_nested_entry,
      (SELECT dl.every_day_attributed FROM daily dl
        WHERE dl.submission_id = eligible.submission_id) AS every_day_attributed,
      (SELECT su.attributed_tokens FROM slop_usage su
        WHERE su.submission_id = eligible.submission_id) AS attributed_tokens,
      site_tokens, median_tokens
    FROM eligible
  `);

  const dbRows = (result as unknown as CandidateDbRow[]) ?? [];

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
    slopTokens: row.slop_tokens == null ? null : toNumber(row.slop_tokens),
    hasOverNestedEntry: row.has_over_nested_entry === true,
    everyDayAttributed: row.every_day_attributed === true,
    attributedTokens: toNumber(row.attributed_tokens),
  }));

  const stats = aggregateUnknowableStats(rows);
  // One structured line per invocation, unconditionally. This is the
  // telemetry the breadth question is answered from: aggregate by event over
  // any log window to get the fraction of slop-matched submissions that were
  // unknowable, broken down by which gate clause failed and how many of their
  // tokens no named model accounts for. The line must also fire when nothing
  // was unknowable — the rate is a fraction, and a fully-knowable window has
  // to contribute its denominator; emitting only failures cannot distinguish
  // "nothing failed" from "nothing was measured".
  console.warn(
    `[moderation] ${JSON.stringify({
      event: UNKNOWABLE_EVENT,
      knowable: stats.knowable,
      unknowable: stats.unknowable,
      byReason: stats.byReason,
      unattributedTokens: stats.unattributedTokens,
      unknowableTotalTokens: stats.unknowableTotalTokens,
      unattributedHistogram: stats.unattributedHistogram,
    })}`
  );

  if (dbRows.length === 0) {
    return [];
  }

  return rankCandidates(rows, {
    siteTokens: toNumber(dbRows[0].site_tokens),
    medianTokens: toNumber(dbRows[0].median_tokens),
  });
}
