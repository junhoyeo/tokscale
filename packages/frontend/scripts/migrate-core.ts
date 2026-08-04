/// <reference types="bun-types" />
// Shared advisory-lock migration runner used by both migrate-prod.ts (Vercel)
// and migrate-docker.ts (Docker / self-hosted). Pass `cwd` when the working
// directory differs from the packages/frontend root (e.g. the Docker
// entrypoint changes to /app before invoking this).
import postgres from "postgres";
import { classifyFailure } from "./migrate-retry";
import { getDatabaseSslMode } from "../src/lib/db";

const LOCK_KEY = "tokscale_drizzle_migrate";
const MAX_LOCK_ATTEMPTS = 60;
const MAX_ATTEMPTS = 5;
const RETRY_DELAY_MS = 3000;

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function runMigrations(opts: {
  databaseUrl: string;
  cwd?: string;
}): Promise<void> {
  const ssl = getDatabaseSslMode();
  const migrationUrl = new URL(opts.databaseUrl);
  migrationUrl.searchParams.set("sslmode", ssl === false ? "disable" : "require");
  const databaseUrl = migrationUrl.toString();
  const sql = postgres(databaseUrl, { max: 1, ssl });

  async function runMigrate(): Promise<{
    ok: boolean;
    retryable: boolean;
    reason: string;
    stderr: string;
  }> {
    const proc = Bun.spawn(["bunx", "drizzle-kit", "migrate"], {
      stdout: "inherit",
      stderr: "pipe",
      ...(opts.cwd ? { cwd: opts.cwd } : {}),
      env: { ...process.env, DATABASE_URL: databaseUrl },
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
  // concurrent drizzle-kit migrate runs. Loops on two conditions so it
  // survives a wobbly database:
  //   - the query itself fails (DB still unreachable): keep retrying so a
  //     transient blip doesn't abort a deploy;
  //   - the lock is held by another build: wait, then re-check.
  // Safe to call again on the migrate-retry path: a transient connection drop
  // can sever the session and silently release the lock (advisory locks die
  // with their connection), so re-establishing it before each retry prevents
  // two instances from migrating at once.
  async function acquireAdvisoryLock(): Promise<void> {
    for (let attempt = 1; attempt <= MAX_LOCK_ATTEMPTS; attempt++) {
      let acquired = false;
      try {
        const [row] = await sql<{ acquired: boolean }[]>`
          SELECT pg_try_advisory_lock(hashtext(${LOCK_KEY})) AS acquired
        `;
        acquired = row?.acquired ?? false;
      } catch (error) {
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
          `warn - migration advisory lock held by another instance (attempt ${attempt}/${MAX_LOCK_ATTEMPTS}); retrying in ${RETRY_DELAY_MS}ms`
        );
        await sleep(RETRY_DELAY_MS);
      }
    }
    throw new Error(
      `could not acquire migration advisory lock after ${MAX_LOCK_ATTEMPTS} attempts`
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
      await acquireAdvisoryLock();
    }
  } finally {
    if (lockAcquired) {
      try {
        // pg_advisory_unlock_all clears all levels — the retry path's
        // re-acquire is re-entrant, so the session can hold the lock more
        // than once.
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
}
