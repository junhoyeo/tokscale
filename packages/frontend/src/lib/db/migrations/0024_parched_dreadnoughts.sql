CREATE TABLE "ratchet_census_work" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"submission_id" uuid NOT NULL,
	"submitted_device_id" uuid NOT NULL,
	"buckets" jsonb NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "ratchet_census_work" ADD CONSTRAINT "ratchet_census_work_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "ratchet_census_work" ADD CONSTRAINT "ratchet_census_work_submitted_device_id_submitted_devices_id_fk" FOREIGN KEY ("submitted_device_id") REFERENCES "public"."submitted_devices"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "idx_ratchet_census_work_submission_id" ON "ratchet_census_work" USING btree ("submission_id");--> statement-breakpoint
ALTER TABLE "submissions" DROP COLUMN "ratchet_census_pending";