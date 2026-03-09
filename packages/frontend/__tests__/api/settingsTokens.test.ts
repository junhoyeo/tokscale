import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const listPersonalTokens = vi.fn();
  const issuePersonalToken = vi.fn();
  const revokePersonalToken = vi.fn();

  class PersonalTokenNameConflictError extends Error {
    constructor(name: string) {
      super(`A personal token named "${name}" already exists`);
      this.name = "PersonalTokenNameConflictError";
    }
  }

  return {
    getSession,
    listPersonalTokens,
    issuePersonalToken,
    revokePersonalToken,
    PersonalTokenNameConflictError,
    reset() {
      getSession.mockReset();
      listPersonalTokens.mockReset();
      issuePersonalToken.mockReset();
      revokePersonalToken.mockReset();
    },
  };
});

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  listPersonalTokens: mockState.listPersonalTokens,
  issuePersonalToken: mockState.issuePersonalToken,
  revokePersonalToken: mockState.revokePersonalToken,
  PersonalTokenNameConflictError: mockState.PersonalTokenNameConflictError,
}));

type ModuleExports = typeof import("../../src/app/api/settings/tokens/route");
type DeleteModuleExports = typeof import("../../src/app/api/settings/tokens/[tokenId]/route");

let GET: ModuleExports["GET"];
let POST: ModuleExports["POST"];
let DELETE: DeleteModuleExports["DELETE"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/settings/tokens/route");
  const deleteRouteModule = await import("../../src/app/api/settings/tokens/[tokenId]/route");
  GET = routeModule.GET;
  POST = routeModule.POST;
  DELETE = deleteRouteModule.DELETE;
});

beforeEach(() => {
  mockState.reset();
});

describe("/api/settings/tokens", () => {
  it("lists tokens for the current user and includes expiresAt", async () => {
    mockState.getSession.mockResolvedValue({ id: "user-1" });
    mockState.listPersonalTokens.mockResolvedValue([
      {
        id: "token-1",
        userId: "user-1",
        name: "GitHub Actions CI",
        createdAt: new Date("2026-03-08T18:00:00.000Z"),
        lastUsedAt: null,
        expiresAt: new Date("2026-04-07T18:00:00.000Z"),
      },
    ]);

    const response = await GET();
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(mockState.listPersonalTokens).toHaveBeenCalledWith("user-1");
    expect(body).toEqual({
      tokens: [
        {
          id: "token-1",
          name: "GitHub Actions CI",
          createdAt: new Date("2026-03-08T18:00:00.000Z").toISOString(),
          lastUsedAt: null,
          expiresAt: new Date("2026-04-07T18:00:00.000Z").toISOString(),
        },
      ],
    });
  });

  it("rejects unauthenticated token creation", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await POST(
      new Request("http://localhost:3000/api/settings/tokens", {
        method: "POST",
        body: JSON.stringify({ name: "GitHub Actions CI" }),
      })
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.issuePersonalToken).not.toHaveBeenCalled();
  });

  it("creates a token with the requested expiry", async () => {
    mockState.getSession.mockResolvedValue({ id: "user-1" });
    mockState.issuePersonalToken.mockResolvedValue({
      id: "token-1",
      userId: "user-1",
      name: "GitHub Actions CI",
      token: "tt_created_token",
      createdAt: new Date("2026-03-08T18:00:00.000Z"),
      lastUsedAt: null,
      expiresAt: new Date("2026-04-07T18:00:00.000Z"),
    });

    const response = await POST(
      new Request("http://localhost:3000/api/settings/tokens", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          name: "  GitHub Actions CI  ",
          expiresAt: "2026-04-07T18:00:00.000Z",
        }),
      })
    );
    const body = await response.json();

    expect(response.status).toBe(201);
    expect(response.headers.get("Cache-Control")).toBe("private, no-store");
    expect(mockState.issuePersonalToken).toHaveBeenCalledWith({
      userId: "user-1",
      name: "GitHub Actions CI",
      expiresAt: new Date("2026-04-07T18:00:00.000Z"),
    });
    expect(body).toEqual({
      token: {
        id: "token-1",
        name: "GitHub Actions CI",
        createdAt: new Date("2026-03-08T18:00:00.000Z").toISOString(),
        lastUsedAt: null,
        expiresAt: new Date("2026-04-07T18:00:00.000Z").toISOString(),
      },
      plainTextToken: "tt_created_token",
    });
  });

  it("returns a conflict for duplicate token names", async () => {
    mockState.getSession.mockResolvedValue({ id: "user-1" });
    mockState.issuePersonalToken.mockRejectedValue(
      new mockState.PersonalTokenNameConflictError("GitHub Actions CI")
    );

    const response = await POST(
      new Request("http://localhost:3000/api/settings/tokens", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ name: "GitHub Actions CI" }),
      })
    );

    expect(response.status).toBe(409);
    expect(await response.json()).toEqual({
      error: "A token with this name already exists",
    });
  });

  it("rejects past expirations before touching the token service", async () => {
    mockState.getSession.mockResolvedValue({ id: "user-1" });

    const response = await POST(
      new Request("http://localhost:3000/api/settings/tokens", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          name: "GitHub Actions CI",
          expiresAt: "2026-03-01T00:00:00.000Z",
        }),
      })
    );

    expect(response.status).toBe(422);
    expect(await response.json()).toEqual({
      error: "Expiration must be in the future",
    });
    expect(mockState.issuePersonalToken).not.toHaveBeenCalled();
  });

  it("rejects unauthenticated token revocation", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await DELETE(new Request("http://localhost:3000/api/settings/tokens/token-1"), {
      params: Promise.resolve({ tokenId: "token-1" }),
    });

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.revokePersonalToken).not.toHaveBeenCalled();
  });

  it("revokes a token for the current user", async () => {
    mockState.getSession.mockResolvedValue({ id: "user-1" });
    mockState.revokePersonalToken.mockResolvedValue(true);

    const response = await DELETE(new Request("http://localhost:3000/api/settings/tokens/token-1"), {
      params: Promise.resolve({ tokenId: "token-1" }),
    });

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ success: true });
    expect(mockState.revokePersonalToken).toHaveBeenCalledWith("user-1", "token-1");
  });

  it("returns 404 when the token does not exist", async () => {
    mockState.getSession.mockResolvedValue({ id: "user-1" });
    mockState.revokePersonalToken.mockResolvedValue(false);

    const response = await DELETE(new Request("http://localhost:3000/api/settings/tokens/token-1"), {
      params: Promise.resolve({ tokenId: "token-1" }),
    });

    expect(response.status).toBe(404);
    expect(await response.json()).toEqual({ error: "Token not found" });
  });
});
