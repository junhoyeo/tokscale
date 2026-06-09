-- This migration creates a UNIQUE functional index on lower(username)
-- so profile lookups can resolve case-insensitively while still
-- rejecting case-variant duplicate registrations.
--
-- The pre-flight DO block fails the migration with a clear message if
-- such duplicates already exist. Clean those up by hand before
-- re-running.
--
-- Note: CREATE UNIQUE INDEX is used instead of CREATE INDEX
-- CONCURRENTLY because Drizzle wraps each migration in a transaction
-- (CONCURRENTLY is not allowed inside a transaction). Build acquires
-- SHARE lock on "users" for the duration of the index build. For a
-- small users table this is fine; if the table grows large enough
-- that the build window matters, run a separate manual
-- `CREATE UNIQUE INDEX CONCURRENTLY` outside Drizzle before this
-- migration is applied. The `IF NOT EXISTS` guard makes the in-tree
-- migration a no-op afterwards.

DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM "users"
    GROUP BY lower("username")
    HAVING count(*) > 1
  ) THEN
    RAISE EXCEPTION 'Cannot create users_username_lower_unique while case-insensitive username duplicates exist';
  END IF;
END $$;
--> statement-breakpoint
CREATE UNIQUE INDEX IF NOT EXISTS "users_username_lower_unique" ON "users" (lower("username"));
