ALTER TABLE "submissions" DROP CONSTRAINT "submissions_user_id_unique";--> statement-breakpoint
ALTER TABLE "submissions" DROP CONSTRAINT "submissions_user_hash_unique";--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "source_id" varchar(255);--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "source_name" varchar(255);--> statement-breakpoint
CREATE UNIQUE INDEX "submissions_user_unsourced_unique" ON "submissions" USING btree ("user_id") WHERE "submissions"."source_id" is null;--> statement-breakpoint
ALTER TABLE "submissions" ADD CONSTRAINT "submissions_user_source_unique" UNIQUE("user_id","source_id");
