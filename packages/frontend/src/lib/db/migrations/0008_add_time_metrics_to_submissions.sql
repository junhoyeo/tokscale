ALTER TABLE "submissions" ADD COLUMN "total_active_time_ms" bigint;
ALTER TABLE "submissions" ADD COLUMN "longest_continuous_ms" bigint;
ALTER TABLE "submissions" ADD COLUMN "max_concurrent_sessions" integer;
ALTER TABLE "submissions" ADD COLUMN "session_count" integer;
ALTER TABLE "daily_breakdown" ADD COLUMN "active_time_ms" bigint;
