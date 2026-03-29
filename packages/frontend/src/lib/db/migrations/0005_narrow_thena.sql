ALTER TABLE "submissions" DROP CONSTRAINT "submissions_user_id_unique";--> statement-breakpoint
ALTER TABLE "submissions" DROP CONSTRAINT "submissions_user_hash_unique";--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "source_id" varchar(255);--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "source_name" varchar(255);--> statement-breakpoint
CREATE INDEX "idx_submissions_user_source" ON "submissions" USING btree ("user_id","source_id");--> statement-breakpoint
CREATE UNIQUE INDEX "submissions_user_unsourced_unique" ON "submissions" USING btree ("user_id") WHERE "submissions"."source_id" is null;--> statement-breakpoint
CREATE UNIQUE INDEX "submissions_user_source_unique" ON "submissions" USING btree ("user_id","source_id") WHERE "submissions"."source_id" is not null;--> statement-breakpoint
CREATE UNIQUE INDEX "submissions_user_unsourced_hash_unique" ON "submissions" USING btree ("user_id","submission_hash") WHERE "submissions"."submission_hash" is not null and "submissions"."source_id" is null;--> statement-breakpoint
CREATE UNIQUE INDEX "submissions_user_source_hash_unique" ON "submissions" USING btree ("user_id","source_id","submission_hash") WHERE "submissions"."submission_hash" is not null and "submissions"."source_id" is not null;
