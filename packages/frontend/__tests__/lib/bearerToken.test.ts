import { describe, expect, it } from "vitest";
import { getBearerToken } from "../../src/lib/auth/bearerToken";

describe("getBearerToken", () => {
  it("returns null when the header is missing or malformed", () => {
    expect(getBearerToken(null)).toBeNull();
    expect(getBearerToken("")).toBeNull();
    expect(getBearerToken("Basic abc")).toBeNull();
    expect(getBearerToken("Bearer")).toBeNull();
  });

  it("accepts the bearer auth scheme case-insensitively", () => {
    expect(getBearerToken("Bearer tt_token")).toBe("tt_token");
    expect(getBearerToken("bearer tt_token")).toBe("tt_token");
    expect(getBearerToken("bEaReR tt_token")).toBe("tt_token");
  });

  it("trims surrounding token whitespace", () => {
    expect(getBearerToken("Bearer   tt_token  ")).toBe("tt_token");
  });
});
