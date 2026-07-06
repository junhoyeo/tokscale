import { readFile } from "node:fs/promises";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const sql = vi.fn((strings: TemplateStringsArray, ...values: unknown[]) => ({
    strings: Array.from(strings),
    values,
  }));

  return {
    sql,
    reset() {
      sql.mockClear();
    },
  };
});

vi.mock("drizzle-orm", async (importOriginal) => ({
  ...(await importOriginal<typeof import("drizzle-orm")>()),
  sql: mockState.sql,
}));

import {
  USERNAME_LOOKUP_LIMIT,
  normalizeUsernameCacheKey,
  getSingleUsernameMatch,
  usernameEqualsIgnoreCase,
} from "../../src/lib/db/usernameLookup";
import {
  USERS_USERNAME_LOWER_UNIQUE_INDEX,
  usernameLowerExpression,
} from "../../src/lib/db/usernameIndex";

beforeEach(() => {
  mockState.reset();
});

describe("username lookup helpers", () => {
  it("builds an exact case-insensitive username condition", () => {
    usernameEqualsIgnoreCase("ImLunaHey");

    const [expressionStrings, column] = mockState.sql.mock.calls[0] as [
      TemplateStringsArray,
      unknown,
    ];
    const [conditionStrings, indexedExpression, username] = mockState.sql.mock.calls[1] as [
      TemplateStringsArray,
      unknown,
      string,
    ];

    expect(Array.from(expressionStrings)).toEqual(["lower(", ")"]);
    expect(column).toBeDefined();
    expect(Array.from(conditionStrings)).toEqual(["", " = ", ""]);
    expect(indexedExpression).toBe(mockState.sql.mock.results[0].value);
    expect(username).toBe("imlunahey");
  });

  it("uses the same lower expression helper that backs the unique index", () => {
    usernameLowerExpression({} as never);

    const [strings] = mockState.sql.mock.calls[0] as [TemplateStringsArray, unknown];

    expect(USERS_USERNAME_LOWER_UNIQUE_INDEX).toBe("users_username_lower_unique");
    expect(Array.from(strings)).toEqual(["lower(", ")"]);
  });

  it("normalizes username cache keys with ASCII case folding", () => {
    expect(normalizeUsernameCacheKey("ImLunaHey")).toBe("imlunahey");
  });

  it("uses a two-row lookup limit to detect case-colliding usernames", () => {
    expect(USERNAME_LOOKUP_LIMIT).toBe(2);
  });

  it("returns a single username match without changing the canonical row", () => {
    const row = { username: "ImLunaHey" };

    expect(getSingleUsernameMatch([row], "imlunahey")).toBe(row);
  });

  it("rejects ambiguous case-insensitive username matches", () => {
    expect(() =>
      getSingleUsernameMatch(
        [{ username: "ImLunaHey" }, { username: "imlunahey" }],
        "imlunahey",
      )
    ).toThrow("Multiple users match username imlunahey case-insensitively");
  });

  it("ships a unique functional index migration for case-insensitive usernames", async () => {
    const migration = await readFile(
      new URL("../../src/lib/db/migrations/0005_add_case_insensitive_username_index.sql", import.meta.url),
      "utf8",
    );

    expect(migration).toContain("CREATE UNIQUE INDEX IF NOT EXISTS");
    expect(migration).toContain('"users_username_lower_unique"');
    expect(migration).toContain('ON "users" (lower("username"))');
  });
});
