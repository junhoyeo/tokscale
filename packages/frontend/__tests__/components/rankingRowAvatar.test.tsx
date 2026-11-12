import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children: React.ReactNode }) => <a href={href}>{children}</a>,
}));

import { MobileRankingRow } from "@/components/leaderboard/RankingUI";

describe("MobileRankingRow", () => {
  it("loads its avatar lazily, at the row size", () => {
    const markup = renderToStaticMarkup(
      <MobileRankingRow
        rank={4}
        href="/u/octocat"
        avatarUrl="https://avatars.githubusercontent.com/u/1234?v=4"
        username="octocat"
        displayName="Octo Cat"
        primaryLabel="Tokens"
        primaryValue="1.2B"
        meta="3 active days"
        isCurrentUser={false}
      />,
    );

    const img = markup.match(/<img[^>]*>/)?.[0] ?? "";
    expect(img).toContain('src="https://avatars.githubusercontent.com/u/1234?v=4&amp;s=76"');
    expect(img).toContain('loading="lazy"');
  });
});
