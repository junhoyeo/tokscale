import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * B5: Cache-Control header on invite create response.
 * B7: Member-role redundancy regression — member trying to invite a member returns 403.
 */

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const createGroupInvite = vi.fn();
  const revalidateGroupCaches = vi.fn();
  const GroupInviteError = class extends Error {
    code: string;
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  };

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    createGroupInvite,
    revalidateGroupCaches,
    GroupInviteError,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      createGroupInvite.mockReset();
      revalidateGroupCaches.mockReset();
    },
  };
});

vi.mock("@/lib/auth/requestSession", () => ({
  getSessionFromRequest: mockState.getSessionFromRequest,
}));

vi.mock("@/lib/groups/invites", () => ({
  createGroupInvite: mockState.createGroupInvite,
  GroupInviteError: mockState.GroupInviteError,
}));

vi.mock("@/lib/groups/permissions", () => ({
  getGroupMembership: mockState.getGroupMembership,
}));

vi.mock("@/lib/groups/queries", () => ({
  getGroupBySlug: mockState.getGroupBySlug,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateGroupCaches: mockState.revalidateGroupCaches,
}));

type ModuleExports = typeof import("../../src/app/api/groups/[slug]/invite/route");

let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/invite/route");
  POST = routeModule.POST;
});

beforeEach(() => mockState.reset());

function session() {
  return { id: "user-1", username: "alice", displayName: null, avatarUrl: null };
}

function group() {
  return {
    id: "group-1",
    slug: "team",
    name: "Team",
    isPublic: true,
    createdBy: "user-2",
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    description: null,
    avatarUrl: null,
  };
}

function invite() {
  return {
    id: "invite-1",
    token: "invite-token",
    role: "member",
    invitedUsername: null,
    expiresAt: new Date("2026-02-01T00:00:00.000Z"),
  };
}

// ─── B5: Cache-Control ────────────────────────────────────────────────────────

describe("POST /api/groups/[slug]/invite — Cache-Control header (B5)", () => {
  it("returns Cache-Control: no-store, private on successful invite creation", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(session());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });
    mockState.createGroupInvite.mockResolvedValue(invite());

    const response = await POST(
      new Request("http://localhost:3000/api/groups/team/invite", {
        method: "POST",
        body: JSON.stringify({ role: "member" }),
        headers: { "Content-Type": "application/json" },
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(201);
    const cacheControl = response.headers.get("Cache-Control");
    expect(cacheControl).toBe("no-store, private");

    // Token must still be present in the response body
    const body = await response.json();
    expect(body.invite.token).toBe("invite-token");
    expect(body.joinUrl).toBe("/groups/join/invite-token");
  });
});

// ─── B7: Member-role redundancy regression ────────────────────────────────────

describe("POST /api/groups/[slug]/invite — member cannot invite (B7)", () => {
  it("returns 403 when a member tries to create any invite", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(session());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "member" });

    const response = await POST(
      new Request("http://localhost:3000/api/groups/team/invite", {
        method: "POST",
        body: JSON.stringify({ role: "member" }),
        headers: { "Content-Type": "application/json" },
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(403);
    expect(mockState.createGroupInvite).not.toHaveBeenCalled();
  });

  it("returns 403 when a member tries to invite an admin (should also be rejected)", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(session());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "member" });

    const response = await POST(
      new Request("http://localhost:3000/api/groups/team/invite", {
        method: "POST",
        body: JSON.stringify({ role: "admin" }),
        headers: { "Content-Type": "application/json" },
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(403);
    expect(mockState.createGroupInvite).not.toHaveBeenCalled();
  });
});
