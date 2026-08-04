/// <reference types="bun-types" />
// This runs BEFORE `next build` in vercel.json's buildCommand
// (`bun run scripts/migrate-prod.ts && next build`), intentionally — not after.
// Apply migrations before building so the deployed application and schema move
// forward together. This remains necessary even though root metadata is
// request-dynamic: build tooling and future route changes may validate schema
// assumptions, and deployment must never start code against an older schema.
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
