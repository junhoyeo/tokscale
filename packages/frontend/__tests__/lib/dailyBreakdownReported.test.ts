import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { PgDialect } from "drizzle-orm/pg-core";
import type { SQL } from "drizzle-orm";

import {
  foldContributionsIntoReportedRows,
  recordDailyBreakdownReported,
  type DailyBreakdownReportedContribution,
} from "@/lib/db/dailyBreakdownReported";

// Pins Phase 4a of docs/ratchet-inflation-recovery.md: an unguarded
// per-(device, date, client) shadow of what the CLI last reported. Nothing
// reads the table; the upsert must be last-write-wins (never GREATEST).

const dialect = new PgDialect();

interface CapturedQuery {
  sql: string;
  params: unknown[];
}

function capturingExecutor() {
  const queries: CapturedQuery[] = [];
  return {
    queries,
    execute: async (query: SQL): Promise<unknown> => {
      queries.push(dialect.sqlToQuery(query));
      return [];
    },
  };
}

function day(
  date: string,
  clients: Array<{
    client: string;
    modelId?: string;
    tokens?: number;
    cost?: number;
    input?: number;
    output?: number;
  }>,
  activeTimeMs?: number | null
): DailyBreakdownReportedContribution {
  return {
    date,
    activeTimeMs,
    clients: clients.map((c) => ({
      client: c.client,
      modelId: c.modelId ?? "model-a",
      messages: 1,
      cost: c.cost ?? 0,
      tokens: {
        input: c.input ?? c.tokens ?? 0,
        output: c.output ?? 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
      },
    })),
  };
}

describe("foldContributionsIntoReportedRows", () => {
  it("emits one row per (date, client), summing models on the same day", () => {
    const rows = foldContributionsIntoReportedRows(
      [
        day("2026-03-02", [
          { client: "claude", modelId: "a", tokens: 400, cost: 0.4 },
          { client: "claude", modelId: "b", tokens: 600, cost: 0.6 },
          { client: "codex", tokens: 50, cost: 0.05 },
        ]),
        day("2026-03-03", [{ client: "claude", tokens: 10, cost: 0.01 }]),
      ],
      "cli"
    );

    expect(rows).toEqual([
      {
        date: "2026-03-02",
        client: "claude",
        tokens: 1000,
        cost: 1,
        input: 1000,
        output: 0,
        activeTimeMs: null,
        origin: "cli",
      },
      {
        date: "2026-03-02",
        client: "codex",
        tokens: 50,
        cost: 0.05,
        input: 50,
        output: 0,
        activeTimeMs: null,
        origin: "cli",
      },
      {
        date: "2026-03-03",
        client: "claude",
        tokens: 10,
        cost: 0.01,
        input: 10,
        output: 0,
        activeTimeMs: null,
        origin: "cli",
      },
    ]);
  });

  it("carries day-level activeTimeMs onto every client row for that day", () => {
    const rows = foldContributionsIntoReportedRows(
      [
        day(
          "2026-03-02",
          [
            { client: "claude", tokens: 10 },
            { client: "codex", tokens: 20 },
          ],
          12_000
        ),
      ],
      "cli"
    );

    expect(rows.map((r) => r.activeTimeMs)).toEqual([12_000, 12_000]);
  });

  it("stamps a uniform origin from the submission-level tag", () => {
    const rows = foldContributionsIntoReportedRows(
      [day("2026-03-02", [{ client: "claude", tokens: 1 }])],
      "backfill"
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].origin).toBe("backfill");
  });

  it("skips malformed dates rather than inventing a shadow key", () => {
    const rows = foldContributionsIntoReportedRows(
      [
        day("2026-02-31", [{ client: "claude", tokens: 10 }]),
        day("not-a-date", [{ client: "claude", tokens: 10 }]),
        day("2026-03-02", [{ client: "claude", tokens: 7 }]),
      ],
      "cli"
    );
    expect(rows).toEqual([
      expect.objectContaining({ date: "2026-03-02", tokens: 7 }),
    ]);
  });

  it("returns nothing for an empty payload", () => {
    expect(foldContributionsIntoReportedRows([], "cli")).toEqual([]);
  });
});

