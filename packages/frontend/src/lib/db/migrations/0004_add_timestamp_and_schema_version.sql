ALTER TABLE "daily_breakdown" ADD COLUMN "timestamp_ms" bigint;
--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "schema_version" integer DEFAULT 0 NOT NULL;
