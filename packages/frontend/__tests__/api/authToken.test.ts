import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();

  return {
    authenticatePersonalToken,
    reset() {
      authenticatePersonalToken.mockReset();
    },
  };
});

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: mockState.authenticatePersonalToken,
}));

type ModuleExports = typeof import("../../src/app/api/auth/token/route");

let GET: ModuleExports["GET"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/auth/token/route");
  GET = routeModule.GET;
});

beforeEach(() => {
  mockState.reset();
});

describe("GET /api/auth/token", () => {
  it("returns 401 when the bearer token is missing", async () => {
    const response = await GET(
      new Request("http://localhost:3000/api/auth/token")
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({
      error: "Missing or invalid Authorization header",
    });
    expect(mockState.authenticatePersonalToken).not.toHaveBeenCalled();
  });

  it("returns 401 when the bearer token is invalid", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({ status: "invalid" });

    const response = await GET(
      new Request("http://localhost:3000/api/auth/token", {
        headers: { Authorization: "Bearer tt_invalid" },
      })
    );

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_invalid", {
      touchLastUsedAt: false,
    });
    expect(await response.json()).toEqual({ error: "Invalid API token" });
  });

  it("accepts the bearer scheme case-insensitively", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: null,
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });

    const response = await GET(
      new Request("http://localhost:3000/api/auth/token", {
        headers: { Authorization: "bEaReR tt_valid" },
      })
    );

    expect(response.status).toBe(200);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_valid", {
      touchLastUsedAt: false,
    });
  });

  it("returns user metadata for a valid bearer token", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: "https://example.com/alice.png",
      isAdmin: false,
      expiresAt: null,
    });

    const response = await GET(
      new Request("http://localhost:3000/api/auth/token", {
        headers: { Authorization: "Bearer tt_valid" },
      })
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      user: {
        username: "alice",
        displayName: "Alice",
        avatarUrl: "https://example.com/alice.png",
      },
    });
  });
});
