import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const authenticateSubmissionReviewOperator = vi.fn();
  const listSubmissionReviewArtifacts = vi.fn();
  const getSubmissionReviewArtifact = vi.fn();
  const adjudicateSubmissionReview = vi.fn();
  const revalidateSubmissionPublicCaches = vi.fn();

  return {
    authenticateSubmissionReviewOperator,
    listSubmissionReviewArtifacts,
    getSubmissionReviewArtifact,
    adjudicateSubmissionReview,
    revalidateSubmissionPublicCaches,
    reset() {
      authenticateSubmissionReviewOperator.mockReset();
      listSubmissionReviewArtifacts.mockReset();
      getSubmissionReviewArtifact.mockReset();
      adjudicateSubmissionReview.mockReset();
      revalidateSubmissionPublicCaches.mockReset();
    },
  };
});

vi.mock("@/lib/submissionReviewAuth", () => ({
  authenticateSubmissionReviewOperator:
    mockState.authenticateSubmissionReviewOperator,
}));

vi.mock("../../src/lib/submissionReviewAuth", () => ({
  authenticateSubmissionReviewOperator:
    mockState.authenticateSubmissionReviewOperator,
}));

vi.mock("../../src/lib/leaderboard/publicCacheInvalidation", () => ({
  revalidateSubmissionPublicCaches:
    mockState.revalidateSubmissionPublicCaches,
}));

vi.mock("@/lib/submissionReviews", () => ({
  REVIEW_FILTER_STATE: {
    REVIEW_REQUIRED: "review_required",
    TRUSTED: "trusted",
    REJECTED: "rejected",
  },
  REVIEW_ARTIFACT_RESULT_KIND: {
    UPDATED: "updated",
    NOT_FOUND: "not_found",
    CONFLICT: "conflict",
  },
  listSubmissionReviewArtifacts: mockState.listSubmissionReviewArtifacts,
  getSubmissionReviewArtifact: mockState.getSubmissionReviewArtifact,
  adjudicateSubmissionReview: mockState.adjudicateSubmissionReview,
}));

vi.mock("../../src/lib/submissionReviews", () => ({
  REVIEW_FILTER_STATE: {
    REVIEW_REQUIRED: "review_required",
    TRUSTED: "trusted",
    REJECTED: "rejected",
  },
  REVIEW_ARTIFACT_RESULT_KIND: {
    UPDATED: "updated",
    NOT_FOUND: "not_found",
    CONFLICT: "conflict",
  },
  listSubmissionReviewArtifacts: mockState.listSubmissionReviewArtifacts,
  getSubmissionReviewArtifact: mockState.getSubmissionReviewArtifact,
  adjudicateSubmissionReview: mockState.adjudicateSubmissionReview,
}));

type ListRouteModule = typeof import("../../src/app/api/reviews/submissions/route");
type DetailRouteModule = typeof import("../../src/app/api/reviews/submissions/[reviewId]/route");

let listRouteGet: ListRouteModule["GET"];
let detailRouteGet: DetailRouteModule["GET"];
let detailRoutePatch: DetailRouteModule["PATCH"];

beforeAll(async () => {
  const listRouteModule = await import("../../src/app/api/reviews/submissions/route");
  listRouteGet = listRouteModule.GET;

  const detailRouteModule = await import(
    "../../src/app/api/reviews/submissions/[reviewId]/route"
  );
  detailRouteGet = detailRouteModule.GET;
  detailRoutePatch = detailRouteModule.PATCH;
});

beforeEach(() => {
  mockState.reset();
  mockState.authenticateSubmissionReviewOperator.mockReturnValue({
    username: "moderator",
  });
});

function createArtifact(trustState = "review_required") {
  return {
    id: "11111111-1111-4111-8111-111111111111",
    user: {
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    },
    submissionHash: "submission-hash",
    trustState,
    reasonCodes: ["historical_day_missing_timestamp"],
    payload: {
      meta: {
        version: "1.0.0",
      },
    },
    totals: {
      totalTokens: 150,
      totalCost: 1.5,
      activeDays: 1,
    },
    dateRange: {
      start: "2024-12-01",
      end: "2024-12-01",
    },
    clients: ["claude"],
    models: ["claude-sonnet-4-20250514"],
    createdAt: "2026-04-18T11:00:00.000Z",
    updatedAt: "2026-04-18T11:05:00.000Z",
    review: {
      reviewedAt: null,
      reviewedByUsername: null,
      reviewNote: null,
    },
    audit: {
      cliVersion: "1.0.0",
      schemaVersion: 0,
    },
  };
}

