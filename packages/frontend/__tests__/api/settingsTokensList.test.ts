import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const listPersonalTokens = vi.fn();
  const issuePersonalToken = vi.fn();

  return {
    getSession,
    listPersonalTokens,
    issuePersonalToken,
    reset() {
      getSession.mockReset();
      listPersonalTokens.mockReset();
      issuePersonalToken.mockReset();
    },
  };
});

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  listPersonalTokens: mockState.listPersonalTokens,
  issuePersonalToken: mockState.issuePersonalToken,
}));

type ModuleExports = typeof import("../../src/app/api/settings/tokens/route");

let GET: ModuleExports["GET"];
let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/settings/tokens/route");
  GET = routeModule.GET;
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
});

describe("GET /api/settings/tokens", () => {
  it("returns 401 when user is not authenticated", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await GET();

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
  });

  it("returns 200 with empty token list when user has no tokens", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.listPersonalTokens.mockResolvedValue([]);

    const response = await GET();

    expect(response.status).toBe(200);
    expect(mockState.listPersonalTokens).toHaveBeenCalledWith("user-1");
    expect(await response.json()).toEqual({ tokens: [] });
  });

  it("returns 200 with mapped token list (excluding token, expiresAt, userId fields)", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.listPersonalTokens.mockResolvedValue([
      {
        id: "token-1",
        name: "My Token",
        token: "tt_secret_value",
        createdAt: new Date("2024-01-01"),
        lastUsedAt: new Date("2024-01-15"),
        expiresAt: new Date("2025-01-01"),
        userId: "user-1",
      },
      {
        id: "token-2",
        name: "Another Token",
        token: "tt_another_secret",
        createdAt: new Date("2024-02-01"),
        lastUsedAt: null,
        expiresAt: null,
        userId: "user-1",
      },
    ]);

    const response = await GET();

    expect(response.status).toBe(200);
    expect(mockState.listPersonalTokens).toHaveBeenCalledWith("user-1");
    const data = await response.json();
    expect(data).toEqual({
      tokens: [
        {
          id: "token-1",
          name: "My Token",
          createdAt: "2024-01-01T00:00:00.000Z",
          lastUsedAt: "2024-01-15T00:00:00.000Z",
        },
        {
          id: "token-2",
          name: "Another Token",
          createdAt: "2024-02-01T00:00:00.000Z",
          lastUsedAt: null,
        },
      ],
    });
    // Verify that token, expiresAt, and userId are not in the response
    expect(data.tokens[0]).not.toHaveProperty("token");
    expect(data.tokens[0]).not.toHaveProperty("expiresAt");
    expect(data.tokens[0]).not.toHaveProperty("userId");
  });

  it("returns 500 when listPersonalTokens throws an error", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.listPersonalTokens.mockRejectedValue(
      new Error("Database error")
    );

    const response = await GET();

    expect(response.status).toBe(500);
    expect(await response.json()).toEqual({ error: "Failed to fetch tokens" });
  });
});

describe("POST /api/settings/tokens", () => {
  it("returns 401 when user is not authenticated", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await POST(
      new Request("http://localhost:3000/api/settings/tokens", {
        method: "POST",
        body: JSON.stringify({ name: "CI" }),
      })
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.issuePersonalToken).not.toHaveBeenCalled();
  });

  it("creates a token and returns the raw token once", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.issuePersonalToken.mockResolvedValue({
      id: "token-1",
      userId: "user-1",
      name: "GitHub Actions",
      token: "tt_raw_token",
      createdAt: new Date("2026-05-01T10:00:00.000Z"),
      lastUsedAt: null,
      expiresAt: null,
    });

    const response = await POST(
      new Request("http://localhost:3000/api/settings/tokens", {
        method: "POST",
        body: JSON.stringify({ name: "  GitHub Actions  " }),
      })
    );

    expect(response.status).toBe(201);
    expect(mockState.issuePersonalToken).toHaveBeenCalledWith({
      userId: "user-1",
      name: "GitHub Actions",
      ensureUniqueName: true,
    });
    expect(await response.json()).toEqual({
      token: {
        id: "token-1",
        name: "GitHub Actions",
        token: "tt_raw_token",
        createdAt: "2026-05-01T10:00:00.000Z",
        lastUsedAt: null,
      },
    });
  });

  it("uses a safe default name when the request name is blank", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.issuePersonalToken.mockResolvedValue({
      id: "token-1",
      userId: "user-1",
      name: "CI token",
      token: "tt_raw_token",
      createdAt: new Date("2026-05-01T10:00:00.000Z"),
      lastUsedAt: null,
      expiresAt: null,
    });

    const response = await POST(
      new Request("http://localhost:3000/api/settings/tokens", {
        method: "POST",
        body: JSON.stringify({ name: "   " }),
      })
    );

    expect(response.status).toBe(201);
    expect(mockState.issuePersonalToken).toHaveBeenCalledWith({
      userId: "user-1",
      name: "CI token",
      ensureUniqueName: true,
    });
  });
});
