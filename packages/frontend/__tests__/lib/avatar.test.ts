import { describe, expect, it } from "vitest";
import { avatarSrc, sizedAvatarUrl } from "@/lib/avatar";

describe("sizedAvatarUrl", () => {
  it("asks GitHub for twice the drawn size", () => {
    expect(sizedAvatarUrl("https://avatars.githubusercontent.com/u/1234?v=4", 32)).toBe(
      "https://avatars.githubusercontent.com/u/1234?v=4&s=64",
    );
  });

  it("replaces a size that is already in the URL", () => {
    expect(sizedAvatarUrl("https://avatars.githubusercontent.com/u/1234?s=460&v=4", 20)).toBe(
      "https://avatars.githubusercontent.com/u/1234?s=40&v=4",
    );
  });

  it("leaves an avatar from another host, or a malformed one, as it is", () => {
    expect(sizedAvatarUrl("https://example.com/me.png", 32)).toBe("https://example.com/me.png");
    expect(sizedAvatarUrl("not a url", 32)).toBe("not a url");
  });
});

describe("avatarSrc", () => {
  it("sizes the stored avatar", () => {
    expect(avatarSrc("https://avatars.githubusercontent.com/u/1234?v=4", "octocat", 36)).toBe(
      "https://avatars.githubusercontent.com/u/1234?v=4&s=72",
    );
  });

  it("falls back to the account's GitHub picture at the same size", () => {
    expect(avatarSrc(null, "octo cat", 36)).toBe("https://github.com/octo%20cat.png?size=72");
  });
});
