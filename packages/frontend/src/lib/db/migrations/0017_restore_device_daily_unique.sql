-- Restore one daily_breakdown row per device and date without holding an
-- ACCESS EXCLUSIVE lock while PostgreSQL builds the replacement index.
--
-- drizzle-kit runs migrations inside a transaction, so CREATE INDEX
-- CONCURRENTLY cannot be used directly here. For a large production table,
-- create the index before deploying this migration with:
--
--   CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS
--     "daily_breakdown_submission_device_date_unique"
--   ON "daily_breakdown" ("submission_id", "submitted_device_id", "date");
--
-- The account/date constraint installed by 0015 guarantees that the wider
-- device/date key is unique while this index is built. The in-migration
-- CREATE is then a no-op when the concurrent pre-deploy step has run, while
-- remaining a safe fallback for fresh or smaller databases.
--
-- A cancelled CREATE INDEX CONCURRENTLY leaves an invalid index behind, and
-- IF NOT EXISTS alone would incorrectly reuse it. Remove any same-named index
-- that is invalid or does not exactly cover the expected key columns before
-- running the fallback build.
DO $$
DECLARE
  index_oid oid;
  index_is_reusable boolean;
BEGIN
  SELECT to_regclass('daily_breakdown_submission_device_date_unique')::oid
    INTO index_oid;

  IF index_oid IS NOT NULL THEN
    SELECT
      i.indisvalid
      AND i.indisready
      AND i.indisunique
      AND i.indpred IS NULL
      AND i.indexprs IS NULL
      AND i.indnkeyatts = 3
      AND i.indnatts = 3
      AND ARRAY(
        SELECT a.attname::text
        FROM unnest(i.indkey::smallint[]) WITH ORDINALITY AS key(attnum, position)
        JOIN pg_attribute a
          ON a.attrelid = i.indrelid
         AND a.attnum = key.attnum
        ORDER BY key.position
      ) = ARRAY['submission_id', 'submitted_device_id', 'date']
      INTO index_is_reusable
      FROM pg_index i
      WHERE i.indexrelid = index_oid
        AND i.indrelid = 'daily_breakdown'::regclass;

    IF index_is_reusable IS NULL THEN
      RAISE EXCEPTION
        'daily_breakdown_submission_device_date_unique exists but is not the expected daily_breakdown index; remove or rename it before retrying migration 0017';
    END IF;

    IF NOT index_is_reusable THEN
      EXECUTE 'DROP INDEX "daily_breakdown_submission_device_date_unique"';
    END IF;
  END IF;
END $$;
--> statement-breakpoint
CREATE UNIQUE INDEX IF NOT EXISTS "daily_breakdown_submission_device_date_unique"
  ON "daily_breakdown" ("submission_id", "submitted_device_id", "date");
--> statement-breakpoint
ALTER TABLE "daily_breakdown"
  DROP CONSTRAINT IF EXISTS "daily_breakdown_submission_id_date_key";
--> statement-breakpoint
ALTER TABLE "daily_breakdown"
  ADD CONSTRAINT "daily_breakdown_submission_device_date_unique"
  UNIQUE USING INDEX "daily_breakdown_submission_device_date_unique";
