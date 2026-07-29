import { afterEach, describe, expect, it, vi } from "vitest";

import { isAdmin, resolveAdminGithubIds } from "@/lib/auth/admin";
import type { SessionUser } from "@/lib/auth/session";

const JUNHOYEO_GITHUB_ID = 32605822;

function session(overrides: Partial<SessionUser> = {}): SessionUser {
  return {
    id: "user-1",
    username: "somebody",
    displayName: null,
    avatarUrl: null,
    githubId: 999,
    ...overrides,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("resolveAdminGithubIds", () => {
  it("defaults to the maintainer when the env var is unset", () => {
    expect(resolveAdminGithubIds(undefined)).toEqual([JUNHOYEO_GITHUB_ID]);
  });

  it("parses a comma-separated list, tolerating whitespace", () => {
    expect(resolveAdminGithubIds("1, 22 ,333")).toEqual([1, 22, 333]);
  });

  it("grants nobody access when the env var is set but empty", () => {
    // Explicitly emptying the list must mean "no admins", not "fall back to
    // the built-in default" — otherwise revoking access silently fails.
    expect(resolveAdminGithubIds("")).toEqual([]);
    expect(resolveAdminGithubIds("  ,  ")).toEqual([]);
  });

  it.each([
    ["non-numeric", "12,abc"],
    ["hex", "0x1f"],
    ["exponent", "1e3"],
    ["negative", "-5"],
    ["decimal", "12.5"],
  ])("rejects the whole list when an entry is %s", (_label, raw) => {
    // Fails closed on the entire list rather than honouring the entries that
    // happened to parse: a typo means we cannot tell who was meant to have
    // access, so nobody does.
    vi.spyOn(console, "error").mockImplementation(() => {});

    expect(resolveAdminGithubIds(raw)).toEqual([]);
  });

  it("rejects ids beyond the safe integer range", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});

    expect(resolveAdminGithubIds("9007199254740993")).toEqual([]);
  });

  it("does not fall back to the default on malformed input", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});

    expect(resolveAdminGithubIds("nonsense")).not.toContain(JUNHOYEO_GITHUB_ID);
  });
});

describe("isAdmin", () => {
  it("accepts the maintainer's GitHub id", () => {
    expect(isAdmin(session({ githubId: JUNHOYEO_GITHUB_ID }))).toBe(true);
  });

  it("rejects a different GitHub id", () => {
    expect(isAdmin(session({ githubId: 12345 }))).toBe(false);
  });

  it("rejects an absent session", () => {
    expect(isAdmin(null)).toBe(false);
    expect(isAdmin(undefined)).toBe(false);
  });

  it("rejects a personal-token session, which carries no GitHub id", () => {
    // validateApiToken sets githubId to null, so CLI tokens can never moderate
    // even if an admin route forgets allowAuthorizationHeader: false.
    expect(isAdmin(session({ githubId: null }))).toBe(false);
  });

  it("does not authorize on username, only on GitHub id", () => {
    // Usernames can be renamed and re-registered by someone else; the numeric
    // id cannot. Matching the name must never be sufficient.
    expect(isAdmin(session({ username: "junhoyeo", githubId: 404 }))).toBe(false);
  });
});
