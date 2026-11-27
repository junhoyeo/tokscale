CREATE TABLE "submission_daily_counts" (
	"submission_id" uuid NOT NULL,
	"date" date NOT NULL,
	"submit_count" integer DEFAULT 1 NOT NULL,
	CONSTRAINT "submission_daily_counts_submission_id_date_pk" PRIMARY KEY("submission_id","date"),
	CONSTRAINT "submission_daily_counts_positive" CHECK ("submission_daily_counts"."submit_count" > 0)
);
--> statement-breakpoint
ALTER TABLE "submission_daily_counts" ADD CONSTRAINT "submission_daily_counts_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;