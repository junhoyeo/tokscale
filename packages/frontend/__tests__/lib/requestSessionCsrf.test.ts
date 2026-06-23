import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * B6: CSRF origin allowlist for cookie-based sessions.
 * - Mutating requests (POST/PATCH/PUT/DELETE) with a mismatched Origin are rejected.
 * - Requests with a valid Bearer token bypass the origin check.
 * - Requests without an Origin header are rejected on mutating cookie sessions (non-browser clients should use Bearer).
 * - The NEXT_PUBLIC_URL origin (self-hosted deployments) is always allowed.
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

afterEach(() => vi.unstubAllEnvs());

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

  it("rejects the removed tokscale.dev origin (domain does not exist)", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://tokscale.dev" })
    );

    expect(result).toBeNull();
    expect(mockState.getSession).not.toHaveBeenCalled();
  });

  it("allows cookie session from the production custom domain when bearer auth is disabled", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      new Request("https://tokscale.ai/api/groups", {
        method: "POST",
        headers: {
          Cookie: "tt_session=session-token",
          Origin: "https://tokscale.ai",
        },
      }),
      { allowAuthorizationHeader: false }
    );

    expect(result).toEqual(validUser);
    expect(mockState.getSession).toHaveBeenCalledTimes(1);
    expect(mockState.getSessionFromHeader).not.toHaveBeenCalled();
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

  it("does not resolve Authorization header sessions when bearer auth is disabled", async () => {
    mockState.getSessionFromHeader.mockResolvedValue(validUser);
    mockState.getSession.mockResolvedValue(null);

    const result = await getSessionFromRequest(
      makeRequest("POST", {
        Origin: "http://localhost:3000",
        Authorization: "Bearer tt_sometoken",
      }),
      { allowAuthorizationHeader: false }
    );

    expect(result).toBeNull();
    expect(mockState.getSessionFromHeader).not.toHaveBeenCalled();
    expect(mockState.getSession).toHaveBeenCalledTimes(1);
  });

  it("allows cookie sessions when bearer auth is disabled and Origin is allowed", async () => {
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "http://localhost:3000" }),
      { allowAuthorizationHeader: false }
    );

    expect(result).toEqual(validUser);
    expect(mockState.getSessionFromHeader).not.toHaveBeenCalled();
    expect(mockState.getSession).toHaveBeenCalledTimes(1);
  });

  it("allows cookie session from the NEXT_PUBLIC_URL origin (self-hosted deployment)", async () => {
    vi.stubEnv("NEXT_PUBLIC_URL", "https://tokscale.my-company.example");
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://tokscale.my-company.example" }),
      { allowAuthorizationHeader: false }
    );

    expect(result).toEqual(validUser);
    expect(mockState.getSession).toHaveBeenCalledTimes(1);
  });

  it("derives the allowed origin from NEXT_PUBLIC_URL with a path or trailing slash", async () => {
    vi.stubEnv("NEXT_PUBLIC_URL", "https://tokscale.my-company.example/app/");
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://tokscale.my-company.example" }),
      { allowAuthorizationHeader: false }
    );

    expect(result).toEqual(validUser);
  });

  it("still allows the NEXT_PUBLIC_URL origin when CSRF_ALLOWED_ORIGINS is set to other origins", async () => {
    vi.stubEnv("NEXT_PUBLIC_URL", "https://tokscale.my-company.example");
    vi.stubEnv("CSRF_ALLOWED_ORIGINS", "https://other.example.com");
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://tokscale.my-company.example" }),
      { allowAuthorizationHeader: false }
    );

    expect(result).toEqual(validUser);
  });

  it("keeps rejecting unknown origins when NEXT_PUBLIC_URL is set", async () => {
    vi.stubEnv("NEXT_PUBLIC_URL", "https://tokscale.my-company.example");
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://evil.example.com" })
    );

    expect(result).toBeNull();
    expect(mockState.getSession).not.toHaveBeenCalled();
  });

  it("does not allowlist the opaque 'null' origin from a non-HTTP NEXT_PUBLIC_URL", async () => {
    vi.stubEnv("NEXT_PUBLIC_URL", "mailto:admin@example.com");
    mockState.getSession.mockResolvedValue(validUser);

    // new URL("mailto:...").origin === "null"; sandboxed iframes send
    // Origin: null, so accepting it would reopen CSRF.
    const result = await getSessionFromRequest(
      makeRequest("POST", { Origin: "null" })
    );

    expect(result).toBeNull();
    expect(mockState.getSession).not.toHaveBeenCalled();
  });

  it("ignores a malformed NEXT_PUBLIC_URL and keeps the explicit allowlist working", async () => {
    vi.stubEnv("NEXT_PUBLIC_URL", "not-a-valid-url");
    mockState.getSession.mockResolvedValue(validUser);

    const allowed = await getSessionFromRequest(
      makeRequest("POST", { Origin: "http://localhost:3000" })
    );
    expect(allowed).toEqual(validUser);

    const rejected = await getSessionFromRequest(
      makeRequest("POST", { Origin: "https://evil.example.com" })
    );
    expect(rejected).toBeNull();
  });

  it("ignores Authorization headers when bearer auth is disabled and still uses valid cookies", async () => {
    mockState.getSessionFromHeader.mockResolvedValue({
      id: "token-user",
      username: "token-user",
      displayName: null,
      avatarUrl: null,
    });
    mockState.getSession.mockResolvedValue(validUser);

    const result = await getSessionFromRequest(
      makeRequest("POST", {
        Origin: "http://localhost:3000",
        Authorization: "Bearer tt_sometoken",
      }),
      { allowAuthorizationHeader: false }
    );

    expect(result).toEqual(validUser);
    expect(mockState.getSessionFromHeader).not.toHaveBeenCalled();
    expect(mockState.getSession).toHaveBeenCalledTimes(1);
  });
});
