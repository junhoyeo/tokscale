-- Existing browser session rows contain raw random tokens. They cannot be
-- converted into hashes after the fact, so expire them and require re-login.
DELETE FROM "sessions";--> statement-breakpoint
DROP INDEX IF EXISTS "idx_sessions_token";--> statement-breakpoint
ALTER TABLE "sessions" RENAME COLUMN "token" TO "token_hash";--> statement-breakpoint
DO $$ BEGIN
  IF EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'sessions_token_unique'
  ) THEN
    ALTER TABLE "sessions" RENAME CONSTRAINT "sessions_token_unique" TO "sessions_token_hash_unique";
  END IF;
END $$;--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_sessions_token_hash" ON "sessions" USING btree ("token_hash");--> statement-breakpoint
