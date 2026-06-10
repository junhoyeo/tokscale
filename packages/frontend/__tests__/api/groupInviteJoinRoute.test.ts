import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupInvitePreview = vi.fn();
  const acceptGroupInvite = vi.fn();

  return {
    getSessionFromRequest,
    getGroupInvitePreview,
    acceptGroupInvite,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupInvitePreview.mockReset();
      acceptGroupInvite.mockReset();
    },
  };
});

vi.mock("@/lib/auth/requestSession", () => ({
  getSessionFromRequest: mockState.getSessionFromRequest,
}));

vi.mock("@/lib/groups/invites", () => ({
  getGroupInvitePreview: mockState.getGroupInvitePreview,
  acceptGroupInvite: mockState.acceptGroupInvite,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateGroupCaches: vi.fn(),
}));

type ModuleExports = typeof import("../../src/app/api/groups/join/[token]/route");

let GET: ModuleExports["GET"];
let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/join/[token]/route");
  GET = routeModule.GET;
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
});

function session() {
  return { id: "user-1", username: "Alice", displayName: null, avatarUrl: null };
}

function mockBrowserSessionOnly() {
  mockState.getSessionFromRequest.mockImplementation(
    async (
      request: Request,
      options?: { allowAuthorizationHeader?: boolean }
    ) => {
      if (
        request.headers.has("Authorization") &&
        options?.allowAuthorizationHeader === false
      ) {
        return null;
      }

      return session();
    }
  );
}

describe("/api/groups/join/[token]", () => {
  it("shows a safe invite preview without requiring authentication", async () => {
    mockState.getGroupInvitePreview.mockResolvedValue({
      group: { name: "Team", slug: "team", isPublic: false },
      role: "member",
      invitedUsername: null,
      expiresAt: "2026-06-01T00:00:00.000Z",
    });

    const response = await GET(
      new Request("http://localhost:3000/api/groups/join/tg_token"),
      { params: Promise.resolve({ token: "tg_token" }) }
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      group: { name: "Team", slug: "team", isPublic: false },
      role: "member",
      invitedUsername: null,
      expiresAt: "2026-06-01T00:00:00.000Z",
    });
  });

  it("requires authentication before accepting an invite", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(null);

    const response = await POST(
      new Request("http://localhost:3000/api/groups/join/tg_token", { method: "POST" }),
      { params: Promise.resolve({ token: "tg_token" }) }
    );

    expect(response.status).toBe(401);
    expect(mockState.acceptGroupInvite).not.toHaveBeenCalled();
  });

  it("rejects Authorization header sessions when accepting an invite", async () => {
    mockBrowserSessionOnly();
    mockState.acceptGroupInvite.mockResolvedValue({
      group: { id: "group-1", name: "Team", slug: "team" },
      role: "member",
    });

    const response = await POST(
      new Request("http://localhost:3000/api/groups/join/tg_token", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_personal",
          Origin: "http://localhost:3000",
        },
      }),
      { params: Promise.resolve({ token: "tg_token" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.acceptGroupInvite).not.toHaveBeenCalled();
  });

  it("accepts an invite for the current user", async () => {
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "user-1",
      username: "Alice",
      displayName: null,
      avatarUrl: null,
    });
    mockState.acceptGroupInvite.mockResolvedValue({
      group: { id: "group-1", name: "Team", slug: "team" },
      role: "member",
    });

    const response = await POST(
      new Request("http://localhost:3000/api/groups/join/tg_token", { method: "POST" }),
      { params: Promise.resolve({ token: "tg_token" }) }
    );

    expect(response.status).toBe(200);
    expect(mockState.acceptGroupInvite).toHaveBeenCalledWith("tg_token", {
      id: "user-1",
      username: "Alice",
      displayName: null,
      avatarUrl: null,
    });
    expect(await response.json()).toEqual({
      group: { id: "group-1", name: "Team", slug: "team" },
      role: "member",
    });
  });
});
