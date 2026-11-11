import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NextResponse } from "next/server";

const {
  mockRequireAdminSession,
  mockFindModerationTarget,
  mockGetModerationHistory,
  mockApplyModerationAction,
  mockGetModerationCandidates,
} = vi.hoisted(() => ({
  mockRequireAdminSession: vi.fn(),
  mockFindModerationTarget: vi.fn(),
  mockGetModerationHistory: vi.fn(),
  mockApplyModerationAction: vi.fn(),
  mockGetModerationCandidates: vi.fn(),
}));

vi.mock("@/lib/moderation/guard", () => ({
  requireAdminSession: mockRequireAdminSession,
}));

vi.mock("@/lib/moderation/actions", () => ({
  findModerationTarget: mockFindModerationTarget,
  getModerationHistory: mockGetModerationHistory,
  applyModerationAction: mockApplyModerationAction,
}));

vi.mock("@/lib/moderation/candidates", () => ({
  getModerationCandidates: mockGetModerationCandidates,
}));

import {
  GET as getUsernameRoute,
  POST as postUsernameRoute,
} from "@/app/api/admin/moderation/[username]/route";
import { GET as getCandidatesRoute } from "@/app/api/admin/moderation/candidates/route";

describe("Admin Moderation API Routes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("GET /api/admin/moderation/[username]", () => {
    it("returns 404 for non-admin callers to conceal the admin surface", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        response: NextResponse.json({ error: "Not found" }, { status: 404 }),
      });

      const response = await getUsernameRoute(
        new Request("https://tokscale.ai/api/admin/moderation/someuser"),
        { params: Promise.resolve({ username: "someuser" }) }
      );

      expect(response.status).toBe(404);
      expect(mockFindModerationTarget).not.toHaveBeenCalled();
    });

    it("returns 404 when target user is not found", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        session: { id: "admin-1", username: "superadmin" },
      });
      mockFindModerationTarget.mockResolvedValueOnce(null);

      const response = await getUsernameRoute(
        new Request("https://tokscale.ai/api/admin/moderation/nonexistent"),
        { params: Promise.resolve({ username: "nonexistent" }) }
      );

      expect(response.status).toBe(404);
      const json = await response.json();
      expect(json.error).toBe("User not found");
    });

    it("returns user moderation status and history for admin", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        session: { id: "admin-1", username: "superadmin" },
      });
      mockFindModerationTarget.mockResolvedValueOnce({
        id: "user-uuid-1",
        username: "targetuser",
        leaderboardHidden: true,
      });
      mockGetModerationHistory.mockResolvedValueOnce([
        {
          id: "action-1",
          action: "hide",
          reason: "Under investigation",
          createdAt: new Date("2026-10-18T10:00:00Z"),
          actorUsername: "system",
        },
      ]);

      const response = await getUsernameRoute(
        new Request("https://tokscale.ai/api/admin/moderation/targetuser"),
        { params: Promise.resolve({ username: "targetuser" }) }
      );

      expect(response.status).toBe(200);
      const json = await response.json();
      expect(json.user).toEqual({
        username: "targetuser",
        leaderboardHidden: true,
      });
      expect(json.history).toHaveLength(1);
      expect(json.history[0].actorUsername).toBe("system");
    });
  });

  describe("POST /api/admin/moderation/[username]", () => {
    it("returns 404 for non-admin callers", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        response: NextResponse.json({ error: "Not found" }, { status: 404 }),
      });

      const response = await postUsernameRoute(
        new Request("https://tokscale.ai/api/admin/moderation/targetuser", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action: "unhide", reason: "Restored" }),
        }),
        { params: Promise.resolve({ username: "targetuser" }) }
      );

      expect(response.status).toBe(404);
      expect(mockApplyModerationAction).not.toHaveBeenCalled();
    });

    it("returns 400 when reason is missing or invalid", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        session: { id: "admin-1", username: "superadmin" },
      });

      const response = await postUsernameRoute(
        new Request("https://tokscale.ai/api/admin/moderation/targetuser", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action: "unhide", reason: "   " }),
        }),
        { params: Promise.resolve({ username: "targetuser" }) }
      );

      expect(response.status).toBe(400);
      const json = await response.json();
      expect(json.error).toMatch(/reason is required/i);
    });

    it("returns 404 when target user is not found", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        session: { id: "admin-1", username: "superadmin" },
      });
      mockFindModerationTarget.mockResolvedValueOnce(null);

      const response = await postUsernameRoute(
        new Request("https://tokscale.ai/api/admin/moderation/targetuser", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action: "unhide", reason: "Restored after review" }),
        }),
        { params: Promise.resolve({ username: "targetuser" }) }
      );

      expect(response.status).toBe(404);
      expect(mockApplyModerationAction).not.toHaveBeenCalled();
    });

    it("applies moderation action with session admin credentials and returns result", async () => {
      const adminSession = { id: "admin-uuid-99", username: "admin_tester" };
      mockRequireAdminSession.mockResolvedValueOnce({ session: adminSession });

      const targetUser = {
        id: "target-uuid-1",
        username: "system_withheld_target",
        leaderboardHidden: true,
      };
      mockFindModerationTarget.mockResolvedValueOnce(targetUser);

      mockApplyModerationAction.mockResolvedValueOnce({
        changed: true,
        leaderboardHidden: false,
      });

      const response = await postUsernameRoute(
        new Request("https://tokscale.ai/api/admin/moderation/system_withheld_target", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            action: "unhide",
            reason: "Restored after review",
          }),
        }),
        { params: Promise.resolve({ username: "system_withheld_target" }) }
      );

      expect(response.status).toBe(200);
      const json = await response.json();
      expect(json).toEqual({
        username: "system_withheld_target",
        leaderboardHidden: false,
        changed: true,
      });

      expect(mockApplyModerationAction).toHaveBeenCalledWith({
        target: targetUser,
        actorUserId: "admin-uuid-99",
        actorUsername: "admin_tester",
        action: "unhide",
        reason: "Restored after review",
      });
    });
  });

  describe("GET /api/admin/moderation/candidates", () => {
    it("returns 404 for non-admin callers", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        response: NextResponse.json({ error: "Not found" }, { status: 404 }),
      });

      const response = await getCandidatesRoute(
        new Request("https://tokscale.ai/api/admin/moderation/candidates")
      );

      expect(response.status).toBe(404);
      expect(mockGetModerationCandidates).not.toHaveBeenCalled();
    });

    it("returns candidates list for authenticated admin", async () => {
      mockRequireAdminSession.mockResolvedValueOnce({
        session: { id: "admin-1", username: "superadmin" },
      });

      const mockCandidates = [
        {
          userId: "user-1",
          username: "flagged_spoofer",
          leaderboardHidden: true,
          score: 80,
          latestActorUsername: "system",
          latestAction: "hide",
          latestActionReason: "Under investigation",
        },
      ];
      mockGetModerationCandidates.mockResolvedValueOnce(mockCandidates);

      const response = await getCandidatesRoute(
        new Request("https://tokscale.ai/api/admin/moderation/candidates")
      );

      expect(response.status).toBe(200);
      const json = await response.json();
      expect(json.candidates).toEqual(mockCandidates);
    });
  });
});
