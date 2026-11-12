import { describe, expect, it, vi } from "vitest";

vi.mock("@/lib/db", () => ({ db: {} }));
vi.mock("next/cache", () => ({ unstable_cache: (load: () => unknown) => load, revalidateTag: vi.fn() }));

import { mentionsModelPath } from "@/lib/explore/db";

describe("mentionsModelPath", () => {
  it("matches a raw model as a per-model key or as a legacy modelId", () => {
    expect(mentionsModelPath(["claude-opus-5"])).toBe(
      '$.* ? (exists(@.models."claude-opus-5") || @.modelId == "claude-opus-5")',
    );
  });

  it("tests every raw id of the model", () => {
    expect(mentionsModelPath(["a", "b"])).toBe(
      '$.* ? (exists(@.models."a") || @.modelId == "a" || exists(@.models."b") || @.modelId == "b")',
    );
  });

  it("quotes submitted names so they cannot change the path", () => {
    expect(mentionsModelPath(['x") || (1 == 1', "back\\slash"])).toBe(
      '$.* ? (exists(@.models."x\\") || (1 == 1") || @.modelId == "x\\") || (1 == 1" || exists(@.models."back\\\\slash") || @.modelId == "back\\\\slash")',
    );
  });
});
