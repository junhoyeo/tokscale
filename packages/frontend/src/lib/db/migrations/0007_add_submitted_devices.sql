CREATE TABLE "submitted_devices" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid NOT NULL,
	"device_key" varchar(96) NOT NULL,
	"display_name" varchar(120),
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"last_submitted_at" timestamp with time zone,
	CONSTRAINT "submitted_devices_user_device_key_unique" UNIQUE("user_id","device_key")
);
--> statement-breakpoint
ALTER TABLE "submitted_devices" ADD CONSTRAINT "submitted_devices_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
CREATE INDEX "idx_submitted_devices_user_id" ON "submitted_devices" USING btree ("user_id");
--> statement-breakpoint
ALTER TABLE "daily_breakdown" ADD COLUMN "submitted_device_id" uuid;
--> statement-breakpoint
INSERT INTO "submitted_devices" (
	"user_id",
	"device_key",
	"display_name",
	"last_submitted_at",
	"updated_at"
)
SELECT
	"user_id",
	'legacy-default',
	'Legacy submissions',
	MAX("updated_at"),
	MAX("updated_at")
FROM "submissions"
GROUP BY "user_id"
ON CONFLICT ("user_id", "device_key") DO NOTHING;
--> statement-breakpoint
UPDATE "daily_breakdown" AS db
SET "submitted_device_id" = sd."id"
FROM "submissions" AS s
INNER JOIN "submitted_devices" AS sd
	ON sd."user_id" = s."user_id"
	AND sd."device_key" = 'legacy-default'
WHERE db."submission_id" = s."id"
	AND db."submitted_device_id" IS NULL;
--> statement-breakpoint
ALTER TABLE "daily_breakdown" ALTER COLUMN "submitted_device_id" SET NOT NULL;
--> statement-breakpoint
ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_submitted_device_id_submitted_devices_id_fk" FOREIGN KEY ("submitted_device_id") REFERENCES "public"."submitted_devices"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
CREATE INDEX "idx_daily_breakdown_submitted_device_id" ON "daily_breakdown" USING btree ("submitted_device_id");
--> statement-breakpoint
ALTER TABLE "daily_breakdown" DROP CONSTRAINT "daily_breakdown_submission_date_unique";
--> statement-breakpoint
ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_submission_device_date_unique" UNIQUE("submission_id","submitted_device_id","date");
