import { describe, expect, it } from "vitest";

import {
  canManageGroupRole,
  createGroupInviteToken,
  hashGroupInviteToken,
  slugifyGroupName,
} from "../../src/lib/groups";

describe("group helpers", () => {
  it("creates URL-safe group slugs with reserved fallback handling", () => {
    expect(slugifyGroupName("  AI Usage Team!!  ")).toBe("ai-usage-team");
    expect(slugifyGroupName("foo_bar")).toBe("foo-bar");
    expect(slugifyGroupName("foo_bar")).not.toBe(slugifyGroupName("foobar"));
    expect(slugifyGroupName("")).toMatch(/^group-[a-z0-9]+$/);
    expect(slugifyGroupName("join")).toMatch(/^join-[a-z0-9]+$/);
  });

  it("keeps generated slugs within the database column limit after suffixes", () => {
    const longName = "A".repeat(140);
    const slug = slugifyGroupName(longName, { suffix: 12345 });

    expect(slug).toHaveLength(100);
    expect(slug).toMatch(/-12345$/);
    expect(slug.endsWith("-")).toBe(false);
  });

  it("stores invite tokens as hashes and returns raw tokens only for links", () => {
    const token = createGroupInviteToken();
    const hashed = hashGroupInviteToken(token);

    expect(token).toMatch(/^tg_[a-f0-9]{48}$/);
    expect(hashed).toMatch(/^[a-f0-9]{64}$/);
    expect(hashed).not.toBe(token);
  });

  it("allows only stronger roles to manage weaker member roles", () => {
    expect(canManageGroupRole("owner", "admin")).toBe(true);
    expect(canManageGroupRole("admin", "member")).toBe(true);
    expect(canManageGroupRole("admin", "owner")).toBe(false);
    expect(canManageGroupRole("member", "member")).toBe(false);
  });
});
