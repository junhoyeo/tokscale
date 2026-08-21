-- Migration 0015 (already released as commit 1356a9a7 and applied to
-- production) locked daily_breakdown IN SHARE ROW EXCLUSIVE MODE and merged
-- every (submission_id, date) duplicate using a per-group PL/pgSQL loop,
-- then added the daily_breakdown_submission_id_date_key UNIQUE constraint
-- that now prevents any new duplicate from ever being inserted again.
--
-- Drizzle tracks applied migrations by journal timestamp order and does NOT
-- re-run a migration whose SQL content changes after it has already been
-- recorded as applied, so 0015 itself must stay byte-identical to what was
-- already released -- any database that already ran it will silently skip
-- edits made to that file. Two operational improvements identified after
-- 0015 shipped therefore live here instead, in a new migration that every
-- database (already-migrated or not) actually executes:
--
--   1. Deadlock-safe lock ordering. 0015 locked only daily_breakdown. The
--      submit route's own transaction (app/api/submit/route.ts) touches
--      submissions before daily_breakdown in every request. This migration
--      locks BOTH tables, submissions before daily_breakdown, IN EXCLUSIVE
--      MODE -- matching the submit route's acquisition order so no
--      lock-order cycle (and therefore no deadlock) can form between this
--      migration's transaction and a concurrent submit request. EXCLUSIVE
--      mode still permits plain reads (leaderboard/profile pages); it only
--      blocks writes and SELECT ... FOR UPDATE/FOR SHARE for the duration
--      of this migration.
--   2. Pre-write backup. Any daily_breakdown row that is still part of a
--      duplicate (submission_id, date) group is copied into
--      daily_breakdown_premigration_0016_backup (a plain, non-temp table)
--      before any UPDATE/DELETE touches it. This table is intentionally NOT
--      dropped by this migration -- drop it manually once the merge has
--      been verified.
--
-- Explicit plan for data 0015 has already deduplicated (the expected case
-- on every database that has already applied 0015, since its own
-- ADD CONSTRAINT daily_breakdown_submission_id_date_key guarantees no two
-- daily_breakdown rows can share a (submission_id, date) pair once it has
-- run): the `HAVING COUNT(*) > 1` scan below finds zero groups, so the
-- backup insert, the merge loop, and the totals recompute at the end of
-- this migration all operate over empty sets. This migration is then a
-- genuine no-op on data for that database -- it only changes lock semantics
-- for its own (brief) execution. The merge logic itself is reused verbatim
-- from 0015 (identical regression-guarded, per-client-key fold; identical
-- kilocode/kilo alias handling; identical legacy NULL-source_breakdown
-- pseudo-client handling; identical active_time_ms MAX fold -- see 0015's
-- own header comment for the full rationale) rather than reimplemented, so
-- it is exercised only as a defense-in-depth safety net for any database
-- where duplicates exist despite the constraint (e.g. a database that has
-- NOT yet applied 0015 and is running it and this migration back-to-back,
-- or a database where the unique constraint was manually dropped, or one
-- restored from a pre-0015 backup).
LOCK TABLE submissions, daily_breakdown IN EXCLUSIVE MODE;
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS daily_breakdown_premigration_0016_backup (
  LIKE daily_breakdown,
  backed_up_at timestamptz NOT NULL DEFAULT now()
);
--> statement-breakpoint
INSERT INTO daily_breakdown_premigration_0016_backup
SELECT db.*, now()
FROM daily_breakdown db
WHERE (db.submission_id, db.date) IN (
  SELECT submission_id, date
  FROM daily_breakdown
  GROUP BY submission_id, date
  HAVING COUNT(*) > 1
);
--> statement-breakpoint
-- Tracks which submissions actually had a duplicate merged, so the totals
-- recompute at the end of this migration only touches submissions whose
-- denormalized totals could actually have gone stale. Qualified to pg_temp
-- so this cleanup can only ever drop a leftover temp table from a prior
-- attempt in the same session -- never a permanent table of the same name.
DROP TABLE IF EXISTS pg_temp.affected_submissions_0016;
--> statement-breakpoint
CREATE TEMP TABLE affected_submissions_0016 (submission_id uuid PRIMARY KEY);
--> statement-breakpoint
DO $$
DECLARE
  dup RECORD;
  device_row RECORD;
  merged_source JSONB;
  merged_tokens NUMERIC;
  merged_cost NUMERIC;
  merged_input NUMERIC;
  merged_output NUMERIC;
  merged_timestamp_ms BIGINT;
  merged_active_time_ms BIGINT;
  active_time_seen BOOLEAN;
  keep_id UUID;
  keep_device_id UUID;
  client_key TEXT;
  client_val JSONB;
  legacy_tokens NUMERIC;
  legacy_cost NUMERIC;
  legacy_input NUMERIC;
  legacy_output NUMERIC;
