-- Migration: rehash any remaining plaintext personal API tokens.
--
-- Pre-#512, the api_tokens.token column stored the raw "tt_<hex>" token
-- as issued. PR #512 introduced SHA-256-at-rest hashing for all NEW
-- tokens and a transitional OR-clause in `authenticatePersonalToken`
-- that auto-rehashed legacy plaintext rows on next use. Rows whose
-- owners never re-authenticated still sit in the DB as plaintext, which
-- means a DB read leaks usable tokens. This migration finishes the
-- transition by rehashing every remaining plaintext row in one shot,
-- so the application code can drop the OR-clause and the
-- rehash-on-use path entirely (paired commit).
--
-- Identification: SHA-256 hex output is `[0-9a-f]{64}` and can never
-- produce the substring "tt_". Any row whose token still starts with
-- "tt_" is definitionally plaintext. Hashed rows are skipped.
--
-- Idempotent: re-running this migration after every plaintext row has
-- already been upgraded simply matches zero rows.
--
-- Note: pgcrypto is required for `digest(text, 'sha256')`. The
-- extension is widely available on managed Postgres (Supabase, Neon,
-- RDS, etc.) and is enabled here defensively. If the deployment lacks
-- it the migration will fail at the CREATE EXTENSION step rather than
-- silently leave plaintext rows behind.

CREATE EXTENSION IF NOT EXISTS pgcrypto;
--> statement-breakpoint
UPDATE "api_tokens"
SET "token" = encode(digest("token", 'sha256'), 'hex')
WHERE LEFT("token", 3) = 'tt_';
