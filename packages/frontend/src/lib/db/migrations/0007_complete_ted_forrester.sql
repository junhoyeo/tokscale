-- Must run atomically as one transaction. We intentionally do NOT use
-- CREATE INDEX CONCURRENTLY here: while the old "one row per user"
-- uniqueness is dropped and before the new partial unique index +
-- (user_id, source_id) composite constraint are in place, concurrent
-- submits from the same user could insert duplicate rows that the new
-- invariants would then reject.
ALTER TABLE "submissions" DROP CONSTRAINT "submissions_user_id_unique";--> statement-breakpoint
ALTER TABLE "submissions" DROP CONSTRAINT "submissions_user_hash_unique";--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "source_id" varchar(255);--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN "source_name" varchar(255);--> statement-breakpoint
CREATE UNIQUE INDEX "submissions_user_unsourced_unique" ON "submissions" USING btree ("user_id") WHERE "submissions"."source_id" is null;--> statement-breakpoint
ALTER TABLE "submissions" ADD CONSTRAINT "submissions_user_source_unique" UNIQUE("user_id","source_id");--> statement-breakpoint
-- submission_hash was originally a dedup key via submissions_user_hash_unique
-- (dropped above). Source-scoped submissions can legitimately share the same
-- client/date fingerprint across machines, so the hash no longer has a role
-- and nothing reads it. Drop the column to stop writing dead data.
ALTER TABLE "submissions" DROP COLUMN "submission_hash";
