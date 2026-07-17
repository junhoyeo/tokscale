ALTER TABLE "daily_breakdown" DROP CONSTRAINT IF EXISTS "daily_breakdown_submission_id_date_key";
--> statement-breakpoint
ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_submission_device_date_unique" UNIQUE("submission_id","submitted_device_id","date");
