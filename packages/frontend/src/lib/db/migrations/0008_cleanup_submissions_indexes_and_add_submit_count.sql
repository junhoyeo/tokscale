-- Schema cleanup surfaced by the prod-DB audit during PR #389 review.
--
-- Index churn: at the time of this migration, pg_stat_user_indexes showed
-- these as having essentially zero scans in production:
--   idx_submissions_user_id        214    (redundant — idx_submissions_leaderboard
--                                          starts with user_id so it serves every
--                                          plain user_id lookup as a left-prefix)
--   idx_submissions_status           1
--   idx_submissions_total_tokens     0
--   idx_submissions_date_range       0
-- vs idx_submissions_leaderboard     3_270_000  (by far the hottest)
-- Dropping the four unused / redundant ones reduces INSERT/UPDATE overhead
-- on every submit without losing any query path.
--
-- FK coverage: device_codes.user_id was the only FK column in the schema
-- without a covering index, so cascade-delete on a user does a seq scan
-- of device_codes. Tiny today, free to fix once.
--
-- submit_count safety net: this column exists in prod (added some time ago
-- via `drizzle-kit push` which writes directly from schema.ts) but has NO
-- corresponding ALTER TABLE statement in any earlier .sql migration file,
-- so a fresh `drizzle-kit migrate` replay from 0000..0005 would end up
-- without the column. Adding it here with IF NOT EXISTS is a no-op on
-- prod and a correctness fix on any fresh restore.

DROP INDEX IF EXISTS "idx_submissions_user_id";--> statement-breakpoint
DROP INDEX IF EXISTS "idx_submissions_status";--> statement-breakpoint
DROP INDEX IF EXISTS "idx_submissions_total_tokens";--> statement-breakpoint
DROP INDEX IF EXISTS "idx_submissions_date_range";--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_device_codes_user_id" ON "device_codes" USING btree ("user_id");--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN IF NOT EXISTS "submit_count" integer DEFAULT 1 NOT NULL;
