import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const cookieStore = {
    get: vi.fn(),
    set: vi.fn(),
    delete: vi.fn(),
  };
  const cookies = vi.fn(async () => cookieStore);
  const getAuthorizationUrl = vi.fn(() => "https://github.com/login/oauth/authorize");
  const exchangeCodeForToken = vi.fn(async () => "access-token");
  const getGitHubUser = vi.fn(async () => ({
    id: 123,
    login: "alice",
    name: "Alice",
    avatar_url: "https://avatars.example/alice.png",
    email: "alice@example.com",
  }));
  const getGitHubUserEmail = vi.fn(async () => "alice@example.com");
  const createSession = vi.fn(async () => "session-token");
  const setSessionCookie = vi.fn(async () => undefined);
  const generateRandomString = vi.fn(() => "state-token");

  const selectLimit = vi.fn(async () => [
    {
      id: "user-1",
      githubId: 123,
      username: "alice",
      displayName: "Alice",
      avatarUrl: "https://avatars.example/alice.png",
      email: "alice@example.com",
    },
  ]);
  const selectWhere = vi.fn(() => ({ limit: selectLimit }));
  const selectFrom = vi.fn(() => ({ where: selectWhere }));
  const updateWhere = vi.fn(async () => undefined);
  const updateSet = vi.fn(() => ({ where: updateWhere }));
  const insertReturning = vi.fn(async () => [{ id: "user-1" }]);
  const insertValues = vi.fn(() => ({ returning: insertReturning }));
  const db = {
    select: vi.fn(() => ({ from: selectFrom })),
    update: vi.fn(() => ({ set: updateSet })),
    insert: vi.fn(() => ({ values: insertValues })),
  };

  return {
    cookieStore,
    cookies,
    getAuthorizationUrl,
    exchangeCodeForToken,
    getGitHubUser,
    getGitHubUserEmail,
    createSession,
    setSessionCookie,
    generateRandomString,
    db,
    selectLimit,
    reset() {
      cookieStore.get.mockReset();
      cookieStore.set.mockReset();
      cookieStore.delete.mockReset();
      cookies.mockClear();
      getAuthorizationUrl.mockClear();
      exchangeCodeForToken.mockClear();
      getGitHubUser.mockClear();
      getGitHubUserEmail.mockClear();
      createSession.mockClear();
      setSessionCookie.mockClear();
      generateRandomString.mockClear();
      db.select.mockClear();
      db.update.mockClear();
      db.insert.mockClear();
      selectLimit.mockClear();
    },
  };
});

vi.mock("next/headers", () => ({
  cookies: mockState.cookies,
}));

vi.mock("@/lib/auth/github", () => ({
  getAuthorizationUrl: mockState.getAuthorizationUrl,
  exchangeCodeForToken: mockState.exchangeCodeForToken,
  getGitHubUser: mockState.getGitHubUser,
  getGitHubUserEmail: mockState.getGitHubUserEmail,
}));

vi.mock("@/lib/auth/session", () => ({
  createSession: mockState.createSession,
  setSessionCookie: mockState.setSessionCookie,
}));

vi.mock("@/lib/auth/utils", () => ({
  generateRandomString: mockState.generateRandomString,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  users: {
    id: "users.id",
    githubId: "users.githubId",
  },
}));

vi.mock("drizzle-orm", () => ({
  eq: vi.fn(() => "eq"),
}));

type StartRouteExports = typeof import("../../src/app/api/auth/github/route");
type CallbackRouteExports = typeof import("../../src/app/api/auth/github/callback/route");

let startGET: StartRouteExports["GET"];
let callbackGET: CallbackRouteExports["GET"];

beforeAll(async () => {
  const [startRoute, callbackRoute] = await Promise.all([
    import("../../src/app/api/auth/github/route"),
    import("../../src/app/api/auth/github/callback/route"),
  ]);
  startGET = startRoute.GET;
  callbackGET = callbackRoute.GET;
});

beforeEach(() => {
  mockState.reset();
  process.env.NEXT_PUBLIC_URL = "https://tokscale.ai";
});

describe("GitHub OAuth returnTo safety", () => {
  it.each([
    ["/settings", "/settings"],
    ["/device?code=abc", "/device?code=abc"],
    ["https://evil.test/path", "/leaderboard"],
    ["//evil.test/path", "/leaderboard"],
    ["@evil.test/path", "/leaderboard"],
    ["\\evil.test\\path", "/leaderboard"],
    ["%2f%2fevil.test/path", "/leaderboard"],
    ["%5cevil.test%5cpath", "/leaderboard"],
  ])("stores safe returnTo value for %s", async (returnTo, expected) => {
    await startGET(
      new Request(
        `https://tokscale.ai/api/auth/github?returnTo=${encodeURIComponent(returnTo)}`
      )
    );

    expect(mockState.cookieStore.set).toHaveBeenCalledTimes(1);
    const [, rawValue] = mockState.cookieStore.set.mock.calls[0];
    expect(JSON.parse(rawValue)).toMatchObject({
      state: "state-token",
      returnTo: expected,
    });
  });

  it.each([
    ["@evil.test/path"],
    ["https://evil.test/path"],
    ["//evil.test/path"],
    ["\\evil.test\\path"],
    ["%2f%2fevil.test/path"],
    ["/%5cevil.test"],
  ])("falls back when callback cookie returnTo is unsafe: %s", async (returnTo) => {
    mockState.cookieStore.get.mockReturnValue({
      value: JSON.stringify({ state: "state-token", returnTo }),
    });

    const response = await callbackGET(
      new Request(
        "https://tokscale.ai/api/auth/github/callback?code=ok&state=state-token"
      )
    );

    expect(response.headers.get("location")).toBe("https://tokscale.ai/leaderboard");
  });

  it("redirects to a safe same-origin relative callback returnTo", async () => {
    mockState.cookieStore.get.mockReturnValue({
      value: JSON.stringify({ state: "state-token", returnTo: "/device?code=abc" }),
    });

    const response = await callbackGET(
      new Request(
        "https://tokscale.ai/api/auth/github/callback?code=ok&state=state-token"
      )
    );

    expect(response.headers.get("location")).toBe(
      "https://tokscale.ai/device?code=abc"
    );
  });
});
