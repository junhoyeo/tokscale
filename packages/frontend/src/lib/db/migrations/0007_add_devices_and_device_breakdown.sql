-- Multi-device usage aggregation: a `devices` table plus a `device_id`
-- dimension on daily_breakdown. Existing rows are backfilled into a per-user
-- "legacy" device, which is also the fallback for pre-device-aware CLIs.

-- 1. Devices table -----------------------------------------------------------
CREATE TABLE IF NOT EXISTS "devices" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid NOT NULL,
	"device_id" varchar(64) NOT NULL,
	"name" varchar(100) NOT NULL,
	"hostname" varchar(255),
	"os" varchar(32),
	"cli_version" varchar(20),
	"last_seen_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "devices_user_device_unique" UNIQUE("user_id","device_id")
);
--> statement-breakpoint
DO $$ BEGIN
	ALTER TABLE "devices" ADD CONSTRAINT "devices_user_id_users_id_fk"
		FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;
EXCEPTION
	WHEN duplicate_object THEN null;
END $$;
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_devices_user_id" ON "devices" USING btree ("user_id");
--> statement-breakpoint

-- 2. Add device_id to daily_breakdown (nullable during backfill) -------------
ALTER TABLE "daily_breakdown" ADD COLUMN IF NOT EXISTS "device_id" uuid;
--> statement-breakpoint

-- 3. One "legacy" device per user that already has a submission --------------
INSERT INTO "devices" ("user_id", "device_id", "name", "created_at", "updated_at")
SELECT DISTINCT s."user_id", 'legacy', 'Legacy device', now(), now()
FROM "submissions" s
ON CONFLICT ("user_id","device_id") DO NOTHING;
--> statement-breakpoint

-- 4. Backfill daily_breakdown.device_id from the owner's legacy device -------
UPDATE "daily_breakdown" d
SET "device_id" = dev."id"
FROM "submissions" s
JOIN "devices" dev ON dev."user_id" = s."user_id" AND dev."device_id" = 'legacy'
WHERE d."submission_id" = s."id" AND d."device_id" IS NULL;
--> statement-breakpoint

-- 5. Enforce NOT NULL now that every row is backfilled ----------------------
ALTER TABLE "daily_breakdown" ALTER COLUMN "device_id" SET NOT NULL;
--> statement-breakpoint
DO $$ BEGIN
	ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_device_id_devices_id_fk"
		FOREIGN KEY ("device_id") REFERENCES "public"."devices"("id") ON DELETE cascade ON UPDATE no action;
EXCEPTION
	WHEN duplicate_object THEN null;
END $$;
--> statement-breakpoint

-- 6. Replace (submission_id, date) unique with (submission_id, device_id, date)
ALTER TABLE "daily_breakdown" DROP CONSTRAINT IF EXISTS "daily_breakdown_submission_date_unique";
--> statement-breakpoint
ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_submission_device_date_unique"
	UNIQUE("submission_id","device_id","date");
--> statement-breakpoint

-- 7. Index for device-scoped queries ---------------------------------------
CREATE INDEX IF NOT EXISTS "idx_daily_breakdown_device_id" ON "daily_breakdown" USING btree ("device_id");
