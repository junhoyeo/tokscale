// @vitest-environment jsdom
import type { AnchorHTMLAttributes } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CommunitySnapshot as Snapshot } from "@/lib/explore";

const recordedLinks = vi.hoisted(() => [] as Array<{ href: string; prefetch?: boolean }>);

vi.mock("next/link", () => ({
  default: ({ href, prefetch, children, ...props }: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string; prefetch?: boolean }) => {
    recordedLinks.push({ href, prefetch });
    return <a href={href} {...props}>{children}</a>;
  },
}));

import { CommunitySnapshot } from "@/components/home/CommunitySnapshot";

const snapshot: Snapshot = {
  humans: [{
    rank: 1,
    username: "ada dev",
    displayName: "Ada",
    avatarUrl: null,
    tokens: 1_000,
    estimatedCost: 1,
    activeDays: 1,
    topModels: [],
    topAgents: [],
  }],
  models: [{
    rank: 1,
    key: "claude-opus-5",
    name: "Claude Opus 5",
    lab: "Anthropic",
    labSlug: "anthropic",
    logoUrl: null,
    activeUsers: 1,
    previousActiveUsers: 0,
    adoption: 1,
    tokens: 1_000,
    estimatedCost: 1,
    isNew: false,
    topAgents: [],
  }],
  agents: [],
  coverage: {
    period: "7d",
    start: "2026-11-01",
    end: "2026-11-07",
    accounts: 1,
    modelIdentifiedShare: 1,
    lastDataAt: "2026-11-07T00:00:00.000Z",
  },
};

describe("links to per-request detail pages", () => {
  beforeEach(() => recordedLinks.splice(0));

  it("do not prefetch a profile or a model, while links to list pages still do", () => {
    renderToStaticMarkup(<CommunitySnapshot snapshot={snapshot} />);

    expect(recordedLinks.find((link) => link.href === "/u/ada%20dev")?.prefetch).toBe(false);
    expect(recordedLinks.find((link) => link.href === "/models/claude-opus-5")?.prefetch).toBe(false);
    expect(recordedLinks.find((link) => link.href === "/leaderboard")?.prefetch).toBeUndefined();
    expect(recordedLinks.find((link) => link.href === "/models")?.prefetch).toBeUndefined();
  });
});