describe("recordDailyBreakdownReported upsert", () => {
  it("is last-write-wins: conflict assigns EXCLUDED, never GREATEST", async () => {
    const executor = capturingExecutor();
    await recordDailyBreakdownReported({
      executor,
      submittedDeviceId: "11111111-1111-4111-8111-111111111111",
      rows: foldContributionsIntoReportedRows(
        [day("2026-03-02", [{ client: "claude", tokens: 1000, cost: 1 }])],
        "cli"
      ),
    });

    const [query] = executor.queries;
    expect(query.sql).toContain("INSERT INTO daily_breakdown_reported");
    expect(query.sql).toMatch(/tokens\s*=\s*EXCLUDED\.tokens/);
    expect(query.sql).toMatch(/cost\s*=\s*EXCLUDED\.cost/);
    expect(query.sql).toMatch(/input\s*=\s*EXCLUDED\.input/);
    expect(query.sql).toMatch(/output\s*=\s*EXCLUDED\.output/);
    expect(query.sql).toMatch(/active_time_ms\s*=\s*EXCLUDED\.active_time_ms/);
    expect(query.sql).toMatch(/origin\s*=\s*EXCLUDED\.origin/);
    expect(query.sql).toMatch(/reported_at\s*=\s*EXCLUDED\.reported_at/);
    // A GREATEST arm would freeze the inflated shadow the same way the
    // guarded daily merge freezes #960 — the whole point of this table is
    // that a lower truthful rescan replaces the previous report.
    expect(query.sql).not.toContain("GREATEST");
  });

  it("matches the primary key the migration actually creates", async () => {
    // A conflict target that does not match a real unique constraint is a
    // runtime error, so the DDL and the upsert are pinned to each other.
    const migration = readFileSync(
      resolve(
        __dirname,
        "../../src/lib/db/migrations/0025_bouncy_vertigo.sql"
      ),
      "utf8"
    );
    const pk = migration.match(/PRIMARY KEY\(([^)]*)\)/)?.[1];
    expect(pk?.split(",").map((c) => c.trim().replace(/"/g, ""))).toEqual([
      "submitted_device_id",
      "date",
      "client",
    ]);

    const executor = capturingExecutor();
    await recordDailyBreakdownReported({
      executor,
      submittedDeviceId: "11111111-1111-4111-8111-111111111111",
      rows: foldContributionsIntoReportedRows(
        [day("2026-03-02", [{ client: "claude", tokens: 1 }])],
        "cli"
      ),
    });
    const conflictTarget = executor.queries[0].sql.match(
      /ON CONFLICT \(([^)]*)\)/
    )?.[1];
    expect(conflictTarget?.split(",").map((c) => c.trim())).toEqual([
      "submitted_device_id",
      "date",
      "client",
    ]);
  });

  it("keeps origin out of the conflict key so a later scan replaces either provenance", async () => {
    const executor = capturingExecutor();
    await recordDailyBreakdownReported({
      executor,
      submittedDeviceId: "11111111-1111-4111-8111-111111111111",
      rows: foldContributionsIntoReportedRows(
        [day("2026-03-02", [{ client: "claude", tokens: 1 }])],
        "cli"
      ),
    });

    const [query] = executor.queries;
    const conflictTarget = query.sql.match(/ON CONFLICT \(([^)]*)\)/)?.[1];
    expect(conflictTarget?.split(",").map((c) => c.trim())).toEqual([
      "submitted_device_id",
      "date",
      "client",
    ]);
    // Unlike Phase 1's submitted_device_client_totals, a later scan of either
    // origin replaces the previous report for that (device, date, client).
    expect(conflictTarget).not.toContain("origin");
  });

  it("binds the folded unguarded totals for the addressed device", async () => {
    const executor = capturingExecutor();
    await recordDailyBreakdownReported({
      executor,
      submittedDeviceId: "22222222-2222-4222-8222-222222222222",
      rows: foldContributionsIntoReportedRows(
        [
          day(
            "2026-03-02",
            [
              {
                client: "claude",
                tokens: 1000,
                cost: 1.5,
                input: 800,
                output: 200,
              },
            ],
            4500
          ),
        ],
        "cli"
      ),
      now: new Date("2026-03-04T00:00:00.000Z"),
    });

    const [query] = executor.queries;
    expect(query.params).toContain("22222222-2222-4222-8222-222222222222");
    expect(query.params).toContain("2026-03-02");
    expect(query.params).toContain("claude");
    expect(query.params).toContain(1000);
    expect(query.params).toContain("1.5000");
    expect(query.params).toContain(800);
    expect(query.params).toContain(200);
    expect(query.params).toContain(4500);
    expect(query.params).toContain("cli");
    expect(query.params).toContain("2026-03-04T00:00:00.000Z");
  });

  it("writes nothing when the folded payload is empty", async () => {
    const executor = capturingExecutor();
    const written = await recordDailyBreakdownReported({
      executor,
      submittedDeviceId: "11111111-1111-4111-8111-111111111111",
      rows: [],
    });
    expect(written).toBe(0);
    expect(executor.queries).toEqual([]);
  });

  it("chunks large payloads under the Postgres bind-parameter cap", async () => {
    const executor = capturingExecutor();
    const padded = Array.from({ length: 1001 }, (_, i) => ({
      date: "2020-01-01",
      client: `client-${i}`,
      tokens: 1,
      cost: 0,
      input: 1,
      output: 0,
      activeTimeMs: null as number | null,
      origin: "cli" as const,
    }));

    await recordDailyBreakdownReported({
      executor,
      submittedDeviceId: "11111111-1111-4111-8111-111111111111",
      rows: padded,
    });

    expect(executor.queries).toHaveLength(2);
    expect(
      executor.queries.every((q) =>
        q.sql.includes("INSERT INTO daily_breakdown_reported")
      )
    ).toBe(true);
  });
});