BEGIN
  FOR dup IN
    SELECT submission_id, date
    FROM daily_breakdown
    GROUP BY submission_id, date
    HAVING COUNT(*) > 1
  LOOP
    merged_source := '{}'::jsonb;
    merged_timestamp_ms := NULL;
    merged_active_time_ms := 0;
    active_time_seen := FALSE;
    keep_id := NULL;
    keep_device_id := NULL;
    legacy_tokens := NULL;
    legacy_cost := NULL;
    legacy_input := NULL;
    legacy_output := NULL;

    INSERT INTO affected_submissions_0016 (submission_id)
    VALUES (dup.submission_id)
    ON CONFLICT DO NOTHING;

    -- Rows for this (submission_id, date), most-recently-submitting device
    -- first. The first row visited becomes the row we keep (its id and
    -- submitted_device_id survive). Client keys (and the legacy NULL
    -- pseudo-client, and active_time_ms) are folded in newest-first: the
    -- first (newest) occurrence of a given key wins by default, but a later
    -- (older) occurrence can still override it under the regression guard
    -- below.
    FOR device_row IN
      SELECT db."id", db."submitted_device_id", db."timestamp_ms",
             db."active_time_ms", db."source_breakdown",
             db."tokens", db."cost", db."input_tokens", db."output_tokens"
      FROM daily_breakdown db
      LEFT JOIN submitted_devices sd ON sd."id" = db."submitted_device_id"
      WHERE db."submission_id" = dup.submission_id
        AND db."date" = dup.date
      ORDER BY sd."last_submitted_at" DESC NULLS LAST, db."id" DESC
    LOOP
      IF keep_id IS NULL THEN
        keep_id := device_row."id";
        keep_device_id := device_row."submitted_device_id";
      END IF;

      IF device_row."active_time_ms" IS NOT NULL THEN
        active_time_seen := TRUE;
        merged_active_time_ms := GREATEST(merged_active_time_ms, device_row."active_time_ms");
      END IF;

      merged_timestamp_ms := LEAST(
        COALESCE(merged_timestamp_ms, device_row."timestamp_ms"),
        COALESCE(device_row."timestamp_ms", merged_timestamp_ms)
      );

      IF device_row."source_breakdown" IS NOT NULL THEN
        FOR client_key, client_val IN
          SELECT key, value FROM jsonb_each(device_row."source_breakdown")
        LOOP
          IF client_key = 'kilocode' THEN
            client_key := 'kilo';
          END IF;

          IF NOT (merged_source ? client_key) THEN
            merged_source := jsonb_set(merged_source, ARRAY[client_key], client_val, true);
          ELSIF COALESCE((client_val->>'tokens')::numeric, 0) >
                COALESCE((merged_source->client_key->>'tokens')::numeric, 0) THEN
            -- Regression guard: prefer the newest row's version of a client
            -- UNLESS it has fewer tokens than another duplicate's version of
            -- the same client, in which case keep the larger one.
            merged_source := jsonb_set(merged_source, ARRAY[client_key], client_val, true);
          END IF;
        END LOOP;
      ELSE
        -- Legacy pseudo-client contribution (see 0015's header comment).
        -- Only overridden by another NULL-breakdown duplicate with a
        -- strictly larger row total, same regression-guard rule as real
        -- client keys.
        IF legacy_tokens IS NULL OR COALESCE(device_row."tokens", 0) > legacy_tokens THEN
          legacy_tokens := COALESCE(device_row."tokens", 0);
          legacy_cost := COALESCE(device_row."cost", 0);
          legacy_input := COALESCE(device_row."input_tokens", 0);
          legacy_output := COALESCE(device_row."output_tokens", 0);
        END IF;
      END IF;
    END LOOP;

    -- Recompute row totals from the merged per-client breakdown rather than
    -- summing row totals across duplicate rows, then add back any legacy
    -- pseudo-client contribution so a NULL-breakdown duplicate's usage is
    -- preserved even though it has no client key of its own.
    SELECT
      COALESCE(SUM((value->>'tokens')::numeric), 0),
      COALESCE(SUM((value->>'cost')::numeric), 0),
      COALESCE(SUM((value->>'input')::numeric), 0),
      COALESCE(SUM((value->>'output')::numeric), 0)
    INTO merged_tokens, merged_cost, merged_input, merged_output
    FROM jsonb_each(merged_source);

    merged_tokens := merged_tokens + COALESCE(legacy_tokens, 0);
    merged_cost := merged_cost + COALESCE(legacy_cost, 0);
    merged_input := merged_input + COALESCE(legacy_input, 0);
    merged_output := merged_output + COALESCE(legacy_output, 0);

    UPDATE daily_breakdown
    SET "submitted_device_id" = keep_device_id,
        "tokens" = merged_tokens,
        "cost" = merged_cost,
        "input_tokens" = merged_input,
        "output_tokens" = merged_output,
        "timestamp_ms" = merged_timestamp_ms,
        "active_time_ms" = CASE WHEN active_time_seen THEN merged_active_time_ms ELSE NULL END,
        "source_breakdown" = CASE
          WHEN merged_source = '{}'::jsonb AND legacy_tokens IS NOT NULL THEN NULL
          ELSE merged_source
        END
    WHERE "id" = keep_id;

    DELETE FROM daily_breakdown
    WHERE "submission_id" = dup.submission_id
      AND "date" = dup.date
      AND "id" <> keep_id;
  END LOOP;
END $$;
--> statement-breakpoint
-- Recompute denormalized `submissions` totals for every submission that had
-- at least one duplicate merged above (see 0015's header comment for why).
-- On the expected case where no duplicates remain, affected_submissions_0016
-- is empty and this is a no-op.
WITH client_rows AS (
  SELECT
    db.submission_id,
    CASE WHEN c.key = 'kilocode' THEN 'kilo' ELSE c.key END AS client_name,
    c.value AS client_value
  FROM daily_breakdown db
  CROSS JOIN LATERAL jsonb_each(COALESCE(db.source_breakdown, '{}'::jsonb)) AS c(key, value)
  WHERE db.submission_id IN (SELECT submission_id FROM affected_submissions_0016)
),
model_rows AS (
  -- Mirrors route.ts STEP 3d: use a client's `models` map when present (even
  -- if empty), falling back to the legacy singular `modelId` field only when
  -- `models` is absent entirely.
  SELECT cr.submission_id, m.key AS model_id
  FROM client_rows cr
  CROSS JOIN LATERAL jsonb_object_keys(COALESCE(cr.client_value->'models', '{}'::jsonb)) AS m(key)
  WHERE cr.client_value ? 'models'
  UNION ALL
  SELECT cr.submission_id, cr.client_value->>'modelId' AS model_id
  FROM client_rows cr
  WHERE NOT (cr.client_value ? 'models') AND cr.client_value ? 'modelId'
),
client_agg AS (
  SELECT
    submission_id,
    COALESCE(SUM((client_value->>'cacheRead')::numeric), 0)::bigint AS cache_read_tokens,
    COALESCE(SUM((client_value->>'cacheWrite')::numeric), 0)::bigint AS cache_creation_tokens,
    COALESCE(SUM((client_value->>'reasoning')::numeric), 0)::bigint AS reasoning_tokens,
    array_agg(DISTINCT client_name) AS sources_used
  FROM client_rows
  GROUP BY submission_id
),
model_agg AS (
  SELECT
    submission_id,
    array_agg(DISTINCT model_id) FILTER (WHERE model_id IS NOT NULL) AS models_used
  FROM model_rows
  GROUP BY submission_id
),
day_agg AS (
  SELECT
    db.submission_id,
    COALESCE(SUM(db.tokens), 0)::bigint AS total_tokens,
    COALESCE(SUM(db.cost), 0) AS total_cost,
    COALESCE(SUM(db.input_tokens), 0)::bigint AS input_tokens,
    COALESCE(SUM(db.output_tokens), 0)::bigint AS output_tokens,
    MIN(db.date) AS date_start,
    MAX(db.date) AS date_end,
    COALESCE(SUM(db.active_time_ms), 0)::bigint AS total_active_time_ms
  FROM daily_breakdown db
  WHERE db.submission_id IN (SELECT submission_id FROM affected_submissions_0016)
  GROUP BY db.submission_id
)
UPDATE submissions s
SET
  total_tokens = da.total_tokens,
  total_cost = da.total_cost,
  input_tokens = da.input_tokens,
  output_tokens = da.output_tokens,
  cache_read_tokens = COALESCE(ca.cache_read_tokens, 0),
  cache_creation_tokens = COALESCE(ca.cache_creation_tokens, 0),
  reasoning_tokens = COALESCE(ca.reasoning_tokens, 0),
  date_start = da.date_start,
  date_end = da.date_end,
  total_active_time_ms = da.total_active_time_ms,
  sources_used = COALESCE(ca.sources_used, '{}'::text[]),
  models_used = COALESCE(ma.models_used, '{}'::text[]),
  updated_at = now()
FROM day_agg da
LEFT JOIN client_agg ca ON ca.submission_id = da.submission_id
LEFT JOIN model_agg ma ON ma.submission_id = da.submission_id
WHERE s.id = da.submission_id;
--> statement-breakpoint
DROP TABLE IF EXISTS pg_temp.affected_submissions_0016;
