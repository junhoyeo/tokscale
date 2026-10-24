import { sql } from "drizzle-orm";
import { db } from "@/lib/db";

type Transaction = Parameters<Parameters<typeof db.transaction>[0]>[0];

/**
 * Deliberately NOT cached, even in process. One primary-key lookup fences every
 * top-level analytics read against committed privacy changes across instances.
 * The migration seeds the singleton. A missing row/table or DB error must
 * propagate: never fall back to a prior namespace or illustrative data.
 */
export async function getExploreCacheGeneration(): Promise<string> {
  const rows = await db.execute<{ revision: string }>(sql`
    SELECT revision::text AS revision FROM explore_cache_revision WHERE id = 1
  `);
  const revision = rows[0]?.revision;
  if (!revision) throw new Error("Explore cache revision unavailable; apply frontend migrations");
  return revision;
}

/** Must be awaited INSIDE the privacy mutation transaction, never best-effort. */
export async function advanceExploreCacheRevision(tx: Transaction): Promise<void> {
  const rows = await tx.execute<{ revision: string }>(sql`
    UPDATE explore_cache_revision SET revision = revision + 1 WHERE id = 1
    RETURNING revision::text AS revision
  `);
  if (rows.length !== 1) throw new Error("Explore cache revision unavailable; apply frontend migrations");
}
