/** @vitest-environment jsdom */

import React, { act } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import ModerationClient from "@/app/admin/moderation/ModerationClient";

// @ts-expect-error React act environment flag for jsdom
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@/components/layout/Navigation", () => ({
  Navigation: () => <nav data-testid="mock-nav" />,
}));

vi.mock("@/components/layout/ServiceFooter", () => ({
  ServiceFooter: () => <footer data-testid="mock-footer" />,
}));

describe("ModerationClient", () => {
  const originalFetch = global.fetch;

  afterEach(() => {
    cleanup();
    global.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it("renders accurate copy excluding hidden accounts from rankings and site-wide totals", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ candidates: [] }),
    });

    await act(async () => {
      render(<ModerationClient />);
    });

    expect(
      screen.getByText(/Hiding removes an account from both rankings and site-wide totals\./i)
    ).toBeDefined();
    expect(
      screen.queryByText(/their usage still counts toward site-wide totals/i)
    ).toBeNull();
  });

  it("distinguishes system-withheld and admin-hidden candidates with appropriate badges and metadata", async () => {
    const mockCandidates = [
      {
        userId: "user-system",
        username: "system_withheld_user",
        leaderboardHidden: true,
        totalTokens: 10_000_000_000,
        totalCost: 50,
        submitCount: 2,
        hasBackfill: false,
        score: 75,
        signals: [
          {
            key: "impliedRate",
            label: "Implied $0.005/token is above list price",
            weight: 15,
          },
        ],
        latestActorUsername: "system",
        latestAction: "hide",
        latestActionReason: "Under investigation",
      },
      {
        userId: "user-admin",
        username: "admin_hidden_user",
        leaderboardHidden: true,
        totalTokens: 5_000_000,
        totalCost: 20,
        submitCount: 3,
        hasBackfill: false,
        score: 30,
        signals: [],
        latestActorUsername: "alice_admin",
        latestAction: "hide",
        latestActionReason: "Abuse",
      },
      {
        userId: "user-normal",
        username: "normal_flagged_user",
        leaderboardHidden: false,
        totalTokens: 1_000_000,
        totalCost: 5,
        submitCount: 1,
        hasBackfill: false,
        score: 20,
        signals: [
          {
            key: "dailyMismatch",
            label: "Stored total is 12x the sum of daily rows",
            weight: 20,
          },
        ],
        latestActorUsername: null,
        latestAction: null,
        latestActionReason: null,
      },
    ];

    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ candidates: mockCandidates }),
    });

    await act(async () => {
      render(<ModerationClient />);
    });

    // Verify system-withheld user
    const systemBadge = screen.getByLabelText("System Withheld");
    expect(systemBadge).toBeDefined();
    expect(systemBadge.getAttribute("data-status")).toBe("system-withheld");
    expect(systemBadge.textContent).toBe("system withheld");
    expect(screen.getByText("[System Withheld]")).toBeDefined();
    expect(screen.getByText(/Automatically withheld by system/i)).toBeDefined();
    expect(screen.getByText(/“Under investigation”/)).toBeDefined();

    // Verify admin-hidden user
    const adminBadge = screen.getByLabelText("Admin Hidden");
    expect(adminBadge).toBeDefined();
    expect(adminBadge.getAttribute("data-status")).toBe("admin-hidden");
    expect(adminBadge.textContent).toBe("admin hidden");
    expect(screen.getByText("[Admin Hidden]")).toBeDefined();
    expect(screen.getByText(/Hidden manually by @alice_admin/i)).toBeDefined();
    expect(screen.getByText(/“Abuse”/)).toBeDefined();

    // Verify normal non-hidden user does not add extra hidden badges
    expect(screen.getByText("@normal_flagged_user")).toBeDefined();
    expect(screen.getAllByText(/Hidden manually by/i)).toHaveLength(1);
    expect(screen.getAllByText(/Automatically withheld by/i)).toHaveLength(1);
  });

  it("applies moderation action with selected reason and authenticated admin", async () => {
    const mockCandidates = [
      {
        userId: "user-system",
        username: "system_withheld_user",
        leaderboardHidden: true,
        totalTokens: 10_000_000_000,
        totalCost: 50,
        submitCount: 2,
        hasBackfill: false,
        score: 75,
        signals: [],
        latestActorUsername: "system",
        latestAction: "hide",
        latestActionReason: "Under investigation",
      },
    ];

    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ candidates: mockCandidates }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ username: "system_withheld_user", leaderboardHidden: false, changed: true }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ candidates: [] }),
      });

    global.fetch = fetchMock;

    await act(async () => {
      render(<ModerationClient />);
    });

    const select = screen.getByLabelText("Reason for changing @system_withheld_user");
    const unhideBtn = screen.getByRole("button", { name: "Unhide" });

    // Try submitting without reason: should error client-side
    await act(async () => {
      fireEvent.click(unhideBtn);
    });

    expect(screen.getByText(/Select a reason before changing @system_withheld_user/i)).toBeDefined();

    // Select reason and submit
    await act(async () => {
      fireEvent.change(select, { target: { value: "Restored after review" } });
    });

    await act(async () => {
      fireEvent.click(unhideBtn);
    });

    // Check POST call
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/admin/moderation/system_withheld_user",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          action: "unhide",
          reason: "Restored after review",
        }),
      })
    );
  });

  it("handles case-insensitive system username and legacy hidden accounts with null metadata", async () => {
    const mockCandidates = [
      {
        userId: "user-system-upper",
        username: "system_user_upper",
        leaderboardHidden: true,
        totalTokens: 10_000_000,
        totalCost: 50,
        submitCount: 2,
        hasBackfill: false,
        score: 75,
        signals: [],
        latestActorUsername: "SYSTEM",
        latestAction: "hide",
        latestActionReason: "Under investigation",
      },
      {
        userId: "user-legacy",
        username: "legacy_hidden_user",
        leaderboardHidden: true,
        totalTokens: 5_000_000,
        totalCost: 20,
        submitCount: 3,
        hasBackfill: false,
        score: 30,
        signals: [],
        latestActorUsername: null,
        latestAction: null,
        latestActionReason: null,
      },
    ];

    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ candidates: mockCandidates }),
    });

    await act(async () => {
      render(<ModerationClient />);
    });

    // Verify uppercase "SYSTEM" is correctly identified as system withheld
    const systemBadge = screen.getByLabelText("System Withheld");
    expect(systemBadge).toBeDefined();
    expect(systemBadge.textContent).toBe("system withheld");
    expect(screen.getByText("[System Withheld]")).toBeDefined();

    // Verify legacy hidden account defaults to admin hidden with generic administrator text
    const adminBadge = screen.getByLabelText("Admin Hidden");
    expect(adminBadge).toBeDefined();
    expect(adminBadge.textContent).toBe("admin hidden");
    expect(screen.getByText("[Admin Hidden]")).toBeDefined();
    expect(screen.getByText(/Hidden manually by administrator/i)).toBeDefined();
  });

  it("applies hide action from rankings for unhidden flagged candidate", async () => {
    const mockCandidates = [
      {
        userId: "user-spoofer",
        username: "spoofer_target",
        leaderboardHidden: false,
        totalTokens: 8_000_000,
        totalCost: 40,
        submitCount: 1,
        hasBackfill: false,
        score: 50,
        signals: [
          {
            key: "slopModelName",
            label: "Model name resembles slop",
            weight: 35,
          },
        ],
        latestActorUsername: null,
        latestAction: null,
        latestActionReason: null,
      },
    ];

    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ candidates: mockCandidates }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ username: "spoofer_target", leaderboardHidden: true, changed: true }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ candidates: [] }),
      });

    global.fetch = fetchMock;

    await act(async () => {
      render(<ModerationClient />);
    });

    const select = screen.getByLabelText("Reason for changing @spoofer_target");
    const hideBtn = screen.getByRole("button", { name: "Hide from rankings" });

    await act(async () => {
      fireEvent.change(select, { target: { value: "Abuse" } });
    });

    await act(async () => {
      fireEvent.click(hideBtn);
    });

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/admin/moderation/spoofer_target",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          action: "hide",
          reason: "Abuse",
        }),
      })
    );
  });
});
