ALTER TABLE "groups" ADD COLUMN "time_zone" varchar(100) DEFAULT 'UTC' NOT NULL;--> statement-breakpoint
ALTER TABLE "groups" ADD COLUMN "week_starts_on" integer DEFAULT 1 NOT NULL;