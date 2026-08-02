import { describe, expect, it } from "vitest";

import {
  SITE_URL,
  groupUrl,
  homeUrl,
  leaderboardUrl,
  profileUrl,
} from "@/lib/seo/urls";

describe("SITE_URL", () => {
  it("matches the metadataBase declared in app/layout.tsx", () => {
    // A canonical whose host differs from metadataBase resolves to a URL the
    // page does not live at, which drops it from the index.
    expect(SITE_URL).toBe("https://tokscale.ai");
  });

  it("has no trailing slash, so `${SITE_URL}/path` never doubles it", () => {
    expect(SITE_URL.endsWith("/")).toBe(false);
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
