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

    const assessment = assessSubmissionTrust(submission, NOW);

    expect(assessment.trustState).toBe(
      SUBMISSION_TRUST_STATE.REVIEW_REQUIRED
    );
    expect(assessment.reasonCodes).toContain(
      SUBMISSION_REASON_CODE.MODEL_PREDATES_PUBLIC_AVAILABILITY
    );
    expect(assessment.rejectionReasonCodes).toEqual([]);
    expect(assessment.reviewDates).toEqual(["2024-08-05"]);
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
