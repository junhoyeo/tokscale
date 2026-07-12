import { describe, expect, it } from "vitest";

import { resolveSortByParam } from "@/lib/leaderboard/constants";

describe("resolveSortByParam", () => {
  it("keeps absent values available for persisted preference fallback", () => {
    expect(resolveSortByParam(null)).toBeNull();
    expect(resolveSortByParam(undefined)).toBeNull();
  });

  it("preserves supported explicit sort values", () => {
    expect(resolveSortByParam("tokens")).toBe("tokens");
    expect(resolveSortByParam("cost")).toBe("cost");
  });

  it("maps retired and unknown explicit values to tokens", () => {
    expect(resolveSortByParam("time")).toBe("tokens");
    expect(resolveSortByParam("unknown")).toBe("tokens");
    expect(resolveSortByParam("")).toBe("tokens");
  });
});
