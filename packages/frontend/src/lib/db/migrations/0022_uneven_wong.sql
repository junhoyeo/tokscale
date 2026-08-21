CREATE TABLE "submitted_device_client_totals" (
	"submitted_device_id" uuid NOT NULL,
	"client" varchar(128) NOT NULL,
	"origin" varchar(16) NOT NULL,
	"bucket_width" varchar(8) NOT NULL,
	"bucket_key" varchar(16) NOT NULL,
	"tokens_highwater" bigint DEFAULT 0 NOT NULL,
	"cost_highwater" numeric(18, 4) DEFAULT '0' NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "submitted_device_client_totals_submitted_device_id_client_origin_bucket_width_bucket_key_pk" PRIMARY KEY("submitted_device_id","client","origin","bucket_width","bucket_key")
);
--> statement-breakpoint
ALTER TABLE "submitted_device_client_totals" ADD CONSTRAINT "submitted_device_client_totals_submitted_device_id_submitted_devices_id_fk" FOREIGN KEY ("submitted_device_id") REFERENCES "public"."submitted_devices"("id") ON DELETE cascade ON UPDATE no action;