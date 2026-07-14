ALTER TABLE "submissions" ADD COLUMN "metadata_received_at" timestamp with time zone;--> statement-breakpoint
UPDATE "submissions" SET "metadata_received_at" = "updated_at";--> statement-breakpoint
ALTER TABLE "submissions" ALTER COLUMN "metadata_received_at" SET DEFAULT now();--> statement-breakpoint
ALTER TABLE "submissions" ALTER COLUMN "metadata_received_at" SET NOT NULL;--> statement-breakpoint
ALTER TABLE "submission_reviews" ADD COLUMN "reviewed_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "submission_reviews" ADD COLUMN "reviewed_by_username" varchar(39);--> statement-breakpoint
ALTER TABLE "submission_reviews" ADD COLUMN "review_note" text;