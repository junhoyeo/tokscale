import { beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const transaction = vi.fn();
  const applyTrustedSubmission = vi.fn();
  const validateSubmission = vi.fn();

  return {
    transaction,
    applyTrustedSubmission,
    validateSubmission,
    reset() {
      transaction.mockReset();
      applyTrustedSubmission.mockReset();
      validateSubmission.mockReset();
    },
  };
});

vi.mock("../../src/lib/db", () => ({
  db: {
    transaction: mockState.transaction,
  },
  submissionReviews: {
    id: "submissionReviews.id",
    userId: "submissionReviews.userId",
    submissionHash: "submissionReviews.submissionHash",
    trustState: "submissionReviews.trustState",
    competitiveWriteApplied: "submissionReviews.competitiveWriteApplied",
    reasonCodes: "submissionReviews.reasonCodes",
    payload: "submissionReviews.payload",
    totalTokens: "submissionReviews.totalTokens",
    totalCost: "submissionReviews.totalCost",
    activeDays: "submissionReviews.activeDays",
    dateStart: "submissionReviews.dateStart",
    dateEnd: "submissionReviews.dateEnd",
    sourcesUsed: "submissionReviews.sourcesUsed",
    modelsUsed: "submissionReviews.modelsUsed",
    cliVersion: "submissionReviews.cliVersion",
    schemaVersion: "submissionReviews.schemaVersion",
    createdAt: "submissionReviews.createdAt",
    updatedAt: "submissionReviews.updatedAt",
    reviewedAt: "submissionReviews.reviewedAt",
    reviewedByUsername: "submissionReviews.reviewedByUsername",
    reviewNote: "submissionReviews.reviewNote",
  },
  users: {
    id: "users.id",
    username: "users.username",
    displayName: "users.displayName",
    avatarUrl: "users.avatarUrl",
  },
  submissions: {
    totalTokens: "submissions.totalTokens",
    totalCost: "submissions.totalCost",
    userId: "submissions.userId",
  },
}));

vi.mock("../../src/lib/submissionPersistence", () => ({
  applyTrustedSubmission: mockState.applyTrustedSubmission,
}));

vi.mock("../../src/lib/validation/submission", () => ({
  validateSubmission: mockState.validateSubmission,
}));

import { adjudicateSubmissionReview } from "../../src/lib/submissionReviews";

function makeSelectBuilder(result: unknown) {
  const builder = {
    from: vi.fn(() => builder),
    innerJoin: vi.fn(() => builder),
    where: vi.fn(() => builder),
    for: vi.fn(() => builder),
    limit: vi.fn(() => Promise.resolve(result)),
  };
  return builder;
}

describe("adjudicateSubmissionReview persistence", () => {
  beforeEach(() => {
    mockState.reset();
  });

  it("forwards the queued receipt watermark and suppresses a second submit count", async () => {
    const receivedAt = new Date("2026-07-14T11:00:00.000Z");
    const adjudicatedAt = new Date("2026-07-14T13:00:00.000Z");
    const payload = {
      meta: {
        version: "2.1.1",
        dateRange: { start: "2026-07-01", end: "2026-07-01" },
      },
      summary: { clients: ["claude"], models: ["claude-sonnet"] },
      contributions: [],
    };
    const currentReview = {
      id: "review-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      submissionHash: "a".repeat(64),
      trustState: "review_required",
      competitiveWriteApplied: true,
      reasonCodes: ["historical_day_missing_timestamp"],
      payload: { ...payload, mcpServers: ["github"] },
      totalTokens: 100,
      totalCost: "0.2500",
      activeDays: 1,
      dateStart: "2026-07-01",
      dateEnd: "2026-07-01",
      sourcesUsed: ["claude"],
      modelsUsed: ["claude-sonnet"],
      cliVersion: "2.1.1",
      schemaVersion: 1,
      createdAt: new Date("2026-07-14T10:00:00.000Z"),
      updatedAt: receivedAt,
      reviewedAt: null,
      reviewedByUsername: null,
      reviewNote: null,
    };
    const updatedReview = {
      trustState: "trusted",
      updatedAt: adjudicatedAt,
      reviewedAt: adjudicatedAt,
      reviewedByUsername: "moderator",
      reviewNote: "Verified",
    };
    const updateBuilder = {
      set: vi.fn(() => updateBuilder),
      where: vi.fn(() => updateBuilder),
      returning: vi.fn(() => Promise.resolve([updatedReview])),
    };
    const tx = {
      select: vi.fn(() => makeSelectBuilder([currentReview])),
      update: vi.fn(() => updateBuilder),
      execute: vi.fn(() => Promise.resolve([])),
    };

    mockState.transaction.mockImplementation(
      async (callback: (transaction: typeof tx) => Promise<unknown>) =>
        callback(tx)
    );
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: payload,
      errors: [],
      warnings: [],
    });

    const result = await adjudicateSubmissionReview({
      reviewId: "review-1",
      trustState: "trusted",
      reviewedByUsername: "moderator",
      reviewNote: "Verified",
    });

    expect(result.kind).toBe("updated");
    expect(mockState.applyTrustedSubmission).toHaveBeenCalledExactlyOnceWith(
      tx,
      expect.objectContaining({
        userId: "user-1",
        data: payload,
        mcpServers: ["github"],
        metadataReceivedAt: receivedAt,
        incrementSubmitCount: false,
      })
    );
  });
});
