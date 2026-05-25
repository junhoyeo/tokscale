import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const revokePersonalToken = vi.fn();

  return {
    getSession,
    revokePersonalToken,
    reset() {
      getSession.mockReset();
      revokePersonalToken.mockReset();
    },
  };
});

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
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

describe("DELETE /api/settings/tokens/[tokenId]", () => {
  it("returns 401 with 'Not authenticated' when session is null", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await DELETE(
      new Request("http://localhost:3000/api/settings/tokens/token-1", {
        method: "DELETE",
      }),
      { params: Promise.resolve({ tokenId: "token-1" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
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
      new Request("http://localhost:3000/api/settings/tokens/token-1", {
        method: "DELETE",
      }),
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
      new Request("http://localhost:3000/api/settings/tokens/token-999", {
        method: "DELETE",
      }),
      { params: Promise.resolve({ tokenId: "token-999" }) }
    );

    expect(response.status).toBe(404);
    expect(await response.json()).toEqual({ error: "Token not found" });
    expect(mockState.revokePersonalToken).toHaveBeenCalledWith("user-1", "token-999");
  });
});
