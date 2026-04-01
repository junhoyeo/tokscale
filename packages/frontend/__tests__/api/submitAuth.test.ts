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
  submissions: {
    id: "submissions.id",
    userId: "submissions.userId",
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

describe("POST /api/submit auth path", () => {
  it("rejects invalid API tokens through the shared auth service", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({ status: "invalid" });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_invalid",
        },
        body: JSON.stringify({}),
      })
    );

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_invalid", {
      touchLastUsedAt: false,
    });
    expect(await response.json()).toEqual({ error: "Invalid API token" });
  });

  it("returns the expired-token error without entering the transaction path", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({ status: "expired" });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_expired",
        },
        body: JSON.stringify({}),
      })
    );

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_expired", {
      touchLastUsedAt: false,
    });
    expect(await response.json()).toEqual({ error: "API token has expired" });
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("accepts a valid token and continues into submission validation", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: false,
      data: null,
      errors: ["bad payload"],
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(400);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_valid", {
      touchLastUsedAt: false,
    });
    expect(mockState.validateSubmission).toHaveBeenCalledTimes(1);
    expect(mockState.db.transaction).not.toHaveBeenCalled();
    expect(await response.json()).toEqual({
      error: "Validation failed",
      details: ["bad payload"],
    });
  });

  it("returns 409 when source identity is required after scoped mode begins", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          generatedAt: new Date().toISOString(),
          version: "1.0.0",
          dateRange: { start: "2024-12-01", end: "2024-12-01" },
        },
        summary: {
          totalTokens: 1500,
          totalCost: 1.5,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 1.5,
          maxCostInSingleDay: 1.5,
          clients: ["claude"],
          models: ["claude-sonnet-4"],
        },
        years: [],
        contributions: [
          {
            date: "2024-12-01",
            totals: { tokens: 1500, cost: 1.5, messages: 5 },
            intensity: 2,
            tokenBreakdown: {
              input: 1000,
              output: 500,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "claude",
                modelId: "claude-sonnet-4",
                tokens: {
                  input: 1000,
                  output: 500,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 1.5,
                messages: 5,
              },
            ],
          },
        ],
      },
      errors: [],
    });
    mockState.db.transaction.mockRejectedValue(
      new Error("Source identity is required for accounts with source-scoped submissions")
    );

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(409);
    expect(await response.json()).toEqual({
      error: "Source identity is required for accounts with source-scoped submissions",
    });
  });
});
