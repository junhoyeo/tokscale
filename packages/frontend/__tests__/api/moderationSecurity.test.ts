import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionUser } from "@/lib/auth/session";

const mocks = vi.hoisted(() => ({
  getSession: vi.fn(),
  getSessionFromHeader: vi.fn(),
  findModerationTarget: vi.fn(),
  getModerationHistory: vi.fn(),
  applyModerationAction: vi.fn(),
  getModerationCandidates: vi.fn(),
}));

// Keep the real request-session, CSRF, admin allowlist and moderation guard.
// Only resolving an already authenticated session and database work are stubbed.
vi.mock("@/lib/auth/session", () => ({
  getSession: mocks.getSession,
  getSessionFromHeader: mocks.getSessionFromHeader,
}));
vi.mock("@/lib/moderation/actions", () => ({
  findModerationTarget: mocks.findModerationTarget,
  getModerationHistory: mocks.getModerationHistory,
  applyModerationAction: mocks.applyModerationAction,
}));
vi.mock("@/lib/moderation/candidates", () => ({
  getModerationCandidates: mocks.getModerationCandidates,
}));

import {
  GET as getHistory,
  POST as moderate,
} from "@/app/api/admin/moderation/[username]/route";
import { GET as getCandidates } from "@/app/api/admin/moderation/candidates/route";

const origin = "https://tokscale.example";
const target = { id: "target-id", username: "target", leaderboardHidden: true };
const admin: SessionUser = {
  id: "real-admin-id",
  username: "real-admin",
  githubId: 123456,
  displayName: null,
  avatarUrl: null,
};

function mutation(
  headers: Record<string, string> = { Origin: origin },
  body: Record<string, unknown> = { action: "unhide", reason: "Restored after review" },
) {
  return new Request(`${origin}/api/admin/moderation/target`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
}

const params = () => ({ params: Promise.resolve({ username: target.username }) });

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubEnv("TOKSCALE_ADMIN_GITHUB_IDS", String(admin.githubId));
  vi.stubEnv("APP_URL", origin);
  vi.stubEnv("CSRF_ALLOWED_ORIGINS", origin);
  mocks.getSession.mockResolvedValue(null);
  mocks.getSessionFromHeader.mockResolvedValue(admin);
  mocks.findModerationTarget.mockResolvedValue(target);
  mocks.getModerationHistory.mockResolvedValue([]);
  mocks.getModerationCandidates.mockResolvedValue([]);
  mocks.applyModerationAction.mockResolvedValue({ changed: true, leaderboardHidden: false });
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

describe("moderation authorization attack regressions", () => {
  it.each(["absent", "ordinary user", "matching admin username", "personal-token identity"])(
    "rejects %s despite forged identity fields in headers and JSON",
    async (identity) => {
      const session = identity === "absent" ? null : {
        ...admin,
        id: "attacker-id",
        username: identity === "matching admin username" ? admin.username : "attacker",
        githubId: identity === "personal-token identity" ? null : 999999,
      };
      mocks.getSession.mockResolvedValue(session);

      const response = await moderate(mutation({
        Origin: origin,
        Authorization: "Bearer tt_admin-token",
        "X-User-Id": admin.id,
        "X-GitHub-Id": String(admin.githubId),
      }, {
        action: "unhide",
        reason: "Restored after review",
        actorUserId: admin.id,
        actorUsername: admin.username,
        githubId: admin.githubId,
        leaderboardHidden: false,
      }), params());

      expect(response.status).toBe(404);
      expect(mocks.getSessionFromHeader).not.toHaveBeenCalled();
      expect(mocks.findModerationTarget).not.toHaveBeenCalled();
      expect(mocks.applyModerationAction).not.toHaveBeenCalled();
    },
  );

  it.each([undefined, "https://attacker.example", `${origin}.attacker.example`, "null"])(
    "rejects an admin cookie mutation with Origin %s even with an Authorization header",
    async (attackOrigin) => {
      mocks.getSession.mockResolvedValue(admin);

      const response = await moderate(mutation({
        ...(attackOrigin ? { Origin: attackOrigin } : {}),
        Authorization: "Bearer tt_admin-token",
        "X-Forwarded-Host": "tokscale.example",
      }), params());

      expect(response.status).toBe(404);
      expect(mocks.getSession).not.toHaveBeenCalled();
      expect(mocks.getSessionFromHeader).not.toHaveBeenCalled();
      expect(mocks.applyModerationAction).not.toHaveBeenCalled();
    },
  );

  it("does not allow a bearer token to read the review queue or history", async () => {
    const headers = { Authorization: "Bearer tt_admin-token" };
    const queue = await getCandidates(new Request(`${origin}/api/admin/moderation/candidates`, { headers }));
    const history = await getHistory(new Request(`${origin}/api/admin/moderation/target`, { headers }), params());

    expect(queue.status).toBe(404);
    expect(history.status).toBe(404);
    expect(mocks.getSessionFromHeader).not.toHaveBeenCalled();
    expect(mocks.getModerationCandidates).not.toHaveBeenCalled();
    expect(mocks.findModerationTarget).not.toHaveBeenCalled();
  });

  it.each(["", "123456,attacker", "123456,1e3", "123456,0x10"])(
    "fails closed on an empty or malformed admin allowlist: %s",
    async (allowlist) => {
      vi.stubEnv("TOKSCALE_ADMIN_GITHUB_IDS", allowlist);
      vi.spyOn(console, "error").mockImplementation(() => {});
      mocks.getSession.mockResolvedValue(admin);

      const response = await moderate(mutation(), params());

      expect(response.status).toBe(404);
      expect(mocks.applyModerationAction).not.toHaveBeenCalled();
    },
  );

  it("takes the actor and target from authenticated server state and discards body overrides", async () => {
    mocks.getSession.mockResolvedValue(admin);
    const response = await moderate(mutation({ Origin: origin }, {
      action: "unhide",
      reason: "  Restored after review  ",
      actorUserId: "forged-admin-id",
      actorUsername: "system",
      target: { id: "another-target", username: "another-user" },
      targetUserId: "another-target",
      leaderboardHidden: true,
    }), params());

    expect(response.status).toBe(200);
    expect(mocks.applyModerationAction).toHaveBeenCalledExactlyOnceWith({
      target,
      actorUserId: admin.id,
      actorUsername: admin.username,
      action: "unhide",
      reason: "Restored after review",
    });
  });

  it.each([
    { action: "unhide'; DROP TABLE users;--", reason: "Restored" },
    { action: "unhide", reason: " " },
    { action: "unhide", reason: "x".repeat(501) },
    { action: "unhide", reason: { $ne: "" } },
  ])("rejects malformed action data before selecting or changing a target: %j", async (body) => {
    mocks.getSession.mockResolvedValue(admin);

    const response = await moderate(mutation({ Origin: origin }, body), params());

    expect(response.status).toBe(400);
    expect(mocks.findModerationTarget).not.toHaveBeenCalled();
    expect(mocks.applyModerationAction).not.toHaveBeenCalled();
  });
});
