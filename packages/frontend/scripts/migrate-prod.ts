/// <reference types="bun-types" />
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

import { runMigrations } from "./migrate-core";

// No cwd override — vercel.json buildCommand runs from packages/frontend,
// which is already drizzle-kit's expected working directory.
await runMigrations({ databaseUrl });
