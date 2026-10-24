/**
 * Time the explore rollups against the local seeded database and print a
 * short sanity report. Uses the uncached compute* functions directly.
 *
 * Usage (from packages/frontend):
 *   DATABASE_URL=postgresql://tokscale:tokscale@127.0.0.1:5480/tokscale DATABASE_SSL=false \
 *     bun scripts/stealth/measure.ts
 */
import { computeCatalog, computeModelDetail, computePeriodRollup } from "@/lib/explore/db";
import type { ExplorePeriod } from "@/lib/explore/types";

async function timed<T>(label: string, fn: () => Promise<T>): Promise<T> {
  const started = performance.now();
  const value = await fn();
  console.log(`${label.padEnd(28)} ${(performance.now() - started).toFixed(0).padStart(6)} ms`);
  return value;
}

function compact(n: number): string {
  return Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 }).format(n);
}

async function main(): Promise<void> {
  const url = process.env.DATABASE_URL ?? "";
  const host = url ? new URL(url).hostname : "";
  if (host !== "127.0.0.1" && host !== "localhost") throw new Error("measure.ts only runs against a local DATABASE_URL");

  const catalog = await timed("catalog (all time)", computeCatalog);
  for (const period of ["7d", "30d", "all"] as ExplorePeriod[]) {
    const rollup = await timed(`rollup ${period}`, () => computePeriodRollup(period, catalog));
    const bytes = new TextEncoder().encode(JSON.stringify(rollup)).byteLength;
    const c = rollup.coverage;
    console.log(
      `  ${c.start}..${c.end} accounts=${c.accounts} identified=${(c.modelIdentifiedShare * 100).toFixed(2)}% lastDataAt=${c.lastDataAt} models=${rollup.models.length} agents=${rollup.agents.length} humans=${rollup.humans.length} cache=${(bytes / 1024).toFixed(0)} KiB`,
    );
    console.log(`  models: ${rollup.models.slice(0, 6).map((m) => `${m.key}(${m.activeUsers}/${m.previousActiveUsers ?? "-"}${m.isNew ? ",new" : ""})`).join(" ")}`);
    console.log(`  agents: ${rollup.agents.slice(0, 6).map((a) => `${a.key}(${a.activeUsers})`).join(" ")}`);
    console.log(`  humans: ${rollup.humans.slice(0, 4).map((h) => `${h.username}(${compact(h.tokens)})`).join(" ")}`);
    const leaked = rollup.models.some((m) => m.key === "synthetic-spoof-model") || rollup.humans.some((h) => h.username.startsWith("synthetic-hidden-"));
    console.log(`  hidden accounts leaked: ${leaked}`);
    if (period === "30d" && rollup.models[0]) {
      const key = rollup.models[0].key;
      const detail = await timed(`model detail ${key} 30d`, () => computeModelDetail(key, period, { catalog, rollup }));
      console.log(
        `  rank=${detail?.summary.rank} users=${detail?.summary.activeUsers} trend=${detail?.trend.length}d agents=${detail?.agents.map((a) => a.key).join(",")} aliases=${detail?.model.aliases.join(",")}`,
      );
    }
    if (period === "all" && rollup.models[0]) {
      const key = rollup.models[0].key;
      await timed(`model detail ${key} all`, () => computeModelDetail(key, period, { catalog, rollup }));
    }
  }
  process.exit(0);
}

await main();
