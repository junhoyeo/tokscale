CREATE TABLE "daily_breakdown_reported" (
	"submitted_device_id" uuid NOT NULL,
	"date" date NOT NULL,
	"client" varchar(128) NOT NULL,
	"tokens" bigint NOT NULL,
	"cost" numeric(14, 4) NOT NULL,
	"input" bigint NOT NULL,
	"output" bigint NOT NULL,
	"active_time_ms" bigint,
	"origin" varchar(16) NOT NULL,
	"reported_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "daily_breakdown_reported_submitted_device_id_date_client_pk" PRIMARY KEY("submitted_device_id","date","client")
);
--> statement-breakpoint
ALTER TABLE "daily_breakdown_reported" ADD CONSTRAINT "daily_breakdown_reported_submitted_device_id_submitted_devices_id_fk" FOREIGN KEY ("submitted_device_id") REFERENCES "public"."submitted_devices"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "idx_daily_breakdown_reported_device_date" ON "daily_breakdown_reported" USING btree ("submitted_device_id","date");