ALTER TABLE "submitted_devices" ADD COLUMN "total_active_time_ms" bigint;--> statement-breakpoint
ALTER TABLE "submitted_devices" ADD COLUMN "longest_continuous_ms" bigint;--> statement-breakpoint
ALTER TABLE "submitted_devices" ADD COLUMN "max_concurrent_sessions" integer;--> statement-breakpoint
ALTER TABLE "submitted_devices" ADD COLUMN "session_count" integer;