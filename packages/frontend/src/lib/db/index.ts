import { drizzle } from "drizzle-orm/postgres-js";
import * as schema from "./schema";

function getConnectionString(): string {
  const connectionString = process.env.DATABASE_URL;

  if (!connectionString) {
    throw new Error("DATABASE_URL environment variable is not set");
  }

  return connectionString;
}

export function getDatabaseSslMode(): "require" | false {
  const configured = process.env.DATABASE_SSL?.trim().toLowerCase();

  if (configured === "false" || configured === "disable") {
    return false;
  }
  if (configured === "true" || configured === "require") {
    return "require";
  }

  // Preserve the existing managed-production default while allowing local
  // container deployments to opt out explicitly via DATABASE_SSL=false.
  return process.env.NODE_ENV === "production" ? "require" : false;
}

// Singleton pattern: prevent creating multiple connection pools across
// serverless invocations sharing the same runtime (hot-start reuse).
//
// Use drizzle's config-based API to create the postgres client internally.
// Passing a `postgres` Sql instance directly causes type errors on Vercel
// due to duplicate package resolution in the monorepo (two copies of postgres
// with incompatible branded types).
function createDb() {
  return drizzle({
    connection: {
      url: getConnectionString(),
      ssl: getDatabaseSslMode(),

      // Each Vercel function instance gets its own pool, and one instance
      // serves many requests at once. With a single connection every query
      // on the instance waits behind the slowest one in flight, so a rollup
      // scan or a submit transaction stalls unrelated page loads.
      //
      // Keep the pool small all the same: the total is instances x max, and
      // exceeding the server's max_connections fails requests (error 53300).
      // Connections open on demand, so an idle instance still holds none.
      max: 3,

      // Close idle connections after 20 s so they don't linger between
      // infrequent invocations.
      idle_timeout: 20,

      // Hard cap: recycle every connection after 5 minutes regardless of
      // activity. Prevents stale connections after deploys / DB restarts.
      max_lifetime: 60 * 5,

      // Fail fast when the DB is unreachable instead of hanging the request.
      connect_timeout: 10,

      // Prepared statements are connection-scoped. In serverless, the
      // connection that prepared a statement may be gone by the next
      // invocation, causing "prepared statement does not exist" errors.
      prepare: false,
    },
    schema,
  });
}

type DbClient = ReturnType<typeof createDb>;

const globalForDb = globalThis as unknown as {
  _db: DbClient | undefined;
};

export function getDb(): DbClient {
  if (!globalForDb._db) {
    globalForDb._db = createDb();
  }

  return globalForDb._db;
}

export const db: DbClient = new Proxy({} as DbClient, {
  get(_target, prop) {
    const value = Reflect.get(getDb(), prop);
    return typeof value === "function" ? value.bind(getDb()) : value;
  },
});

export * from "./schema";
