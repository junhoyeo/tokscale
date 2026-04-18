ALTER TABLE "submission_reviews" ADD COLUMN "reviewed_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "submission_reviews" ADD COLUMN "reviewed_by_username" varchar(39);--> statement-breakpoint
ALTER TABLE "submission_reviews" ADD COLUMN "review_note" text;