describe("#960 divergence the shadow must capture", () => {
  it("records the moved day and leaves the emptied day absent from the payload fold", () => {
    // First Seoul scan reported 2026-03-03 = 1000. UTC rescan moves the same
    // 1000 tokens onto 2026-03-02. The guarded store keeps both days (2000);
    // the shadow of THIS payload only knows about 03-02.
    const rows = foldContributionsIntoReportedRows(
      [day("2026-03-02", [{ client: "claude", tokens: 1000 }])],
      "cli"
    );

    expect(rows).toEqual([
      expect.objectContaining({
        date: "2026-03-02",
        client: "claude",
        tokens: 1000,
      }),
    ]);
    expect(rows.some((r) => r.date === "2026-03-03")).toBe(false);
  });

  it("records the lower incoming total even when the guard would preserve higher", () => {
    // Guarded merge would keep stored codex=500 on 03-02 while accepting the
    // move onto 03-03. The shadow must still report the truthful incoming:
    // claude 500 on 03-02 and codex 500 on 03-03 — no preserved stale codex.
    const rows = foldContributionsIntoReportedRows(
      [
        day("2026-03-02", [{ client: "claude", tokens: 500 }]),
        day("2026-03-03", [{ client: "codex", tokens: 500 }]),
      ],
      "cli"
    );

    expect(rows).toEqual([
      expect.objectContaining({
        date: "2026-03-02",
        client: "claude",
        tokens: 500,
      }),
      expect.objectContaining({
        date: "2026-03-03",
        client: "codex",
        tokens: 500,
      }),
    ]);
    expect(
      rows.some((r) => r.date === "2026-03-02" && r.client === "codex")
    ).toBe(false);
  });
});
