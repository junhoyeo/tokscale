ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_submission_id_date_key" UNIQUE ("submission_id", "date");
