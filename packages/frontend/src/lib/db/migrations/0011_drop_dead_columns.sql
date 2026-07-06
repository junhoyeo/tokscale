-- =============================================================================
-- LOCK-WINDOW NOTICE (read before running on large tables)
-- =============================================================================
-- This migration acquires ACCESS EXCLUSIVE locks on multiple tables
-- (submissions, daily_breakdown, users) in a single transaction:
--   ALTER TABLE submissions   DROP COLUMN status
--   ALTER TABLE daily_breakdown DROP COLUMN provider_breakdown
--   ALTER TABLE daily_breakdown DROP COLUMN model_breakdown
--   ALTER TABLE users          DROP COLUMN is_admin
--   + four DROP INDEX / three CREATE INDEX statements
--
-- Each statement takes its own ACCESS EXCLUSIVE lock, which blocks all reads
-- and writes on that table for the duration of the statement. Running several
-- such statements in sequence extends the effective lock window.
--
-- This is ACCEPTABLE TODAY because all affected tables are small (< 100k rows)
-- and the DDL completes in milliseconds. On larger datasets, ACCESS EXCLUSIVE
-- holds can cause visible latency spikes or queue pile-ups under high concurrency.
--
-- FUTURE GUIDANCE: if table sizes grow significantly, split multi-table DDL into
-- separate migration files so lock windows do not overlap. Similarly, any future
-- migration adding multiple CREATE INDEX statements should use CREATE INDEX
-- CONCURRENTLY in separate transactions — CONCURRENTLY is not allowed inside an
-- explicit transaction block and cannot be batched with other DDL here.
-- =============================================================================

-- Drop dead schema surface confirmed by full-codebase audit on 2026-05-25.
-- Combines three cleanup categories surfaced by the audit:
--   (A) dead columns (no reads anywhere in src/)
--   (B) dead and redundant indexes
--   (C) redundant unique constraint
-- Plus one positive add: covering index for device_codes.user_id FK.
--
-- All DROPs are gated with IF EXISTS so the migration is safe to replay
-- against any environment regardless of out-of-band schema changes.

-- =============================================================================
-- (A) Dead columns
-- =============================================================================
--
-- daily_breakdown.provider_breakdown: declared in schema.ts but never read
--   or written anywhere in src/. Pure dead JSONB allocated on every daily row.
--
-- daily_breakdown.model_breakdown: written on every submit, but the only
--   SELECT (users/[username]/route.ts:110) places it in the result row and
--   never references the value. Net result: storage + serialization churn
--   with no consumer.
--
-- submissions.status: only ever written as 'verified' on insert
--   (submit/route.ts:205); zero WHERE filters exist anywhere in the codebase.
--   The accompanying idx_submissions_status (dropped below) served no query.
--   Status semantics were planned ("pending"/"rejected"?) but never wired.
--
-- users.is_admin: column is set/fetched into SessionUser, but no admin
--   gate exists in the codebase. A user with is_admin=true had zero
--   additional privileges. The column lied about what it did; if admin
--   features ever ship, reintroduce WITH an actual gate.

ALTER TABLE "submissions" DROP COLUMN IF EXISTS "status";--> statement-breakpoint
ALTER TABLE "daily_breakdown" DROP COLUMN IF EXISTS "provider_breakdown";--> statement-breakpoint
ALTER TABLE "daily_breakdown" DROP COLUMN IF EXISTS "model_breakdown";--> statement-breakpoint
ALTER TABLE "users" DROP COLUMN IF EXISTS "is_admin";--> statement-breakpoint

-- =============================================================================
-- (B) Dead and redundant indexes
-- =============================================================================
--
-- idx_submissions_status: column dropped above; index has nothing to point at.
--
-- idx_submissions_user_id: redundant left-prefix of idx_submissions_leaderboard
--   (user_id, total_tokens, total_cost, created_at). Per #389's prod-stats
--   audit, 214 scans on this vs 3.27M on the leaderboard composite — every
--   plain user_id lookup is already served by the composite's left-prefix.
--
-- idx_submissions_total_tokens: per #389's audit, 0 production scans.
--   Leaderboard reads use SUM() per-user GROUP BY which is served by the
--   composite leaderboard index; no single-row WHERE/ORDER BY on total_tokens
--   exists anywhere.
--
-- idx_submissions_date_range (date_start, date_end): per #389's audit, 0
--   production scans. No filter ever queries these columns; the date range
--   actually used by leaderboard period filters is on daily_breakdown.date
--   (which has its own index).
--
-- HOLD on the duplicate-of-unique-constraint indexes:
-- idx_users_username, idx_sessions_token, idx_api_tokens_token,
-- idx_device_codes_device_code, idx_device_codes_user_code.
-- The 2026-05-25 prod pg_stat_user_indexes audit showed these are HOT
-- (sessions_token 89k scans, users_username 30k, api_tokens_token 27k)
-- while their unique-constraint siblings show 0 scans — the planner
-- consistently picks the explicit non-unique index. Dropping would
-- invalidate cached plans; keep them until a controlled re-plan can
-- be scheduled.

DROP INDEX IF EXISTS "idx_submissions_status";--> statement-breakpoint
DROP INDEX IF EXISTS "idx_submissions_user_id";--> statement-breakpoint
DROP INDEX IF EXISTS "idx_submissions_total_tokens";--> statement-breakpoint
DROP INDEX IF EXISTS "idx_submissions_date_range";--> statement-breakpoint

-- FK coverage gap: three FK columns were lacking covering indexes.
-- Cascade-delete of a user did a seq scan of device_codes / group_members
-- / group_invites. Small tables today, free to fix once.
CREATE INDEX IF NOT EXISTS "idx_device_codes_user_id" ON "device_codes" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_group_members_invited_by" ON "group_members" USING btree ("invited_by");--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_group_invites_invited_by" ON "group_invites" USING btree ("invited_by");--> statement-breakpoint

-- =============================================================================
-- (C) Redundant unique constraint
-- =============================================================================
--
-- submissions_user_hash_unique (user_id, submission_hash) is functionally
-- subsumed by submissions_user_id_unique (user_id): the stronger constraint
-- already enforces one row per user, so any (user_id, submission_hash) tuple
-- collision would already be rejected at the user_id level. The submission_hash
-- column itself stays — it's still read by /api/submit for idempotency
-- comparison against the stored value. Only the redundant uniqueness goes.

ALTER TABLE "submissions" DROP CONSTRAINT IF EXISTS "submissions_user_hash_unique";