describe("submission review routes", () => {
  it("rejects requests without dedicated operator credentials", async () => {
    mockState.authenticateSubmissionReviewOperator.mockReturnValue(null);

    const response = await listRouteGet(
      new Request("http://localhost:3000/api/reviews/submissions")
    );

    expect(response.status).toBe(401);
    expect(mockState.listSubmissionReviewArtifacts).not.toHaveBeenCalled();
  });

  it("rejects malformed review identifiers before querying", async () => {
    const response = await detailRouteGet(
      new Request("http://localhost:3000/api/reviews/submissions/not-a-uuid"),
      {
        params: Promise.resolve({ reviewId: "not-a-uuid" }),
      }
    );

    expect(response.status).toBe(400);
    expect(mockState.getSubmissionReviewArtifact).not.toHaveBeenCalled();
  });

  it("lists persisted review artifacts for review operators", async () => {
    mockState.listSubmissionReviewArtifacts.mockResolvedValue([
      createArtifact(),
    ]);

    const response = await listRouteGet(
      new Request("http://localhost:3000/api/reviews/submissions")
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      reviews: [createArtifact()],
    });
    expect(mockState.listSubmissionReviewArtifacts).toHaveBeenCalledWith({
      trustState: "review_required",
    });
  });

  it("loads a single persisted review artifact for follow-up inspection", async () => {
    mockState.getSubmissionReviewArtifact.mockResolvedValue(createArtifact());

    const response = await detailRouteGet(
      new Request("http://localhost:3000/api/reviews/submissions/11111111-1111-4111-8111-111111111111"),
      {
        params: Promise.resolve({ reviewId: "11111111-1111-4111-8111-111111111111" }),
      }
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      review: createArtifact(),
    });
    expect(mockState.getSubmissionReviewArtifact).toHaveBeenCalledWith(
      "11111111-1111-4111-8111-111111111111"
    );
  });

  it("rejects oversized adjudication bodies before service execution", async () => {
    const response = await detailRoutePatch(
      new Request("http://localhost:3000/api/reviews/submissions/11111111-1111-4111-8111-111111111111", {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          trustState: "trusted",
          extra: "x".repeat(20 * 1024),
        }),
      }),
      {
        params: Promise.resolve({ reviewId: "11111111-1111-4111-8111-111111111111" }),
      }
    );

    expect(response.status).toBe(413);
    expect(mockState.adjudicateSubmissionReview).not.toHaveBeenCalled();
  });

  it.each(["null", "[]", "true", "1", '"text"'])(
    "rejects non-object adjudication body %s",
    async (body) => {
      const response = await detailRoutePatch(
        new Request(
          "http://localhost:3000/api/reviews/submissions/11111111-1111-4111-8111-111111111111",
          {
            method: "PATCH",
            headers: {
              "Content-Type": "application/json",
            },
            body,
          }
        ),
        {
          params: Promise.resolve({
            reviewId: "11111111-1111-4111-8111-111111111111",
          }),
        }
      );

      expect(response.status).toBe(400);
      expect(mockState.adjudicateSubmissionReview).not.toHaveBeenCalled();
    }
  );

  it("promotes suspicious history through the adjudication route and revalidates competitive caches", async () => {
    const trustedArtifact = createArtifact("trusted");


    mockState.adjudicateSubmissionReview.mockResolvedValue({
      kind: "updated",
      artifact: trustedArtifact,
      competitiveWriteApplied: true,
      affectedCompetitiveUsernames: ["alice", "bob"],
    });

    const response = await detailRoutePatch(
      new Request("http://localhost:3000/api/reviews/submissions/11111111-1111-4111-8111-111111111111", {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          trustState: "trusted",
          reviewNote: "Validated against source logs",
        }),
      }),
      {
        params: Promise.resolve({ reviewId: "11111111-1111-4111-8111-111111111111" }),
      }
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      review: trustedArtifact,
      competitiveWriteApplied: true,
    });
    expect(mockState.adjudicateSubmissionReview).toHaveBeenCalledWith({
      reviewId: "11111111-1111-4111-8111-111111111111",
      trustState: "trusted",
      reviewedByUsername: "moderator",
      reviewNote: "Validated against source logs",
    });
    expect(
      mockState.revalidateSubmissionPublicCaches
    ).toHaveBeenCalledExactlyOnceWith("user-1", "alice", ["alice", "bob"]);
  });

  it("rejects suspicious history through the adjudication route without competitive cache refreshes", async () => {
    const rejectedArtifact = createArtifact("rejected");


    mockState.adjudicateSubmissionReview.mockResolvedValue({
      kind: "updated",
      artifact: rejectedArtifact,
      competitiveWriteApplied: false,
      affectedCompetitiveUsernames: [],
    });

    const response = await detailRoutePatch(
      new Request("http://localhost:3000/api/reviews/submissions/11111111-1111-4111-8111-111111111111", {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          trustState: "rejected",
          reviewNote: "Historical evidence was incomplete",
        }),
      }),
      {
        params: Promise.resolve({ reviewId: "11111111-1111-4111-8111-111111111111" }),
      }
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      review: rejectedArtifact,
      competitiveWriteApplied: false,
    });
    expect(
      mockState.revalidateSubmissionPublicCaches
    ).not.toHaveBeenCalled();
  });
});
