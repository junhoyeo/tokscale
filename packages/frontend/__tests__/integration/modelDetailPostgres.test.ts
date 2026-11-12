import { randomUUID } from "node:crypto";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { sql } from "drizzle-orm";

vi.mock("next/cache", () => ({ unstable_cache: (load: () => unknown) => load, revalidateTag: vi.fn() }));

import { db } from "@/lib/db";
import { computeModelDetail } from "@/lib/explore/db";
import type { ModelCatalog, PeriodRollup } from "@/lib/explore/rollup";

// Never run against the shared development DB or an inherited production URL.
// Opt in by explicitly selecting a migrated, isolated local database:
// DATABASE_URL=postgresql://...@127.0.0.1:5480/tokscale_analytics_revision_<name>
const url = process.env.DATABASE_URL ? new URL(process.env.DATABASE_URL) : null;
const isolatedLocal = url?.hostname === "127.0.0.1" && url.port === "5480"
  && url.pathname.startsWith("/tokscale_analytics_revision_");

(isolatedLocal ? describe : describe.skip)("model detail PostgreSQL integration", () => {
  const suffix = randomUUID().slice(0, 8);
  const userId = randomUUID();
  const username = `detail_${suffix}`;
  const key = `detail-model-${suffix}`;
  // Raw ids as accounts submit them, including characters that are special in a jsonpath string.
  const quoted = `${key}") || (1 == 1`;
  const legacy = `${key}-legacy\\id`;
  const other = `other-model-${suffix}`;
  const days = {
    perModel: "2031-03-01",
    legacy: "2031-03-02",
    otherModel: "2031-03-03",
    // A per-model map wins over modelId, so this day has no usage of the legacy id.
    shadowedLegacy: "2031-03-04",
  };

  beforeAll(async () => {
    await db.execute(sql`SET statement_timeout = '10s'`);
    await db.execute(sql`INSERT INTO users (id, github_id, username) VALUES (${userId}, ${1_800_000_000 + Math.floor(Math.random() * 100_000_000)}, ${username})`);
    const [device] = await db.execute<{ id: string }>(sql`
      INSERT INTO submitted_devices (user_id, device_key) VALUES (${userId}, ${`device-${suffix}`}) RETURNING id
    `);
    const [submission] = await db.execute<{ id: string }>(sql`
      INSERT INTO submissions (user_id, total_tokens, total_cost, input_tokens, output_tokens, date_start, date_end, sources_used, models_used)
      VALUES (${userId}, 225, 2, 225, 0, ${days.perModel}, ${days.shadowedLegacy}, ARRAY['claude', 'codex'], ARRAY[${key}])
      RETURNING id
    `);
    const day = (date: string, tokens: number, breakdown: unknown) => db.execute(sql`
      INSERT INTO daily_breakdown (submission_id, submitted_device_id, date, tokens, cost, input_tokens, output_tokens, source_breakdown)
      VALUES (${submission.id}, ${device.id}, ${date}, ${tokens}, 1, ${tokens}, 0, ${JSON.stringify(breakdown)}::jsonb)
    `);
    await day(days.perModel, 100, { claude: { tokens: 100, models: { [quoted]: { tokens: 100, cost: 1 } } } });
    await day(days.legacy, 50, { codex: { tokens: 50, cost: 0.5, modelId: legacy } });
    await day(days.otherModel, 70, { claude: { tokens: 70, models: { [other]: { tokens: 70 } } } });
    await day(days.shadowedLegacy, 5, { claude: { tokens: 5, modelId: legacy, models: { [other]: { tokens: 5 } } } });
  });

  afterAll(async () => {
    await db.execute(sql`DELETE FROM users WHERE id = ${userId}`);
  });

  it("counts only the days that carry one of the model's raw ids", async () => {
    const catalog: ModelCatalog = { modelFirstSeen: {}, agentFirstSeen: {}, aliases: { [key]: [quoted, legacy] } };
    const rollup = {
      coverage: { period: "all", start: days.perModel, end: days.shadowedLegacy, accounts: 1, modelIdentifiedShare: 1, lastDataAt: null },
      models: [{ key }],
      agents: [],
      humans: [],
    } as unknown as PeriodRollup;

    const detail = await computeModelDetail(key, "all", { catalog, rollup });

    expect(detail?.trend.filter((point) => point.tokens > 0)).toEqual([
      { date: days.perModel, activeUsers: 1, tokens: 100 },
      { date: days.legacy, activeUsers: 1, tokens: 50 },
    ]);
    expect(detail?.agents.map((agent) => agent.key).sort()).toEqual(["claude", "codex"]);
    expect(detail?.humans.map((human) => [human.username, human.tokens])).toEqual([[username, 150]]);
  });
});
