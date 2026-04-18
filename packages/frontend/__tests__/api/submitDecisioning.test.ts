import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
  const revalidateTag = vi.fn();

  const db = {
    transaction: vi.fn(),
  };

  return {
    authenticatePersonalToken,
    validateSubmission,
    generateSubmissionHash,
    revalidateTag,
    db,
    reset() {
      authenticatePersonalToken.mockReset();
      validateSubmission.mockReset();
      generateSubmissionHash.mockClear();
      revalidateTag.mockClear();
      db.transaction.mockReset();
    },
  };
});

vi.mock("next/cache", () => ({
  revalidateTag: mockState.revalidateTag,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: mockState.authenticatePersonalToken,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  apiTokens: {
    id: "apiTokens.id",
  },
  submissions: {
    id: "submissions.id",
    userId: "submissions.userId",
    schemaVersion: "submissions.schemaVersion",
  },
  submissionReviews: {
    id: "submissionReviews.id",
  },
  dailyBreakdown: {
    id: "dailyBreakdown.id",
    submissionId: "dailyBreakdown.submissionId",
  },
}));

vi.mock("@/lib/validation/submission", () => ({
  validateSubmission: mockState.validateSubmission,
  generateSubmissionHash: mockState.generateSubmissionHash,
}));

vi.mock("@/lib/db/helpers", () => ({
  mergeClientBreakdowns: vi.fn(),
  recalculateDayTotals: vi.fn(),
  buildModelBreakdown: vi.fn(),
  clientContributionToBreakdownData: vi.fn(),
  mergeTimestampMs: vi.fn(),
}));

type ModuleExports = typeof import("../../src/app/api/submit/route");

let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/submit/route");
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
});

function createValidAuthRecord() {
  return {
    status: "valid" as const,
    tokenId: "token-1",
    userId: "user-1",
    username: "alice",
    displayName: "Alice",
    avatarUrl: null,
    isAdmin: false,
    expiresAt: null,
  };
}

function createSubmissionPayload(date: string) {
  return {
    meta: {
      generatedAt: new Date().toISOString(),
      version: "1.0.0",
      dateRange: {
        start: date,
        end: date,
      },
    },
    summary: {
      totalTokens: 150,
      totalCost: 1.5,
      totalDays: 1,
      activeDays: 1,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: ["claude"],
      models: ["claude-sonnet-4-20250514"],
    },
    years: [],
    contributions: [
      {
        date,
        timestampMs: Date.parse(`${date}T10:00:00.000Z`),
        totals: { tokens: 150, cost: 1.5, messages: 1 },
        intensity: 1 as const,
        tokenBreakdown: {
          input: 100,
          output: 50,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        clients: [
          {
            client: "claude" as const,
            modelId: "claude-sonnet-4-20250514",
            tokens: {
              input: 100,
              output: 50,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 1.5,
            messages: 1,
          },
        ],
      },
    ],
  };
}

describe("POST /api/submit trust decisioning", () => {
  it("returns an explicit trusted response envelope for accepted competitive writes", async () => {
    const today = new Date().toISOString().slice(0, 10);
    const payload = createSubmissionPayload(today);

    mockState.authenticatePersonalToken.mockResolvedValue(createValidAuthRecord());
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      errors: [],
      warnings: ["Cost total minor mismatch: summary=1.50, calculated=1.50"],
      trustState: "trusted",
      reasonCodes: [],
      rejectionReasonCodes: [],
      data: payload,
    });
    mockState.db.transaction.mockResolvedValue({
      trustState: "trusted",
      submissionId: "submission-1",
      isNewSubmission: true,
      metrics: {
        totalTokens: 150,
        totalCost: 1.5,
        dateRange: { start: today, end: today },
        activeDays: 1,
        clients: ["claude"],
      },
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify(payload),
      })
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      submissionId: "submission-1",
      reviewId: undefined,
      username: "alice",
      metrics: {
        totalTokens: 150,
        totalCost: 1.5,
        dateRange: { start: today, end: today },
        activeDays: 1,
        clients: ["claude"],
      },
      trustState: "trusted",
      mode: "create",
      reasonCodes: undefined,
      competitiveWriteApplied: true,
      warnings: ["Cost total minor mismatch: summary=1.50, calculated=1.50"],
    });
    expect(mockState.revalidateTag).toHaveBeenCalledTimes(4);
  });

  it("hard-rejects impossible timeline payloads with machine-readable error codes", async () => {
    const payload = createSubmissionPayload("2024-12-01");

    mockState.authenticatePersonalToken.mockResolvedValue(createValidAuthRecord());
    mockState.validateSubmission.mockReturnValue({
      valid: false,
      errors: ["Day 2024-12-01 has timestamp 1733097900000 outside its claimed UTC bucket"],
      warnings: [],
      trustState: "rejected",
      reasonCodes: [],
      rejectionReasonCodes: ["timestamp_day_mismatch"],
      data: undefined,
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify(payload),
      })
    );

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      error: "Validation failed",
      details: ["Day 2024-12-01 has timestamp 1733097900000 outside its claimed UTC bucket"],
      trustState: "rejected",
      errorCodes: ["timestamp_day_mismatch"],
    });
    expect(mockState.db.transaction).not.toHaveBeenCalled();
    expect(mockState.revalidateTag).not.toHaveBeenCalled();
  });

  it("accepts suspicious history into the review path with explicit non-trusted decisioning", async () => {
    const oldDate = "2024-12-01";
    const payload = {
      ...createSubmissionPayload(oldDate),
      contributions: [
        {
          ...createSubmissionPayload(oldDate).contributions[0],
          timestampMs: undefined,
        },
      ],
    };

    mockState.authenticatePersonalToken.mockResolvedValue(createValidAuthRecord());
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      errors: [],
      warnings: [
        "Day 2024-12-01 is older than 30 days and has no timestampMs audit metadata",
      ],
      trustState: "review_required",
      reasonCodes: ["historical_day_missing_timestamp"],
      rejectionReasonCodes: [],
      data: payload,
    });
    mockState.db.transaction.mockResolvedValue({
      trustState: "review_required",
      reviewId: "review-1",
      metrics: {
        totalTokens: 150,
        totalCost: 1.5,
        dateRange: { start: oldDate, end: oldDate },
        activeDays: 1,
        clients: ["claude"],
      },
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify(payload),
      })
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      submissionId: undefined,
      reviewId: "review-1",
      username: "alice",
      metrics: {
        totalTokens: 150,
        totalCost: 1.5,
        dateRange: { start: oldDate, end: oldDate },
        activeDays: 1,
        clients: ["claude"],
      },
      trustState: "review_required",
      mode: "review",
      reasonCodes: ["historical_day_missing_timestamp"],
      competitiveWriteApplied: false,
      warnings: [
        "Day 2024-12-01 is older than 30 days and has no timestampMs audit metadata",
      ],
    });
    expect(mockState.revalidateTag).not.toHaveBeenCalled();
  });
});
