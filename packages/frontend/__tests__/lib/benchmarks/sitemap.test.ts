import { afterEach, describe, expect, it, vi } from "vitest";
import { benchmarksSitemapEntries, modelsCatalogSitemapEntry } from "@/lib/benchmarks/sitemap";
import { benchmarkData } from "@/lib/benchmarks";

afterEach(() => vi.unstubAllEnvs());

describe("benchmark sitemap entries", () => {
  it("lists /benchmarks and every benchmark page with results, dated by snapshot retrieval", () => {
    const entries = benchmarksSitemapEntries();
    const urls = entries.map((e) => e.url);
    expect(urls[0]).toBe("https://tokscale.ai/benchmarks");
    expect(urls).toContain("https://tokscale.ai/benchmarks/terminal-bench-4");
    expect(urls).toContain("https://tokscale.ai/benchmarks/epoch-capabilities-index");
    // Link-only benchmarks have no page.
    expect(urls).not.toContain("https://tokscale.ai/benchmarks/swe-bench-pro-v2");
    const tbench = entries.find((e) => e.url.endsWith("/terminal-bench-4"));
    const retrieved = benchmarkData.get("terminal-bench-4")?.snapshot?.retrievedAt.slice(0, 10);
    expect((tbench?.lastModified as Date).toISOString().slice(0, 10)).toBe(retrieved);
    expect(entries.find((e) => e.url.endsWith("/swe-bench-verified"))?.priority).toBe(0.3);
  });

  it("follows the runtime public origin", () => {
    vi.stubEnv("APP_URL", "https://self-hosted.example/");
    expect(benchmarksSitemapEntries()[0].url).toBe("https://self-hosted.example/benchmarks");
  });

  it("dates /models/all by the newest submission, or omits the date", () => {
    const date = new Date("2026-10-19T09:24:00.000Z");
    expect(modelsCatalogSitemapEntry(date)).toEqual({ url: "https://tokscale.ai/models/all", changeFrequency: "daily", priority: 0.6, lastModified: date });
    expect(modelsCatalogSitemapEntry(null)).not.toHaveProperty("lastModified");
  });
});
