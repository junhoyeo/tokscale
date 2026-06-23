import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const getSessionFromHeader = vi.fn();
  const revokePersonalToken = vi.fn();

  return {
    getSession,
    getSessionFromHeader,
    revokePersonalToken,
    reset() {
      getSession.mockReset();
      getSessionFromHeader.mockReset();
      revokePersonalToken.mockReset();
    },
  };
});

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
  getSessionFromHeader: mockState.getSessionFromHeader,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  revokePersonalToken: mockState.revokePersonalToken,
}));

type ModuleExports = typeof import("../../src/app/api/settings/tokens/[tokenId]/route");

let DELETE: ModuleExports["DELETE"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/settings/tokens/[tokenId]/route");
  DELETE = routeModule.DELETE;
});

beforeEach(() => {
  mockState.reset();
});

function createDeleteRequest(
  tokenId: string,
  headers: Record<string, string> = { Origin: "http://localhost:3000" }
) {
  return new Request(`http://localhost:3000/api/settings/tokens/${tokenId}`, {
    method: "DELETE",
    headers,
  });
}

describe("DELETE /api/settings/tokens/[tokenId]", () => {
  it("returns 401 with 'Not authenticated' when session is null", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await DELETE(
      createDeleteRequest("token-1"),
      { params: Promise.resolve({ tokenId: "token-1" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
  });

  it("returns 401 for cookie auth when Origin is missing", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });

    const response = await DELETE(
      createDeleteRequest("token-1", {}),
      { params: Promise.resolve({ tokenId: "token-1" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.revokePersonalToken).not.toHaveBeenCalled();
  });

  it("returns 401 for cookie auth when Origin is not allowed", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });

    const response = await DELETE(
      createDeleteRequest("token-1", { Origin: "https://attacker.example" }),
      { params: Promise.resolve({ tokenId: "token-1" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.revokePersonalToken).not.toHaveBeenCalled();
  });

  it("rejects Authorization-header session auth for token delete mutations", async () => {
    mockState.getSession.mockResolvedValue(null);
    mockState.getSessionFromHeader.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });

    const response = await DELETE(
      createDeleteRequest("token-1", {
        Origin: "http://localhost:3000",
        Authorization: "Bearer header-session",
      }),
      { params: Promise.resolve({ tokenId: "token-1" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSessionFromHeader).not.toHaveBeenCalled();
    expect(mockState.revokePersonalToken).not.toHaveBeenCalled();
  });

  it("returns 200 with 'success: true' when token is successfully revoked", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    mockState.revokePersonalToken.mockResolvedValue(true);

    const response = await DELETE(
      createDeleteRequest("token-1"),
      { params: Promise.resolve({ tokenId: "token-1" }) }
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ success: true });
    expect(mockState.revokePersonalToken).toHaveBeenCalledWith("user-1", "token-1");
  });

  it("returns 404 with 'Token not found' when token does not exist or belongs to another user", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    mockState.revokePersonalToken.mockResolvedValue(false);

    const response = await DELETE(
      createDeleteRequest("token-999"),
      { params: Promise.resolve({ tokenId: "token-999" }) }
    );

    expect(response.status).toBe(404);
    expect(await response.json()).toEqual({ error: "Token not found" });
    expect(mockState.revokePersonalToken).toHaveBeenCalledWith("user-1", "token-999");
  });
});
