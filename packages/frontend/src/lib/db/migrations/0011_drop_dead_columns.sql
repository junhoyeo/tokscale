-- Drop columns and indexes confirmed dead by full-codebase audit on 2026-05-25.
--
-- daily_breakdown.provider_breakdown: declared in schema.ts but never read
--   or written anywhere in src/. Pure dead JSONB column allocated on every
--   daily row.
--
-- daily_breakdown.model_breakdown: written on every submit, but the only
--   SELECT (users/[username]/route.ts:110) places it in the result row and
--   never references the value. Net result: storage + JSON serialization
--   churn with no consumer.
--
-- submissions.status + idx_submissions_status: column is only ever written
--   as 'verified' on insert (submit/route.ts:205); zero WHERE filters exist
--   anywhere in the codebase. The index serves no query. Status semantics
--   were planned ("pending"/"rejected"?) but never implemented.
--
-- users.is_admin: column is set/fetched into SessionUser, but no admin
--   gate exists in the codebase. A user with is_admin=true had zero
--   additional privileges. The column lied about what it did; if admin
--   features ever ship, reintroduce the column WITH an actual gate.
--
-- All DROPs use IF EXISTS so the migration is safe to replay against any
-- environment regardless of whether the column was already removed
-- out-of-band.

DROP INDEX IF EXISTS "idx_submissions_status";--> statement-breakpoint
ALTER TABLE "submissions" DROP COLUMN IF EXISTS "status";--> statement-breakpoint
ALTER TABLE "daily_breakdown" DROP COLUMN IF EXISTS "provider_breakdown";--> statement-breakpoint
ALTER TABLE "daily_breakdown" DROP COLUMN IF EXISTS "model_breakdown";--> statement-breakpoint
ALTER TABLE "users" DROP COLUMN IF EXISTS "is_admin";
