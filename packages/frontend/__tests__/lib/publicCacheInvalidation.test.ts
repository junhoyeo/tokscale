import { beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => ({
  revalidateTag: vi.fn(),
  revalidatePath: vi.fn(),
  revalidateLeaderboardPublicSurfacePaths: vi.fn(),
  revalidateUserGroupLeaderboards: vi.fn(),
}));

vi.mock("next/cache", () => ({
  revalidateTag: mockState.revalidateTag,
  revalidatePath: mockState.revalidatePath,
}));

vi.mock("../../src/lib/groups/cache", () => ({
  revalidateUserGroupLeaderboards: mockState.revalidateUserGroupLeaderboards,
}));

vi.mock("../../src/lib/leaderboard/publicSurfaceRevalidation", () => ({
  revalidateLeaderboardPublicSurfacePaths:
    mockState.revalidateLeaderboardPublicSurfacePaths,
}));

import { revalidateSubmissionPublicCaches } from "../../src/lib/leaderboard/publicCacheInvalidation";

describe("revalidateSubmissionPublicCaches", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockState.revalidateUserGroupLeaderboards.mockResolvedValue(undefined);
  });

  it("normalizes and deduplicates every rank-affected username", async () => {
    await revalidateSubmissionPublicCaches("user-1", "Alice", [
      "ALICE",
      "Bob",
      "bOB",
    ]);

    expect(mockState.revalidateTag).toHaveBeenCalledWith("user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenCalledWith("user:bob", "max");
    expect(mockState.revalidateTag).toHaveBeenCalledWith(
      "user-rank:bob",
      "max"
    );
    expect(mockState.revalidateTag).toHaveBeenCalledWith(
      "embed-user:bob:tokens",
      "max"
    );
    expect(mockState.revalidateTag).not.toHaveBeenCalledWith(
      "user:ALICE",
      "max"
    );
    expect(
      mockState.revalidateTag.mock.calls.filter(
        (call: unknown[]) => call[0] === "user-rank:bob"
      )
    ).toHaveLength(1);
    expect(mockState.revalidatePath).toHaveBeenCalledWith("/u/Bob");
    expect(mockState.revalidatePath).toHaveBeenCalledWith("/u/bob");
    expect(mockState.revalidatePath).toHaveBeenCalledWith(
      "/api/badge/Bob/svg"
    );
    expect(mockState.revalidatePath).toHaveBeenCalledWith(
      "/api/badge/bob/svg"
    );
    expect(
      mockState.revalidateLeaderboardPublicSurfacePaths
    ).toHaveBeenCalledTimes(1);
    expect(
      mockState.revalidateUserGroupLeaderboards
    ).toHaveBeenCalledExactlyOnceWith("user-1");
  });
});
