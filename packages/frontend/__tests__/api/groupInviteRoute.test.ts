import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const createGroupInvite = vi.fn();
  const revalidateGroupCaches = vi.fn();

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    createGroupInvite,
    revalidateGroupCaches,
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

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/invite/route");
  POST = routeModule.POST;
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
    });
  });
});
