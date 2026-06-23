import { describe, expect, it } from "vitest";
import {
  GITHUB_USERNAME_REGEX,
  isValidGitHubUsername,
} from "../../src/lib/validation/username";

describe("github username validation", () => {
  it("accepts valid usernames", () => {
    const validUsernames = [
      "octocat",
      "tokscale",
      "junhoyeo",
      "A",
      "a-b-c",
      "user123",
      "A".repeat(39),
    ];

    for (const username of validUsernames) {
      expect(isValidGitHubUsername(username)).toBe(true);
      expect(GITHUB_USERNAME_REGEX.test(username)).toBe(true);
    }
  });

  it("rejects invalid usernames", () => {
    const invalidUsernames = [
      "",
      "a/b",
      "a b",
      "a_b",
      "a.b",
      "a@b",
      "a!",
      "a?",
      "A".repeat(40),
    ];

    for (const username of invalidUsernames) {
      expect(isValidGitHubUsername(username)).toBe(false);
      expect(GITHUB_USERNAME_REGEX.test(username)).toBe(false);
    }
  });
});
