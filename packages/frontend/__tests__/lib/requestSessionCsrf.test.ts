import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * B6: CSRF origin allowlist for cookie-based sessions.
 * - Mutating requests (POST/PATCH/PUT/DELETE) with a mismatched Origin are rejected.
 * - Requests with a valid Bearer token bypass the origin check.
 * - Requests without an Origin header are rejected on mutating cookie sessions (non-browser clients should use Bearer).
 */

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const getSessionFromHeader = vi.fn();

  return {
    getSession,
    getSessionFromHeader,
    reset() {
      getSession.mockReset();
      getSessionFromHeader.mockReset();
    },
  };
});

vi.mock("../../src/lib/auth/session", () => ({
  getSession: mockState.getSession,
  getSessionFromHeader: mockState.getSessionFromHeader,
}));

type ModuleExports = typeof import("../../src/lib/auth/requestSession");

let getSessionFromRequest: ModuleExports["getSessionFromRequest"];

beforeAll(async () => {
  const mod = await import("../../src/lib/auth/requestSession");
  getSessionFromRequest = mod.getSessionFromRequest;
});

beforeEach(() => mockState.reset());

const validUser = { id: "user-1", username: "alice", displayName: null, avatarUrl: null };

function makeRequest(
  method: string,
  headers: Record<string, string> = {}
): Request {
  return new Request("http://localhost:3000/api/groups", {
    method,
    headers,
  });
}

describe("getSessionFromRequest — CSRF origin check (B6)", () => {
  it("rejects cookie session when Origin is an unknown domain on POST", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://evil.example.com" })
    );

    expect(result).toBeNull();
    expect(mockState.getSession).not.toHaveBeenCalled();
  });

  it("rejects cookie session when Origin is unknown on PATCH", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("PATCH", { Origin: "https://attacker.io" })
    );

    expect(result).toBeNull();
  });

  it("rejects cookie session when Origin is unknown on PUT", async () => {
    const result = await getSessionFromRequest(
      makeRequest("PUT", { Origin: "https://attacker.io" })
    );

    expect(result).toBeNull();
    expect(mockState.getSession).not.toHaveBeenCalled();
  });

  it("rejects cookie session when Origin is unknown on DELETE", async () => {
    const result = await getSessionFromRequest(
      makeRequest("DELETE", { Origin: "https://attacker.io" })
    );

    expect(result).toBeNull();
  });

  it("allows cookie session when Origin is in the default allowlist (localhost)", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "http://localhost:3000" })
    );

    expect(result).toEqual(validUser);
    expect(mockState.getSession).toHaveBeenCalledTimes(1);
  });

  it("allows cookie session when Origin is in the default allowlist (production)", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://tokscale.dev" })
    );

    expect(result).toEqual(validUser);
  });

  it("rejects cookie session on mutating method with no Origin header (non-browser clients must use Bearer)", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(makeRequest("POST"));

    expect(result).toBeNull();
    // Stricter than before: a missing Origin is now treated as untrusted
    // for cookie-authenticated mutations. Non-browser clients should
    // present a Bearer token (covered by the next test).
    expect(mockState.getSession).not.toHaveBeenCalled();
  });

  it("allows GET requests regardless of Origin", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("GET", { Origin: "https://evil.example.com" })
    );

    expect(result).toEqual(validUser);
  });

  it("bypasses origin check when Authorization (Bearer) header is present", async () => {
    mockState.getSessionFromHeader.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", {
        Origin: "https://evil.example.com",
        Authorization: "Bearer tt_sometoken",
      })
    );

    // Should delegate to getSessionFromHeader (Bearer path), not return null
    expect(result).toEqual(validUser);
    expect(mockState.getSessionFromHeader).toHaveBeenCalledTimes(1);
    expect(mockState.getSession).not.toHaveBeenCalled();
  });
});
