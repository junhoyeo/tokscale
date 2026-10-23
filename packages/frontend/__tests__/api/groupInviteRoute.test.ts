import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const createGroupInvite = vi.fn();
  const listGroupInvites = vi.fn();
  const revokeGroupInvite = vi.fn();
  const revalidateGroupCaches = vi.fn();

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    createGroupInvite,
    listGroupInvites,
    revokeGroupInvite,
    revalidateGroupCaches,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      createGroupInvite.mockReset();
      listGroupInvites.mockReset();
      revokeGroupInvite.mockReset();
      revalidateGroupCaches.mockReset();
    },
  };
});

vi.mock("@/lib/auth/requestSession", () => ({
  getSessionFromRequest: mockState.getSessionFromRequest,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateGroupCaches: mockState.revalidateGroupCaches,
}));

vi.mock("@/lib/groups/invites", () => {
  class GroupInviteError extends Error {
    constructor(
      public readonly code: "not_found" | "forbidden" | "invalid",
      message: string
    ) {
      super(message);
    }
  }

  return {
    createGroupInvite: mockState.createGroupInvite,
    listGroupInvites: mockState.listGroupInvites,
    revokeGroupInvite: mockState.revokeGroupInvite,
    GroupInviteError,
  };
});

vi.mock("@/lib/groups/permissions", () => ({
  getGroupMembership: mockState.getGroupMembership,
}));

vi.mock("@/lib/groups/queries", () => ({
  getGroupBySlug: mockState.getGroupBySlug,
}));

vi.mock("../../src/lib/db/schema", () => ({
  groupRoles: ["owner", "admin", "member"],
}));

type ModuleExports = typeof import("../../src/app/api/groups/[slug]/invite/route");

let POST: ModuleExports["POST"];
let GET: ModuleExports["GET"];
let DELETE: ModuleExports["DELETE"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/invite/route");
  POST = routeModule.POST;
  GET = routeModule.GET;
  DELETE = routeModule.DELETE;
});

beforeEach(() => {
  mockState.reset();
  mockState.getSessionFromRequest.mockResolvedValue({
    id: "actor-1",
    username: "actor",
    displayName: null,
    avatarUrl: null,
  });
  mockState.getGroupBySlug.mockResolvedValue({
    id: "group-1",
    slug: "team",
    name: "Team",
    isPublic: false,
  });
  mockState.createGroupInvite.mockResolvedValue({
    id: "invite-1",
    token: "tg_token",
    role: "member",
    invitedUsername: null,
    expiresAt: new Date("2026-06-01T00:00:00Z"),
  });
});

function requestWithRole(role: string) {
  return new Request("http://localhost:3000/api/groups/team/invite", {
    method: "POST",
    body: JSON.stringify({ role }),
  });
}

describe("POST /api/groups/[slug]/invite", () => {
  it("forbids admins from creating admin invites", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });

    const response = await POST(requestWithRole("admin"), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(403);
    expect(await response.json()).toEqual({ error: "Forbidden" });
    expect(mockState.createGroupInvite).not.toHaveBeenCalled();
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });

  it("forbids owner invites", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });

    const response = await POST(requestWithRole("owner"), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(403);
    expect(await response.json()).toEqual({ error: "Forbidden" });
    expect(mockState.createGroupInvite).not.toHaveBeenCalled();
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });

  it("allows owners to create admin invites", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });
    mockState.createGroupInvite.mockResolvedValue({
      id: "invite-1",
      token: "tg_token",
      role: "admin",
      invitedUsername: null,
      expiresAt: new Date("2026-06-01T00:00:00Z"),
    });

    const response = await POST(requestWithRole("admin"), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(201);
    expect(mockState.createGroupInvite).toHaveBeenCalledWith({
      groupId: "group-1",
      invitedBy: "actor-1",
      role: "admin",
      invitedUsername: null,
      maxUses: 1,
      expiresAt: expect.any(Date),
    });
    expect(mockState.revalidateGroupCaches).toHaveBeenCalledWith("group-1", "team");
  });

  it("allows admins to create member invites", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });

    const response = await POST(requestWithRole("member"), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(201);
    expect(mockState.createGroupInvite).toHaveBeenCalledWith({
      groupId: "group-1",
      invitedBy: "actor-1",
      role: "member",
      invitedUsername: null,
      maxUses: 1,
      expiresAt: expect.any(Date),
    });
    expect(mockState.revalidateGroupCaches).toHaveBeenCalledWith("group-1", "team");
  });

  it("defaults invalid requested roles to member invites", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });

    const response = await POST(requestWithRole("invalid"), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(201);
    expect(mockState.createGroupInvite).toHaveBeenCalledWith({
      groupId: "group-1",
      invitedBy: "actor-1",
      role: "member",
      invitedUsername: null,
      maxUses: 1,
      expiresAt: expect.any(Date),
    });
  });
});


