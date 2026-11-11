import { describe, expect, it } from "vitest";
import { redactAnalyticsUrl } from "@/lib/analytics/redact";

describe("redactAnalyticsUrl", () => {
  it("redacts group invite tokens", () => {
    expect(
      redactAnalyticsUrl("https://tokscale.ai/groups/join/secret-token?utm_source=discord")
    ).toBe("https://tokscale.ai/groups/join/[token]?utm_source=discord");
  });

  it("redacts private group slugs but keeps group index paths", () => {
    expect(redactAnalyticsUrl("https://tokscale.ai/groups/private-team")).toBe(
      "https://tokscale.ai/groups/[slug]"
    );
    expect(redactAnalyticsUrl("https://tokscale.ai/groups")).toBe(
      "https://tokscale.ai/groups"
    );
    expect(redactAnalyticsUrl("https://tokscale.ai/groups/new")).toBe(
      "https://tokscale.ai/groups/new"
    );
  });

  it("keeps public profile paths", () => {
    expect(redactAnalyticsUrl("https://tokscale.ai/u/octocat?period=30d")).toBe(
      "https://tokscale.ai/u/octocat?period=30d"
    );
  });

  it("keeps only campaign and view query parameters", () => {
    expect(
      redactAnalyticsUrl(
        "https://tokscale.ai/leaderboard?utm_source=x&utm_medium=social&utm_campaign=launch&utm_term=agents&utm_content=card&view=models&period=30d&search=alice&q=secret&returnTo=%2Fsettings&code=oauth&state=csrf&future=value#results"
      )
    ).toBe(
      "https://tokscale.ai/leaderboard?utm_source=x&utm_medium=social&utm_campaign=launch&utm_term=agents&utm_content=card&view=models&period=30d"
    );
  });

  it("returns malformed input without its query or hash", () => {
    expect(redactAnalyticsUrl("not a URL?search=alice#result")).toBe("not a URL");
  });
});
