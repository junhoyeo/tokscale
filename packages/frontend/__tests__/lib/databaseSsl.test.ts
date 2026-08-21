import { afterEach, describe, expect, it, vi } from "vitest";

const environment = { ...process.env };

async function loadDb() {
  vi.resetModules();
  delete (globalThis as { _db?: unknown })._db;
  const drizzle = vi.fn(() => ({}));
  vi.doMock("drizzle-orm/postgres-js", () => ({ drizzle }));
  await import("@/lib/db");
  return drizzle;
}

afterEach(() => {
  process.env = { ...environment };
  delete (globalThis as { _db?: unknown })._db;
  vi.resetModules();
  vi.unmock("drizzle-orm/postgres-js");
});

describe("database SSL mode", () => {
  it("allows the self-hosted Compose stack to disable TLS in production", async () => {
    Object.assign(process.env, { NODE_ENV: "production" });
    process.env.DATABASE_URL = "postgresql://tokscale:tokscale@db:5432/tokscale";
    process.env.DATABASE_SSL = "false";
    const drizzle = await loadDb();

    const { getDb } = await import("@/lib/db");
    getDb();

    expect(drizzle).toHaveBeenCalledWith(
      expect.objectContaining({ connection: expect.objectContaining({ ssl: false }) })
    );
  });

  it("keeps TLS required for production databases unless explicitly disabled", async () => {
    Object.assign(process.env, { NODE_ENV: "production" });
    process.env.DATABASE_URL = "postgresql://example.com/tokscale";
    delete process.env.DATABASE_SSL;
    const drizzle = await loadDb();

    const { getDb } = await import("@/lib/db");
    getDb();

    expect(drizzle).toHaveBeenCalledWith(
      expect.objectContaining({ connection: expect.objectContaining({ ssl: "require" }) })
    );
  });
});
