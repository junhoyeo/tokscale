ALTER TABLE "submissions" ALTER COLUMN "total_cost" SET DATA TYPE numeric(18, 4);--> statement-breakpoint
ALTER TABLE "daily_breakdown" ALTER COLUMN "cost" SET DATA TYPE numeric(14, 4);
