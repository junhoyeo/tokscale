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
-- only the newest-submitting device's version of that client (the same
-- "latest snapshot wins" semantics the route's
-- mergeClientBreakdownsWithRegressionGuard applies to same-device
-- resubmits). A client key that appears in exactly one row (a genuine
-- two-machine duplicate with distinct client usage) is kept as-is. Row
-- totals (tokens, cost, input, output) are then recomputed from the merged
-- per-client breakdown rather than summed across rows, so they can never
-- double-count a device-change duplicate.
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
  keep_id UUID;
  keep_device_id UUID;
  client_key TEXT;
  client_val JSONB;
BEGIN
  FOR dup IN
    SELECT submission_id, date
    FROM daily_breakdown
    GROUP BY submission_id, date
    HAVING COUNT(*) > 1
  LOOP
    merged_source := '{}'::jsonb;
    merged_timestamp_ms := NULL;
    merged_active_time_ms := NULL;
    keep_id := NULL;
    keep_device_id := NULL;

    -- Rows for this (submission_id, date), most-recently-submitting device
    -- first. The first row visited becomes the row we keep (its id and
    -- submitted_device_id survive). Client keys are folded in newest-first:
    -- the first (newest) occurrence of a given client key wins outright, and
    -- later (older) occurrences of the same key are skipped rather than
    -- summed in.
    FOR device_row IN
      SELECT db."id", db."submitted_device_id", db."timestamp_ms",
             db."active_time_ms", db."source_breakdown"
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

      merged_active_time_ms := COALESCE(merged_active_time_ms, 0) + COALESCE(device_row."active_time_ms", 0);
      merged_timestamp_ms := LEAST(
        COALESCE(merged_timestamp_ms, device_row."timestamp_ms"),
        COALESCE(device_row."timestamp_ms", merged_timestamp_ms)
      );

      IF device_row."source_breakdown" IS NOT NULL THEN
        FOR client_key, client_val IN
          SELECT key, value FROM jsonb_each(device_row."source_breakdown")
        LOOP
          IF NOT (merged_source ? client_key) THEN
            merged_source := jsonb_set(merged_source, ARRAY[client_key], client_val, true);
          END IF;
        END LOOP;
      END IF;
    END LOOP;

    -- Recompute row totals from the merged per-client breakdown rather than
    -- summing row totals across duplicate rows.
    SELECT
      COALESCE(SUM((value->>'tokens')::numeric), 0),
      COALESCE(SUM((value->>'cost')::numeric), 0),
      COALESCE(SUM((value->>'input')::numeric), 0),
      COALESCE(SUM((value->>'output')::numeric), 0)
    INTO merged_tokens, merged_cost, merged_input, merged_output
    FROM jsonb_each(merged_source);

    UPDATE daily_breakdown
    SET "submitted_device_id" = keep_device_id,
        "tokens" = merged_tokens,
        "cost" = merged_cost,
        "input_tokens" = merged_input,
        "output_tokens" = merged_output,
        "timestamp_ms" = merged_timestamp_ms,
        "active_time_ms" = merged_active_time_ms,
        "source_breakdown" = merged_source
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
