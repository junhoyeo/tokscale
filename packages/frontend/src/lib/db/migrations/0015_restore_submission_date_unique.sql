-- Migration 0007 deliberately dropped the (submission_id, date) unique
-- constraint in favor of (submission_id, submitted_device_id, date) so that
-- multi-device users could hold one daily_breakdown row per device per day.
-- That means production can already contain multiple rows sharing the same
-- (submission_id, date) across different devices, so a bare
-- ADD CONSTRAINT UNIQUE (submission_id, date) would abort at deploy time.
--
-- Before restoring the (submission_id, date) uniqueness, merge any existing
-- duplicate (submission_id, date) rows into a single row and delete the
-- rest. The merge is per-client-key aware, not a blind sum of row totals:
-- the PR's core scenario is a DEVICE CHANGE, where the same local history is
-- resubmitted under a new device id, so two duplicate rows commonly carry
-- the *same* client keys describing the *same* underlying usage. Summing
-- those would double the user's daily usage. Instead, within
-- source_breakdown, a client key that appears in more than one row keeps
-- only ONE row's version of that client -- preferring the most-recently-
-- submitting device's snapshot UNLESS it would regress (lower token total)
-- relative to another duplicate's snapshot of the same client, in which
-- case the larger-token version is kept. This mirrors the runtime's
-- mergeClientBreakdownsWithRegressionGuard (lib/db/helpers.ts), which
-- likewise refuses to let a fresher-but-smaller snapshot silently erase a
-- larger one, e.g. an incomplete newer local history export. A client key
-- that appears in exactly one row (a genuine two-machine duplicate with
-- distinct client usage) is kept as-is.
--
-- "kilocode" is a legacy alias of "kilo" (see LEGACY_CLIENT_ALIASES in
-- lib/validation/submission.ts and lib/publicProfileData.ts): older CLI
-- versions wrote source_breakdown entries keyed "kilocode" for the same
-- client newer versions key "kilo". Both keys are normalized to "kilo"
-- before folding, so a pre-alias duplicate and a post-alias duplicate of
-- the same underlying usage collide onto one key (subject to the same
-- newest-wins-unless-regression rule above) instead of being summed as if
-- they were two disjoint clients.
--
-- A duplicate row can also have source_breakdown = NULL: this is legacy
-- schemaVersion 0/1 data written before the CLI tracked a per-client
-- breakdown at all -- the row only carries a row-level total (tokens,
-- cost, input_tokens, output_tokens). It has no client key to fold into
-- source_breakdown, but its usage must never be silently dropped just
-- because it lacks JSON attribution. It is folded in as an anonymous
-- "pseudo-client" contribution using the same newest-wins-unless-regression
-- rule (compared only against other NULL-breakdown duplicates for that
-- day, since there's no way to tell whether it overlaps with any specific
-- real client's usage), added into the row's numeric totals but kept out
-- of the persisted source_breakdown JSON since it has no real client
-- identity to attribute it to. If every duplicate for a day was
-- NULL-breakdown, the merged row's source_breakdown stays NULL, preserving
-- the original "row-total-only, no per-client attribution" semantics.
--
-- Row totals (tokens, cost, input, output) are recomputed from the merged
-- per-client breakdown (plus any legacy pseudo-client contribution) rather
-- than summed across rows, so they can never double-count a device-change
-- duplicate.
--
-- active_time_ms is tracked at the row level only (there is no per-client
-- active-time breakdown), so duplicate rows cannot be disambiguated into
-- "overlapping" vs. "genuinely additional" active time the way per-client
-- token data can. Unconditionally summing it across duplicates would
-- always double it for this migration's core device-change-duplicate
-- scenario, so the merge takes the MAX active_time_ms across duplicates
-- instead -- a safe value that never fabricates time beyond what any
-- single duplicate actually reported for the day, and never drops it
-- either.
--
-- Because this merge can shrink a submission's daily_breakdown rows (and
-- therefore its true totals), any submissions that had at least one
-- duplicate merged have their denormalized `submissions` totals (total
-- tokens/cost/input/output, cache/reasoning tokens, date range, active
-- time, sources_used, models_used) recomputed from the now-deduplicated
-- daily_breakdown rows at the end of this migration, mirroring the
-- aggregation route.ts performs on every submit (STEP 3d/3e in
-- app/api/submit/route.ts). Without this, a duplicate's tokens that were
-- already counted into `submissions.total_tokens` at submit time would
-- stay inflated forever for a user who never submits again.
--
-- Finally, an old-version CLI/writer could otherwise insert a new
-- (submission_id, date) duplicate between this migration's scan and the
-- ADD CONSTRAINT below, aborting the migration. LOCK TABLE daily_breakdown
-- IN SHARE ROW EXCLUSIVE MODE at the top of this transaction blocks any
-- concurrent WRITE to daily_breakdown -- INSERT/UPDATE/DELETE, including
-- the submit route's raw INSERT ... ON CONFLICT / UPDATE statements, all of
-- which need at least ROW EXCLUSIVE -- until this transaction commits.
-- (Verified: SHARE ROW EXCLUSIVE does NOT block ACCESS SHARE (plain reads,
-- e.g. leaderboard/profile pages) or ROW SHARE (SELECT ... FOR UPDATE, used
-- by the submit route to lock candidate rows before deciding whether to
-- insert or update); it only blocks the actual write once one is attempted,
-- which is exactly what closes the race here -- a FOR UPDATE read alone
-- cannot create a duplicate row.) The stronger EXCLUSIVE MODE was
-- considered but rejected: it would additionally block SELECT ... FOR
-- UPDATE/FOR SHARE reads (needless extra blocking of the submit route's
-- read step) without closing any race that SHARE ROW EXCLUSIVE doesn't
-- already close, since the write itself is what must be serialized against.
LOCK TABLE daily_breakdown IN SHARE ROW EXCLUSIVE MODE;
--> statement-breakpoint
-- Tracks which submissions actually had a duplicate merged, so the totals
-- recompute at the end of this migration only touches submissions whose
-- denormalized totals could actually have gone stale.
DROP TABLE IF EXISTS affected_submissions_0015;
--> statement-breakpoint
CREATE TEMP TABLE affected_submissions_0015 (submission_id uuid PRIMARY KEY);
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

    INSERT INTO affected_submissions_0015 (submission_id)
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
        -- Legacy pseudo-client contribution (see header comment). Only
        -- overridden by another NULL-breakdown duplicate with a strictly
        -- larger row total, same regression-guard rule as real client keys.
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
-- The (submission_id, submitted_device_id, date) constraint is strictly
-- subsumed by the new (submission_id, date) constraint below, so drop it.
ALTER TABLE "daily_breakdown" DROP CONSTRAINT IF EXISTS "daily_breakdown_submission_device_date_unique";
--> statement-breakpoint
-- Guarded so this migration is safe to re-run (e.g. if it partially applied
-- before a deploy retry).
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'daily_breakdown_submission_id_date_key'
  ) THEN
    ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_submission_id_date_key" UNIQUE ("submission_id", "date");
  END IF;
END $$;
--> statement-breakpoint
-- Recompute denormalized `submissions` totals for every submission that had
-- at least one duplicate merged above (see header comment). On a re-run
-- where no duplicates remain, affected_submissions_0015 is empty and this
-- is a no-op, keeping the migration idempotent.
WITH client_rows AS (
  SELECT
    db.submission_id,
    CASE WHEN c.key = 'kilocode' THEN 'kilo' ELSE c.key END AS client_name,
    c.value AS client_value
  FROM daily_breakdown db
  CROSS JOIN LATERAL jsonb_each(COALESCE(db.source_breakdown, '{}'::jsonb)) AS c(key, value)
  WHERE db.submission_id IN (SELECT submission_id FROM affected_submissions_0015)
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
  WHERE db.submission_id IN (SELECT submission_id FROM affected_submissions_0015)
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
DROP TABLE IF EXISTS affected_submissions_0015;
