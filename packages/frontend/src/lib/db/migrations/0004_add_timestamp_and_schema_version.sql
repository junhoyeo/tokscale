-- Migration: Add timestamp tracking for timezone-aware date display
--
-- Problem: All dates in daily_breakdown are UTC-bucketed. Users in non-UTC
-- timezones see incorrect dates (e.g., KST user's late-night usage shows as
-- previous day). See: https://github.com/junhoyeo/tokscale/issues/145
--
-- Solution: Store the earliest message timestamp per UTC day bucket so the
-- frontend can derive the correct local date for display.
--
-- - daily_breakdown.timestamp_ms: NULL for legacy rows (no original timestamps
--   available), populated on new/re-submitted data via CLI >= 1.3.0
-- - submissions.schema_version: 0=legacy payload, 1=timestamp-aware CLI
--
-- Both columns are backward-compatible: NULL/0 defaults mean existing data
-- is untouched. Migration is progressive — rows are filled when users
-- re-submit with an updated CLI.

ALTER TABLE "daily_breakdown" ADD COLUMN "timestamp_ms" bigint;
--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "schema_version" integer DEFAULT 0 NOT NULL;