describe("invite limits and management routes", () => {
  const context = () => ({ params: Promise.resolve({ slug: "team" }) });
  it.each([{ maxUses: 0 }, { maxUses: 1001 }, { maxUses: 1.5 }, { maxUses: "3" }, { maxUses: 2, invitedUsername: "alice" }, { expiresInDays: 0 }, { expiresInDays: 31 }, { expiresInDays: "14" }])("rejects invalid policy %j", async (body) => {
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });
    const result = await POST(new Request("http://localhost/api/groups/team/invite", { method: "POST", body: JSON.stringify(body) }), context());
    expect(result.status).toBe(400);
    expect(mockState.createGroupInvite).not.toHaveBeenCalled();
  });
  it("passes the chosen capacity and expiry to the transactional service", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });
    const before = Date.now();
    const response = await POST(new Request("http://localhost/api/groups/team/invite", { method: "POST", body: JSON.stringify({ maxUses: 25, expiresInDays: 7 }) }), context());
    expect(response.status).toBe(201);
    const input = mockState.createGroupInvite.mock.calls[0][0];
    expect(input.maxUses).toBe(25);
    expect(input.expiresAt.getTime()).toBeGreaterThanOrEqual(before + 7 * 86400000);
    expect(input.expiresAt.getTime()).toBeLessThanOrEqual(Date.now() + 7 * 86400000);
  });
  it("lists managed invites without a public cache", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });
    mockState.listGroupInvites.mockResolvedValue([{ id: "invite", maxUses: 5, useCount: 2 }]);
    const response = await GET(new Request("http://localhost/api/groups/team/invite"), context());
    expect(response.status).toBe(200);
    expect(response.headers.get("cache-control")).toBe("no-store, private");
    expect(mockState.listGroupInvites).toHaveBeenCalledWith("group-1", "admin");
    expect(mockState.getSessionFromRequest).toHaveBeenCalledWith(expect.any(Request), { allowAuthorizationHeader: false });
  });
  it("revokes by ID within the authorized group", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });
    const inviteId = "11111111-1111-4111-8111-111111111111";
    const response = await DELETE(new Request("http://localhost/api/groups/team/invite", { method: "DELETE", body: JSON.stringify({ inviteId }) }), context());
    expect(response.status).toBe(200);
    expect(mockState.revokeGroupInvite).toHaveBeenCalledWith("group-1", inviteId, "actor-1");
    expect(mockState.getSessionFromRequest).toHaveBeenCalledWith(expect.any(Request), { allowAuthorizationHeader: false });
  });
  it.each(["GET", "DELETE"])("hides private groups from nonmembers on %s", async (method) => {
    mockState.getGroupMembership.mockResolvedValue(null);
    const response = await (method === "GET" ? GET : DELETE)(new Request("http://localhost/api/groups/team/invite", { method }), context());
    expect(response.status).toBe(404);
    expect(mockState.listGroupInvites).not.toHaveBeenCalled();
    expect(mockState.revokeGroupInvite).not.toHaveBeenCalled();
  });
  it.each(["GET", "DELETE"])("rejects regular members on %s", async (method) => {
    mockState.getGroupMembership.mockResolvedValue({ role: "member" });
    const response = await (method === "GET" ? GET : DELETE)(new Request("http://localhost/api/groups/team/invite", { method }), context());
    expect(response.status).toBe(403);
    expect(mockState.listGroupInvites).not.toHaveBeenCalled();
    expect(mockState.revokeGroupInvite).not.toHaveBeenCalled();
  });
  it("rejects a malformed revoke body before writing", async () => {
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });
    const response = await DELETE(new Request("http://localhost/api/groups/team/invite", { method: "DELETE", body: '{"inviteId":"bad"}' }), context());
    expect(response.status).toBe(400);
    expect(mockState.revokeGroupInvite).not.toHaveBeenCalled();
  });
});
