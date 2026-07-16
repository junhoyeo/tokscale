-- Migration 0007 deliberately dropped the (submission_id, date) unique
-- constraint in favor of (submission_id, submitted_device_id, date) so that
-- multi-device users could hold one daily_breakdown row per device per day.
-- That means production can already contain multiple rows sharing the same
-- (submission_id, date) across different devices, so a bare
-- ADD CONSTRAINT UNIQUE (submission_id, date) would abort at deploy time.
--
-- Before restoring the (submission_id, date) uniqueness, merge any existing
-- duplicate (submission_id, date) rows into a single row: sum the numeric
-- totals, deep-merge source_breakdown per client/model, keep the row
-- belonging to whichever device most recently submitted, and delete the
-- rest.
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
  existing_client_val JSONB;
  model_key TEXT;
  model_val JSONB;
  existing_model_val JSONB;
BEGIN
  FOR dup IN
    SELECT submission_id, date
    FROM daily_breakdown
    GROUP BY submission_id, date
    HAVING COUNT(*) > 1
  LOOP
    merged_source := '{}'::jsonb;
    merged_tokens := 0;
    merged_cost := 0;
    merged_input := 0;
    merged_output := 0;
    merged_timestamp_ms := NULL;
    merged_active_time_ms := NULL;
    keep_id := NULL;
    keep_device_id := NULL;

    -- Rows for this (submission_id, date), most-recently-submitting device
    -- first. The first row visited becomes the row we keep (its id and
    -- submitted_device_id survive); every row's numeric/JSON data is folded
    -- into the merged totals regardless of visit order.
    FOR device_row IN
      SELECT db."id", db."submitted_device_id", db."tokens", db."cost",
             db."input_tokens", db."output_tokens", db."timestamp_ms",
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

      merged_tokens := merged_tokens + COALESCE(device_row."tokens", 0);
      merged_cost := merged_cost + COALESCE(device_row."cost", 0);
      merged_input := merged_input + COALESCE(device_row."input_tokens", 0);
      merged_output := merged_output + COALESCE(device_row."output_tokens", 0);
      merged_active_time_ms := COALESCE(merged_active_time_ms, 0) + COALESCE(device_row."active_time_ms", 0);
      merged_timestamp_ms := LEAST(
        COALESCE(merged_timestamp_ms, device_row."timestamp_ms"),
        COALESCE(device_row."timestamp_ms", merged_timestamp_ms)
      );

      IF device_row."source_breakdown" IS NOT NULL THEN
        FOR client_key, client_val IN
          SELECT key, value FROM jsonb_each(device_row."source_breakdown")
        LOOP
          existing_client_val := merged_source -> client_key;

          IF existing_client_val IS NULL THEN
            merged_source := jsonb_set(merged_source, ARRAY[client_key], client_val, true);
          ELSE
            existing_client_val := jsonb_set(existing_client_val, '{tokens}',
              to_jsonb(COALESCE((existing_client_val->>'tokens')::numeric, 0) + COALESCE((client_val->>'tokens')::numeric, 0)));
            existing_client_val := jsonb_set(existing_client_val, '{cost}',
              to_jsonb(COALESCE((existing_client_val->>'cost')::numeric, 0) + COALESCE((client_val->>'cost')::numeric, 0)));
            existing_client_val := jsonb_set(existing_client_val, '{input}',
              to_jsonb(COALESCE((existing_client_val->>'input')::numeric, 0) + COALESCE((client_val->>'input')::numeric, 0)));
            existing_client_val := jsonb_set(existing_client_val, '{output}',
              to_jsonb(COALESCE((existing_client_val->>'output')::numeric, 0) + COALESCE((client_val->>'output')::numeric, 0)));
            existing_client_val := jsonb_set(existing_client_val, '{cacheRead}',
              to_jsonb(COALESCE((existing_client_val->>'cacheRead')::numeric, 0) + COALESCE((client_val->>'cacheRead')::numeric, 0)));
            existing_client_val := jsonb_set(existing_client_val, '{cacheWrite}',
              to_jsonb(COALESCE((existing_client_val->>'cacheWrite')::numeric, 0) + COALESCE((client_val->>'cacheWrite')::numeric, 0)));
            existing_client_val := jsonb_set(existing_client_val, '{reasoning}',
              to_jsonb(COALESCE((existing_client_val->>'reasoning')::numeric, 0) + COALESCE((client_val->>'reasoning')::numeric, 0)));
            existing_client_val := jsonb_set(existing_client_val, '{messages}',
              to_jsonb(COALESCE((existing_client_val->>'messages')::numeric, 0) + COALESCE((client_val->>'messages')::numeric, 0)));

            IF client_val ? 'models' THEN
              FOR model_key, model_val IN
                SELECT key, value FROM jsonb_each(client_val -> 'models')
              LOOP
                existing_model_val := existing_client_val #> ARRAY['models', model_key];

                IF existing_model_val IS NULL THEN
                  existing_client_val := jsonb_set(existing_client_val, ARRAY['models', model_key], model_val, true);
                ELSE
                  existing_model_val := jsonb_set(existing_model_val, '{tokens}',
                    to_jsonb(COALESCE((existing_model_val->>'tokens')::numeric, 0) + COALESCE((model_val->>'tokens')::numeric, 0)));
                  existing_model_val := jsonb_set(existing_model_val, '{cost}',
                    to_jsonb(COALESCE((existing_model_val->>'cost')::numeric, 0) + COALESCE((model_val->>'cost')::numeric, 0)));
                  existing_model_val := jsonb_set(existing_model_val, '{input}',
                    to_jsonb(COALESCE((existing_model_val->>'input')::numeric, 0) + COALESCE((model_val->>'input')::numeric, 0)));
                  existing_model_val := jsonb_set(existing_model_val, '{output}',
                    to_jsonb(COALESCE((existing_model_val->>'output')::numeric, 0) + COALESCE((model_val->>'output')::numeric, 0)));
                  existing_model_val := jsonb_set(existing_model_val, '{cacheRead}',
                    to_jsonb(COALESCE((existing_model_val->>'cacheRead')::numeric, 0) + COALESCE((model_val->>'cacheRead')::numeric, 0)));
                  existing_model_val := jsonb_set(existing_model_val, '{cacheWrite}',
                    to_jsonb(COALESCE((existing_model_val->>'cacheWrite')::numeric, 0) + COALESCE((model_val->>'cacheWrite')::numeric, 0)));
                  existing_model_val := jsonb_set(existing_model_val, '{reasoning}',
                    to_jsonb(COALESCE((existing_model_val->>'reasoning')::numeric, 0) + COALESCE((model_val->>'reasoning')::numeric, 0)));
                  existing_model_val := jsonb_set(existing_model_val, '{messages}',
                    to_jsonb(COALESCE((existing_model_val->>'messages')::numeric, 0) + COALESCE((model_val->>'messages')::numeric, 0)));

                  existing_client_val := jsonb_set(existing_client_val, ARRAY['models', model_key], existing_model_val, true);
                END IF;
              END LOOP;
            END IF;

            merged_source := jsonb_set(merged_source, ARRAY[client_key], existing_client_val, true);
          END IF;
        END LOOP;
      END IF;
    END LOOP;

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
