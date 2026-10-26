import { afterEach, describe, expect, it, vi } from "vitest";
import {
  breadcrumbJsonLd,
  buildExploreSitemapEntries,
  exploreLeaderboardUrl,
  methodologyUrl,
  modelPath,
  modelUrl,
  modelsIndexUrl,
  serializeJsonLd,
} from "@/lib/seo/explore";

describe("explore SEO helpers", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("builds canonical URLs on the configured origin", () => {
    vi.stubEnv("APP_URL", "https://tokscale.ai/some/path");
    expect(modelUrl("claude-opus-5")).toBe("https://tokscale.ai/models/claude-opus-5");
    expect(modelsIndexUrl()).toBe("https://tokscale.ai/models");
    expect(methodologyUrl()).toBe("https://tokscale.ai/methodology");
    expect(exploreLeaderboardUrl("agents")).toBe("https://tokscale.ai/leaderboard?view=agents");
  });

  it("encodes model keys in paths", () => {
    expect(modelPath("gpt-5.6-sol")).toBe("/models/gpt-5.6-sol");
    expect(modelPath("a/b c")).toBe("/models/a%2Fb%20c");
  });

  it("numbers breadcrumb positions from 1", () => {
    const ld = breadcrumbJsonLd([
      { name: "Tokscale", url: "https://tokscale.ai" },
      { name: "Models", url: "https://tokscale.ai/models" },
    ]);
    expect(ld["@type"]).toBe("BreadcrumbList");
    expect(ld.itemListElement.map((item) => item.position)).toEqual([1, 2]);
    expect(ld.itemListElement[1]).toMatchObject({ "@type": "ListItem", name: "Models", item: "https://tokscale.ai/models" });
  });

  it("escapes < so a name cannot close the script tag", () => {
    const json = serializeJsonLd({ name: "</script><script>alert(1)</script>" });
    expect(json).not.toContain("<");
    expect(JSON.parse(json).name).toBe("</script><script>alert(1)</script>");
  });

  it("lists explore pages and every model in the sitemap entries", () => {
    vi.stubEnv("APP_URL", "https://tokscale.ai");
    const lastModified = new Date("2026-10-19T04:12:00.000Z");
    const entries = buildExploreSitemapEntries(["claude-opus-5", "gpt-5.6-sol"], lastModified);
    const urls = entries.map((entry) => entry.url);
    expect(urls).toContain("https://tokscale.ai/leaderboard?view=models");
    expect(urls).toContain("https://tokscale.ai/models");
    expect(urls).toContain("https://tokscale.ai/models/gpt-5.6-sol");
    expect(entries.find((e) => e.url.endsWith("/models/claude-opus-5"))?.lastModified).toEqual(lastModified);
    expect(entries.find((e) => e.url.endsWith("/methodology"))?.lastModified).toBeInstanceOf(Date);

    const undated = buildExploreSitemapEntries([], null);
    expect(undated.find((e) => e.url.endsWith("/models"))).not.toHaveProperty("lastModified");
  });
});
