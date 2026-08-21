/// <reference types="bun-types" />
// Migration entrypoint for Docker / self-hosted deployments.
// Delegates to migrate-core for the advisory-lock + retry strategy.
// No VERCEL_ENV gate — that guard is only relevant inside Vercel builds.
import { runMigrations } from "./migrate-core";

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  throw new Error("DATABASE_URL is required");
}

// cwd: docker-entrypoint.sh changes to /app before exec, so drizzle-kit
// must be pointed back at packages/frontend where drizzle.config.ts lives.
await runMigrations({ databaseUrl, cwd: import.meta.dir + "/.." });
