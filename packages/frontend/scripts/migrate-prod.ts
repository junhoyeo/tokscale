/// <reference types="bun-types" />
import postgres from "postgres";
import { classifyFailure } from "./migrate-retry";

// This runs BEFORE `next build` in vercel.json's buildCommand
// (`bun run scripts/migrate-prod.ts && next build`), intentionally — not after.
// `src/app/(main)/page.tsx` (HomePage) has no dynamic rendering signal, so it's
// statically prerendered at build time, and its render path calls
// `getLeaderboardData`, which queries the DB directly (through an
// `unstable_cache` wrapper — caching the fetch doesn't defer *when* it first
// runs). Reordering to build-then-migrate would make `next build` fail
// whenever a PR's new code depends on its own accompanying migration,
// permanently blocking that deploy since the migration never gets a chance to
// run. The residual risk of the current order (migrate succeeds, then build
// fails for an unrelated reason, leaving new schema paired with old code) is
// mitigated by this repo's convention of additive-only migrations.
//
// Vercel has no buildCommand-level distinction between "preview build for a
// WIP branch" and "production build" other than VERCEL_ENV — and DATABASE_URL
// is the SAME value across Production/Preview/Development in this project.
// Without this gate, pushing an unreviewed migration to any branch would
// apply it to prod the moment its preview build runs.
if (process.env.VERCEL_ENV !== "production") {
  console.log(
    `skip - migrate-prod: VERCEL_ENV=${process.env.VERCEL_ENV ?? "(unset)"}, not production`
  );
  process.exit(0);
}

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  throw new Error("DATABASE_URL is required");
}

// drizzle-kit/drizzle-orm take no advisory lock of their own, so concurrent
// builds (rapid pushes, a manual redeploy overlapping an in-flight one) can
// race two `drizzle-kit migrate` runs against each other. Hold a session
// lock for the lifetime of this process to serialize them.
const LOCK_KEY = "tokscale_drizzle_migrate";
const MAX_LOCK_ATTEMPTS = 60;
const MAX_ATTEMPTS = 5;
const RETRY_DELAY_MS = 3000;

const sql = postgres(databaseUrl, { max: 1 });

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function runMigrate(): Promise<{
  ok: boolean;
  retryable: boolean;
  reason: string;
  stderr: string;
}> {
  const proc = Bun.spawn(["bunx", "drizzle-kit", "migrate"], {
    stdout: "inherit",
    stderr: "pipe",
  });
  const stderr = await new Response(proc.stderr).text();
  process.stderr.write(stderr);
  const exitCode = await proc.exited;
  if (exitCode === 0) {
    return { ok: true, retryable: false, reason: "", stderr };
  }
  const { retryable, reason } = classifyFailure(stderr);
  return { ok: false, retryable, reason, stderr };
}

// Acquire (or re-acquire) the session-scoped advisory lock that serializes
// concurrent `drizzle-kit migrate` runs across overlapping builds. Loops on
// two conditions so it survives a wobbly database:
//   - the query itself fails (DB still unreachable, e.g. mid-outage): keep
//     retrying so a transient blip doesn't abort a deploy;
//   - the lock is held by another build: wait for that build to finish, then
//     re-check -- we never run a migration without holding the lock.
// Throws only after MAX_LOCK_ATTEMPTS. Safe to call again on the migrate-retry
// path: a transient connection drop can sever the session and silently release
// the lock (advisory locks die with their connection), so re-establishing it
// before each retry is what prevents two builds from migrating at once.
async function acquireAdvisoryLock(): Promise<void> {
  for (let attempt = 1; attempt <= MAX_LOCK_ATTEMPTS; attempt++) {
    let acquired = false;
    try {
      const [row] = await sql<{ acquired: boolean }[]>`
        SELECT pg_try_advisory_lock(hashtext(${LOCK_KEY})) AS acquired
      `;
      acquired = row?.acquired ?? false;
    } catch (error) {
      // Only wait out a TRANSIENT connectivity failure (same classifier as the
      // migrate retry). A permanent error -- bad credentials, unreachable host,
      // a missing function -- won't fix itself, so fail the deploy immediately
      // instead of burning ~MAX_LOCK_ATTEMPTS of pointless waits.
      const errorText = `${error} ${(error as { code?: unknown })?.code ?? ""}`;
      if (attempt === MAX_LOCK_ATTEMPTS || !classifyFailure(errorText).retryable) {
        throw error;
      }
      console.warn(
        `warn - could not reach DB to acquire migration advisory lock (attempt ${attempt}/${MAX_LOCK_ATTEMPTS}); retrying in ${RETRY_DELAY_MS}ms`
      );
      await sleep(RETRY_DELAY_MS);
      continue;
    }
    if (acquired) return;
    if (attempt < MAX_LOCK_ATTEMPTS) {
      console.warn(
        `warn - migration advisory lock held by another build (attempt ${attempt}/${MAX_LOCK_ATTEMPTS}); retrying in ${RETRY_DELAY_MS}ms`
      );
      await sleep(RETRY_DELAY_MS);
    }
  }
  throw new Error(
    `could not acquire migration advisory lock after ${MAX_LOCK_ATTEMPTS} attempts -- a concurrent build may be stuck`
  );
}

let lockAcquired = false;

try {
  await acquireAdvisoryLock();
  lockAcquired = true;
  console.log(`ok - acquired advisory lock (${LOCK_KEY})`);

  let lastResult: Awaited<ReturnType<typeof runMigrate>> | undefined;
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
    lastResult = await runMigrate();
    if (lastResult.ok) {
      console.log(`ok - drizzle-kit migrate succeeded (attempt ${attempt}/${MAX_ATTEMPTS})`);
      break;
    }
    if (!lastResult.retryable) {
      throw new Error(
        `drizzle-kit migrate failed (attempt ${attempt}/${MAX_ATTEMPTS}, ${lastResult.reason} — not retrying)`
      );
    }
    console.warn(
      `warn - drizzle-kit migrate hit a transient ${lastResult.reason} (attempt ${attempt}/${MAX_ATTEMPTS})`
    );
    if (attempt === MAX_ATTEMPTS) {
      throw new Error(
        `drizzle-kit migrate failed with a transient ${lastResult.reason} ${MAX_ATTEMPTS} times in a row`
      );
    }
    await sleep(RETRY_DELAY_MS);

    // The retryable failure may have severed the parent session holding the
    // advisory lock (see acquireAdvisoryLock). Re-establish it before the next
    // attempt so we never migrate without the lock -- waiting through both a
    // still-recovering DB and a concurrent holder rather than aborting.
    await acquireAdvisoryLock();
  }
} finally {
  if (lockAcquired) {
    try {
      // Release every level held -- the retry path's re-acquire is re-entrant,
      // so the session can hold the lock more than once. pg_advisory_unlock_all
      // clears them all regardless of depth.
      await sql`SELECT pg_advisory_unlock_all()`;
    } catch (error) {
      console.error("warn - failed to release migration advisory lock", error);
    }
  }

  try {
    await sql.end();
  } catch (error) {
    console.error("warn - failed to close migration database connection", error);
  }
}
