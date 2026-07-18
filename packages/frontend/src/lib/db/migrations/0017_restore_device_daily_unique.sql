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
CREATE UNIQUE INDEX IF NOT EXISTS "daily_breakdown_submission_device_date_unique"
  ON "daily_breakdown" ("submission_id", "submitted_device_id", "date");
--> statement-breakpoint
ALTER TABLE "daily_breakdown"
  DROP CONSTRAINT IF EXISTS "daily_breakdown_submission_id_date_key";
--> statement-breakpoint
ALTER TABLE "daily_breakdown"
  ADD CONSTRAINT "daily_breakdown_submission_device_date_unique"
  UNIQUE USING INDEX "daily_breakdown_submission_device_date_unique";
