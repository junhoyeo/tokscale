import { describe, expect, it } from "vitest";
import {
  generateSubmissionHash,
  validateSubmission,
  type SubmissionData,
} from "../../src/lib/validation/submission";
import {
  assessSubmissionTrust,
  subsetSubmissionByDates,
  SUBMISSION_REASON_CODE,
  SUBMISSION_TRUST_STATE,
} from "../../src/lib/validation/submissionTrust";

type TestDay = {
  date: string;
  modelId?: string;
  timestampMs?: number;
  tokens?: number;
  cost?: number;
};

function createSubmission(days: TestDay[]): SubmissionData {
  const contributions = days.map((day) => {
    const tokens = day.tokens ?? 100;
    const cost = day.cost ?? 0.25;
    return {
      date: day.date,
      ...(day.timestampMs == null ? {} : { timestampMs: day.timestampMs }),
      totals: { tokens, cost, messages: 1 },
      intensity: 1,
      tokenBreakdown: {
        input: tokens,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
      },
      clients: [
        {
          client: "claude",
          modelId: day.modelId ?? "claude-sonnet",
          providerId: "anthropic",
          tokens: {
            input: tokens,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost,
          messages: 1,
        },
      ],
    };
  });
  const totalTokens = contributions.reduce(
    (sum, day) => sum + day.totals.tokens,
    0
  );
  const totalCost = contributions.reduce(
    (sum, day) => sum + day.totals.cost,
    0
  );
  const sortedDates = days.map((day) => day.date).sort();
  const years = Array.from(new Set(sortedDates.map((date) => date.slice(0, 4)))).map(
    (year) => {
      const yearDays = contributions.filter((day) => day.date.startsWith(year));
      return {
        year,
        totalTokens: yearDays.reduce((sum, day) => sum + day.totals.tokens, 0),
        totalCost: yearDays.reduce((sum, day) => sum + day.totals.cost, 0),
        range: {
          start: yearDays[0].date,
          end: yearDays[yearDays.length - 1].date,
        },
      };
    }
  );

  const result = validateSubmission({
    meta: {
      generatedAt: "2026-07-14T00:00:00.000Z",
      version: "2.1.1",
      dateRange: {
        start: sortedDates[0],
        end: sortedDates[sortedDates.length - 1],
      },
    },
    summary: {
      totalTokens,
      totalCost,
      totalDays: contributions.length,
      activeDays: contributions.length,
      averagePerDay: totalTokens / contributions.length,
      maxCostInSingleDay: Math.max(
        ...contributions.map((day) => day.totals.cost)
      ),
      clients: ["claude"],
      models: Array.from(
        new Set(contributions.map((day) => day.clients[0].modelId))
      ),
    },
    years,
    contributions,
  });

  expect(result.errors).toEqual([]);
  expect(result.data).toBeDefined();
  return result.data!;
}

const NOW = new Date("2026-07-14T12:00:00.000Z");

describe("submit trust decisioning", () => {
  it("trusts a fresh timestamp-matched payload", () => {
    const submission = createSubmission([
      {
        date: "2026-07-10",
        timestampMs: Date.parse("2026-07-10T12:00:00.000Z"),
      },
    ]);

    expect(assessSubmissionTrust(submission, NOW)).toEqual({
      trustState: SUBMISSION_TRUST_STATE.TRUSTED,
      reasonCodes: [],
      rejectionReasonCodes: [],
      reviewDates: [],
      errors: [],
      warnings: [],
    });

  });

  it("rejects duplicate dates during strict validation", () => {
    const submission = createSubmission([{ date: "2026-07-10" }]);
    const duplicate = {
      ...submission,
      contributions: [
        submission.contributions[0],
        submission.contributions[0],
      ],
    };

    const result = validateSubmission(duplicate);

    expect(result.valid).toBe(false);
    expect(result.errors).toContain("Duplicate date found: 2026-07-10");
  });

  it("rejects contribution totals that have no client data", () => {
    const submission = createSubmission([{ date: "2026-07-10" }]);
    submission.contributions[0].clients = [];

    const result = validateSubmission(submission);

    expect(result.valid).toBe(false);
    expect(result.errors).toContain(
      "Contribution day 2026-07-10 has no client data"
    );
  });

  it("counts message-only contributions as active days", () => {
    const submission = createSubmission([
      { date: "2026-07-10", tokens: 0, cost: 0 },
    ]);

    const result = validateSubmission(submission);

    expect(result.valid).toBe(true);
    expect(result.warnings).not.toContainEqual(
      expect.stringContaining("Active days mismatch")
    );
  });

  it("rejects oversized model identifiers", () => {
    const submission = createSubmission([{ date: "2026-07-10" }]);
    const oversizedModelId = "m".repeat(513);
    submission.contributions[0].clients[0].modelId = oversizedModelId;
    submission.summary.models = [oversizedModelId];

    const result = validateSubmission(submission);

    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("contributions.0.clients.0.modelId"),
        expect.stringContaining("summary.models.0"),
      ])
    );
  });

  it("rejects future-dated payloads during strict validation", () => {
    const result = validateSubmission({
      ...createSubmission([{ date: "2026-07-10" }]),
      meta: {
        generatedAt: "2026-07-14T00:00:00.000Z",
        version: "2.1.1",
        dateRange: { start: "2999-01-01", end: "2999-01-01" },
      },
      contributions: [
        {
          ...createSubmission([{ date: "2026-07-10" }]).contributions[0],
          date: "2999-01-01",
        },
      ],
    });

    expect(result.valid).toBe(false);
    expect(
      result.errors.some((error) => error.includes("Future date"))
    ).toBe(true);
  });

  it("rejects impossible calendar dates before database writes", () => {
    const submission = createSubmission([{ date: "2026-02-28" }]);
    submission.meta.dateRange = {
      start: "2026-02-31",
      end: "2026-02-31",
    };
    submission.years[0].range = {
      start: "2026-02-31",
      end: "2026-02-31",
    };
    submission.contributions[0].date = "2026-02-31";

    const result = validateSubmission(submission);

    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("meta.dateRange.start: Invalid calendar date"),
        expect.stringContaining("contributions.0.date: Invalid calendar date"),
      ])
    );
  });

  it("rejects daily costs that cannot fit the database column", () => {
    const submission = createSubmission([{ date: "2026-07-10" }]);
    const oversizedDailyCost = 10_000_000_000;
    submission.summary.totalCost = oversizedDailyCost;
    submission.summary.averagePerDay = oversizedDailyCost;
    submission.summary.maxCostInSingleDay = oversizedDailyCost;
    submission.years[0].totalCost = oversizedDailyCost;
    submission.contributions[0].totals.cost = oversizedDailyCost;
    submission.contributions[0].clients[0].cost = oversizedDailyCost;

    const result = validateSubmission(submission);

    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("summary.maxCostInSingleDay"),
        expect.stringContaining("contributions.0.totals.cost"),
        expect.stringContaining("contributions.0.clients.0.cost"),
      ])
    );
  });

  it("queues only old untimestamped days for review", () => {
    const submission = createSubmission([
      { date: "2026-01-01", tokens: 400, cost: 1 },
      { date: "2026-07-10", tokens: 100, cost: 0.25 },
    ]);

    const assessment = assessSubmissionTrust(submission, NOW);

    expect(assessment.trustState).toBe(
      SUBMISSION_TRUST_STATE.REVIEW_REQUIRED
    );
    expect(assessment.reasonCodes).toEqual([
      SUBMISSION_REASON_CODE.HISTORICAL_DAY_MISSING_TIMESTAMP,
    ]);
    expect(assessment.reviewDates).toEqual(["2026-01-01"]);
  });

  it("pins the historical timestamp boundary", () => {
    const boundaryDay = createSubmission([{ date: "2026-06-14" }]);
    const outsideWindowDay = createSubmission([{ date: "2026-06-13" }]);
    const timestampedHistoricalDay = createSubmission([
      {
        date: "2026-06-01",
        timestampMs: Date.parse("2026-06-01T12:00:00.000Z"),
      },
    ]);

    expect(assessSubmissionTrust(boundaryDay, NOW).trustState).toBe(
      SUBMISSION_TRUST_STATE.TRUSTED
    );
    expect(assessSubmissionTrust(outsideWindowDay, NOW)).toEqual(
      expect.objectContaining({
        trustState: SUBMISSION_TRUST_STATE.REVIEW_REQUIRED,
        reviewDates: ["2026-06-13"],
      })
    );
    expect(
      assessSubmissionTrust(timestampedHistoricalDay, NOW).trustState
    ).toBe(SUBMISSION_TRUST_STATE.TRUSTED);
  });

  it("partitions every contribution date exactly once", () => {
    const submission = createSubmission([
      { date: "2026-01-01" },
      {
        date: "2026-01-02",
        timestampMs: Date.parse("2026-01-02T12:00:00.000Z"),
      },
      { date: "2026-07-10" },
      {
        date: "2024-08-05",
        modelId: "gpt-4o-2024-08-06",
        timestampMs: Date.parse("2024-08-05T12:00:00.000Z"),
      },
    ]);
    submission.timeMetrics = {
      totalActiveTimeMs: 100,
      longestContinuousMs: 100,
      maxConcurrentSessions: 1,
      sessionCount: 1,
    };

    const assessment = assessSubmissionTrust(submission, NOW);
    const reviewedDates = new Set(assessment.reviewDates);
    const trustedDates = new Set(
      submission.contributions
        .map((day) => day.date)
        .filter((date) => !reviewedDates.has(date))
    );
    const reviewed = subsetSubmissionByDates(submission, reviewedDates);
    const trusted = subsetSubmissionByDates(submission, trustedDates);

    expect(assessment.reviewDates).toEqual(["2024-08-05", "2026-01-01"]);
    expect(
      assessment.reviewDates.filter((date) => trustedDates.has(date))
    ).toEqual([]);
    expect(
      new Set([
        ...reviewed!.contributions.map((day) => day.date),
        ...trusted!.contributions.map((day) => day.date),
      ])
    ).toEqual(
      new Set(submission.contributions.map((day) => day.date))
    );
    expect(reviewed!.timeMetrics).toBeUndefined();
    expect(trusted!.timeMetrics).toBeUndefined();
  });

  it("recomputes queued payload metadata from only reviewed days", () => {
    const submission = createSubmission([
      { date: "2026-01-01", tokens: 400, cost: 1 },
      { date: "2026-07-10", tokens: 100, cost: 0.25 },
    ]);

    const queued = subsetSubmissionByDates(
      submission,
      new Set(["2026-01-01"])
    );

    expect(queued?.contributions.map((day) => day.date)).toEqual([
      "2026-01-01",
    ]);
    expect(queued?.summary).toEqual(
      expect.objectContaining({
        totalTokens: 400,
        totalCost: 1,
        totalDays: 1,
        activeDays: 1,
        averagePerDay: 1,
      })
    );
    expect(queued?.meta.dateRange).toEqual({
      start: "2026-01-01",
      end: "2026-01-01",
    });
    expect(queued?.years).toEqual([
      {
        year: "2026",
        totalTokens: 400,
        totalCost: 1,
        range: { start: "2026-01-01", end: "2026-01-01" },
      },
    ]);
    expect(generateSubmissionHash(queued!)).not.toBe(
      generateSubmissionHash(submission)
    );
  });

  it("queues a model-date heuristic mismatch instead of rejecting it", () => {
    const submission = createSubmission([
      {
        date: "2024-08-05",
        modelId: "gpt-4o-2024-08-06",
        timestampMs: Date.parse("2024-08-05T12:00:00.000Z"),
      },
    ]);

    submission.contributions[0].clients.push({
      ...submission.contributions[0].clients[0],
    });

    const assessment = assessSubmissionTrust(submission, NOW);

    expect(assessment.trustState).toBe(
      SUBMISSION_TRUST_STATE.REVIEW_REQUIRED
    );
    expect(assessment.reasonCodes).toContain(
      SUBMISSION_REASON_CODE.MODEL_PREDATES_PUBLIC_AVAILABILITY
    );
    expect(assessment.rejectionReasonCodes).toEqual([]);
    expect(assessment.reviewDates).toEqual(["2024-08-05"]);
    expect(assessment.warnings).toHaveLength(1);
  });

  it("parses compact dated model identifiers", () => {
    const submission = createSubmission([
      {
        date: "2024-08-05",
        modelId: "gpt-4o-20240806",
        timestampMs: Date.parse("2024-08-05T12:00:00.000Z"),
      },
    ]);

    expect(assessSubmissionTrust(submission, NOW).reasonCodes).toContain(
      SUBMISSION_REASON_CODE.MODEL_PREDATES_PUBLIC_AVAILABILITY
    );
  });

  it("hard-rejects a timestamp outside the claimed UTC day", () => {
    const submission = createSubmission([
      {
        date: "2026-07-10",
        timestampMs: Date.parse("2026-07-11T00:00:00.000Z"),
      },
    ]);

    const assessment = assessSubmissionTrust(submission, NOW);

    expect(assessment.trustState).toBe(SUBMISSION_TRUST_STATE.REJECTED);
    expect(assessment.rejectionReasonCodes).toEqual([
      SUBMISSION_REASON_CODE.TIMESTAMP_DAY_MISMATCH,
    ]);
  });

  it("hard rejection outranks an independent review signal", () => {
    const submission = createSubmission([
      {
        date: "2026-01-01",
        modelId: "gpt-5.5",
      },
      {
        date: "2026-07-10",
        timestampMs: Date.parse("2026-07-11T00:00:00.000Z"),
      },
    ]);

    const assessment = assessSubmissionTrust(submission, NOW);

    expect(assessment.trustState).toBe(SUBMISSION_TRUST_STATE.REJECTED);
    expect(assessment.rejectionReasonCodes).toEqual([
      SUBMISSION_REASON_CODE.TIMESTAMP_DAY_MISMATCH,
    ]);
    expect(assessment.reasonCodes).toEqual([]);
    expect(assessment.reviewDates).toEqual([]);
  });

  it("returns a structured rejection for timestamps outside the Date range", () => {
    const submission = createSubmission([
      { date: "2026-07-10", timestampMs: Number.MAX_SAFE_INTEGER },
    ]);

    expect(() => assessSubmissionTrust(submission, NOW)).not.toThrow();
    const assessment = assessSubmissionTrust(submission, NOW);
    expect(assessment.trustState).toBe(SUBMISSION_TRUST_STATE.REJECTED);
    expect(assessment.rejectionReasonCodes).toContain(
      SUBMISSION_REASON_CODE.TIMESTAMP_DAY_MISMATCH
    );
  });

  it("does not escalate mixed timestamp coverage by itself", () => {
    const submission = createSubmission([
      {
        date: "2026-07-09",
        timestampMs: Date.parse("2026-07-09T12:00:00.000Z"),
      },
      { date: "2026-07-10" },
    ]);

    expect(assessSubmissionTrust(submission, NOW).trustState).toBe(
      SUBMISSION_TRUST_STATE.TRUSTED
    );
  });
});
