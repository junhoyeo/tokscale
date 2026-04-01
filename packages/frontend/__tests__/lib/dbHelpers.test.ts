import { describe, expect, it } from "vitest";
import { resolveSubmissionScope } from "../../src/lib/db/helpers";

describe("resolveSubmissionScope", () => {
  it("reuses the exact source-scoped row when one already exists", () => {
    const result = resolveSubmissionScope(
      [
        { id: "row-1", sourceId: "machine-a" },
        { id: "row-2", sourceId: "machine-b" },
      ],
      "machine-b"
    );

    expect(result).toEqual({
      kind: "existing",
      submissionId: "row-2",
      upgradeLegacyRow: false,
    });
  });

  it("upgrades a lone legacy unsourced row on the first source-aware submit", () => {
    const result = resolveSubmissionScope(
      [{ id: "legacy-row", sourceId: null }],
      "machine-a"
    );

    expect(result).toEqual({
      kind: "existing",
      submissionId: "legacy-row",
      upgradeLegacyRow: true,
    });
  });

  it("creates a new row for a new source after scoped mode already exists", () => {
    const result = resolveSubmissionScope(
      [{ id: "row-1", sourceId: "machine-a" }],
      "machine-b"
    );

    expect(result).toEqual({ kind: "create" });
  });

  it("rejects ambiguous unsourced submits after scoped mode begins", () => {
    const result = resolveSubmissionScope(
      [{ id: "row-1", sourceId: "machine-a" }],
      null
    );

    expect(result).toEqual({ kind: "rejectMissingSourceIdentity" });
  });

  it("keeps using the legacy unsourced row until scoped mode starts", () => {
    const result = resolveSubmissionScope(
      [{ id: "legacy-row", sourceId: null }],
      null
    );

    expect(result).toEqual({
      kind: "existing",
      submissionId: "legacy-row",
      upgradeLegacyRow: false,
    });
  });
});
