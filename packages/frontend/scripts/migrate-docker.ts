/// <reference types="bun-types" />
// Migration entrypoint for Docker / self-hosted deployments.
// Mirrors the advisory-lock + retry strategy from migrate-prod.ts without
// the VERCEL_ENV gate (which is irrelevant outside Vercel builds).
import postgres from "postgres";
import { classifyFailure } from "./migrate-retry";

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  throw new Error("DATABASE_URL is required");
}

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
    cwd: import.meta.dir + "/..",
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
