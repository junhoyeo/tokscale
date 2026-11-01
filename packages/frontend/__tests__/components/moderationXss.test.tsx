/** @vitest-environment jsdom */

import React, { act } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ModerationClient from "@/app/admin/moderation/ModerationClient";
import { scoreCandidate, type CandidateRow } from "@/lib/moderation/heuristics";

vi.mock("@/components/layout/Navigation", () => ({ Navigation: () => <nav /> }));
vi.mock("@/components/layout/ServiceFooter", () => ({ ServiceFooter: () => <footer /> }));

const attack = '<img src=x onerror="globalThis.moderationXss=1"><svg onload="globalThis.moderationXss=2"></svg><script>globalThis.moderationXss=3</script>';

function candidate(overrides: Partial<CandidateRow> = {}) {
  return scoreCandidate({
    userId: "target-id",
    username: "target",
    avatarUrl: null,
    leaderboardHidden: true,
    totalTokens: 1_000_000,
    totalCost: 1,
    submitCount: 1,
    hasBackfill: false,
    dailyTokens: 1_000_000,
    nearDuplicateCount: 0,
    slopModels: [`fake-api-${attack}`],
    slopTokens: 1_000_000,
    hasOverNestedEntry: false,
    everyDayAttributed: true,
    attributedTokens: 1_000_000n,
    totalTokensExact: 1_000_000n,
    latestActorUsername: attack,
    latestAction: "hide",
    latestActionReason: attack,
    ...overrides,
  }, { siteTokens: 100_000_000, medianTokens: 1_000_000 });
}

function expectNoInjectedElements(container: HTMLElement) {
  expect(container.querySelectorAll("script, img, svg, iframe")).toHaveLength(0);
  expect(container.querySelectorAll("[onerror], [onload], [onclick]")).toHaveLength(0);
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("moderation stored XSS attack regressions", () => {
  it("renders model names, audit reasons and actor attributes as literal text", async () => {
    const data = candidate();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ candidates: [data] }),
    }));

    let rendered!: ReturnType<typeof render>;
    await act(async () => { rendered = render(<ModerationClient />); });

    const label = data.signals.find((signal) => signal.key === "slopModelName")!.label;
    expect(screen.getByText(label)).toBeDefined();
    expect(rendered.container.textContent).toContain(`Reason: “${attack}”`);
    expect(screen.getByLabelText("Admin Hidden").getAttribute("title")).toBe(`Hidden manually by @${attack}`);
    expectNoInjectedElements(rendered.container);
  });

  it("keeps an attacker-controlled username in a local profile path and encodes the mutation path", async () => {
    const username = 'javascript:alert(1)/"><img src=x onerror=alert(1)>';
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ candidates: [candidate({ username })] }),
    });
    vi.stubGlobal("fetch", fetchMock);

    let rendered!: ReturnType<typeof render>;
    await act(async () => { rendered = render(<ModerationClient />); });

    const link = screen.getByRole("link", { name: `@${username}` });
    expect(link.getAttribute("href")).toBe(`/u/${username}`);
    expect(new URL((link as HTMLAnchorElement).href).origin).toBe(window.location.origin);
    expect(link.getAttribute("rel")).toContain("noopener");
    expectNoInjectedElements(rendered.container);

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "Restored after review" } });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Unhide" })); });

    expect(fetchMock).toHaveBeenCalledWith(`/api/admin/moderation/${encodeURIComponent(username)}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ action: "unhide", reason: "Restored after review" }),
    });
    expectNoInjectedElements(rendered.container);
  });

  it("renders malicious API error text without parsing HTML", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: async () => ({ candidates: [candidate()] }) })
      .mockResolvedValueOnce({ ok: false, json: async () => ({ error: attack }) });
    vi.stubGlobal("fetch", fetchMock);

    let rendered!: ReturnType<typeof render>;
    await act(async () => { rendered = render(<ModerationClient />); });
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "Restored after review" } });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Unhide" })); });

    expect(screen.getByRole("alert").textContent).toBe(attack);
    expectNoInjectedElements(rendered.container);
  });
});
