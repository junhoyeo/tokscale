import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  SITE_URL,
  getPublicOrigin,
  groupUrl,
  homeUrl,
  leaderboardUrl,
  profileUrl,
} from "@/lib/seo/urls";
import { getRootMetadata } from "@/lib/seo/rootMetadata";

afterEach(() => vi.unstubAllEnvs());

describe("SITE_URL", () => {
  it("uses the hosted default when no runtime APP_URL is configured", () => {
    expect(SITE_URL).toBe("https://tokscale.ai");
  });

  it("has no trailing slash, so `${SITE_URL}/path` never doubles it", () => {
    expect(SITE_URL.endsWith("/")).toBe(false);
  });
});

describe("root metadata", () => {
  it("uses absolute runtime-origin Open Graph and Twitter images", () => {
    const metadata = getRootMetadata("https://runtime.example");
    expect(metadata.metadataBase?.toString()).toBe("https://runtime.example/");
    expect(metadata.openGraph?.url).toBe("https://runtime.example");
    expect(metadata.openGraph?.images).toEqual(expect.arrayContaining([
      expect.objectContaining({ url: "https://runtime.example/og-image.png" }),
    ]));
    expect(metadata.twitter?.images).toContain("https://runtime.example/og-image.png");
  });
});

describe("getPublicOrigin", () => {
  it("normalizes a self-hosted public URL to its http(s) origin", () => {
    expect(getPublicOrigin("https://tokscale.example.com/a/path/")).toBe(
      "https://tokscale.example.com"
    );
  });

  it("falls back to the hosted origin for malformed or non-http URLs", () => {
    expect(getPublicOrigin("not a URL")).toBe("https://tokscale.ai");
    expect(getPublicOrigin("file:///tmp/tokscale")).toBe("https://tokscale.ai");
  });

  it("uses APP_URL at runtime rather than a build-inlined NEXT_PUBLIC_URL", () => {
    vi.stubEnv("APP_URL", "https://runtime.example");
    vi.stubEnv("NEXT_PUBLIC_URL", "https://build.example");
    expect(getPublicOrigin()).toBe("https://runtime.example");
  });

  it("uses localhost by default for development and hosted origin for production", () => {
    vi.stubEnv("NODE_ENV", "development");
    vi.unstubAllEnvs();
    vi.stubEnv("NODE_ENV", "development");
    expect(getPublicOrigin()).toBe("http://localhost:3000");
    vi.stubEnv("NODE_ENV", "production");
    expect(getPublicOrigin()).toBe("https://tokscale.ai");
  });

  it("keeps the frontend environment template aligned with the runtime resolver", () => {
    const template = readFileSync(resolve(__dirname, "../../.env.example"), "utf8");
    expect(template).toContain("APP_URL=http://localhost:3000");
    expect(template).not.toMatch(/^NEXT_PUBLIC_URL=/m);
  });
});

describe("homeUrl", () => {
  it("is the bare origin, which both consumers emit verbatim", () => {
    // Verified against prod: the sitemap <loc> and the page's canonical tag
    // both render exactly this, with no trailing slash added by either.
    expect(homeUrl()).toBe("https://tokscale.ai");
  });
});

describe("leaderboardUrl", () => {
  it("defaults to the bare URL that every filter param collapses onto", () => {
    expect(leaderboardUrl()).toBe("https://tokscale.ai/leaderboard");
  });

  it("keeps view=groups, which selects a different page rather than a filter", () => {
    // /groups permanently redirects here, so this is the groups browser's only
    // real URL — collapsing it into the user leaderboard would deindex it.
    expect(leaderboardUrl("groups")).toBe(
      "https://tokscale.ai/leaderboard?view=groups"
    );
  });

  it("carries no filter param on either variant", () => {
    for (const url of [leaderboardUrl(), leaderboardUrl("groups")]) {
      for (const param of ["period=", "sortBy=", "page=", "search=", "from=", "to="]) {
        expect(url).not.toContain(param);
      }
    }
  });
});

describe("profileUrl", () => {
  it("drops the period param so all three windows consolidate", () => {
    expect(profileUrl("junhoyeo")).toBe("https://tokscale.ai/u/junhoyeo");
  });

  it("preserves casing, since /u/[username] redirects any other spelling", () => {
    expect(profileUrl("JunhoYeo")).toBe("https://tokscale.ai/u/JunhoYeo");
  });

  it("percent-encodes so an unexpected value cannot emit a malformed URL", () => {
    expect(profileUrl("a b/c?d")).toBe("https://tokscale.ai/u/a%20b%2Fc%3Fd");
  });
});

describe("groupUrl", () => {
  it("builds the group detail URL from the slug", () => {
    expect(groupUrl("anthropic")).toBe("https://tokscale.ai/groups/anthropic");
  });

  it("percent-encodes the slug", () => {
    expect(groupUrl("a b/c")).toBe("https://tokscale.ai/groups/a%20b%2Fc");
  });
});
