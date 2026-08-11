import { beforeEach, describe, expect, it, vi } from "vitest";

const { mockExecute } = vi.hoisted(() => ({ mockExecute: vi.fn() }));

vi.mock("@/lib/db", () => ({
  db: { execute: mockExecute },
}));

import {
  getRatchetCensusReport,
  normalizeRatchetCensusCandidateLimit,
  RATCHET_CENSUS_DEFAULT_CANDIDATE_LIMIT,
  RATCHET_CENSUS_MAX_CANDIDATE_LIMIT,
} from "@/lib/reconciliation/ratchetCensus";

function reportFixture() {
  return {
    coverage: {
      totalUsers: 10,
      measuredUsers: 7,
      totalTokens: "90071992547409930",
      measuredTokens: "80000000000000000",
      userCoverage: 0.7,
      tokenCoverage: 0.888889,
      pendingWorkItems: 2,
    },
    divergenceBands: [
      { band: "warming", users: 3, tokens: "100" },
      { band: "clean", users: 6, tokens: "700" },
      { band: "severe", users: 1, tokens: "200" },
    ],
    observedCells: {
      comparableCells: 25,
      under: 1,
      clean: 20,
      mild: 2,
      clear: 1,
      severe: 1,
      maxRatio: 3.5,
    },
    segments: {
      byOrigin: [
        { key: "cli", expectedCells: 100, measuredCells: 90, cellCoverage: 0.9 },
      ],
      byClient: [
        { key: "claude", expectedCells: 50, measuredCells: 45, cellCoverage: 0.9 },
      ],
      byCliVersion: [
        {
          cliVersion: "4.12.0",
          users: 5,
          measuredUsers: 4,
          totalTokens: "600",
          measuredTokens: "500",
        },
      ],
    },
    candidates: [
      {
        username: "candidate",
        totalTokens: "200",
        highwaterTokens: "50",
        ratio: 4,
        band: "severe",
        expectedCells: 4,
        measuredCells: 4,
        cliVersion: "4.12.0",
        deviceCount: 2,
      },
    ],
  };
}

function flattenSqlChunks(node: unknown): unknown[] {
  if (
    node &&
    typeof node === "object" &&
    Array.isArray((node as { queryChunks?: unknown }).queryChunks)
  ) {
    return (node as { queryChunks: unknown[] }).queryChunks.flatMap(flattenSqlChunks);
  }
  if (
    node &&
    typeof node === "object" &&
    Array.isArray((node as { value?: unknown }).value)
  ) {
    return (node as { value: unknown[] }).value;
  }
  return [node];
}

describe("ratchet census report", () => {
  beforeEach(() => mockExecute.mockReset());

  it("preserves bigint token totals as strings and adds the snapshot timestamp", async () => {
    mockExecute.mockResolvedValue([{ report: reportFixture() }]);

    const report = await getRatchetCensusReport({
      candidateLimit: 12,
      now: new Date("2026-08-08T12:00:00.000Z"),
    });

    expect(report.coverage.totalTokens).toBe("90071992547409930");
    expect(report.candidates[0].highwaterTokens).toBe("50");
    expect(report.generatedAt).toBe("2026-08-08T12:00:00.000Z");
    expect(mockExecute).toHaveBeenCalledTimes(1);
  });

  it("keeps census coverage origin-aware and never treats an omitted shadow cell as zero", async () => {
    mockExecute.mockResolvedValue([{ report: reportFixture() }]);

    await getRatchetCensusReport();

    const query = flattenSqlChunks(mockExecute.mock.calls[0][0]).join(" ");
    expect(query).toContain("source.value #>> '{provenance,origin}'");
    expect(query).toContain("t.origin = e.origin");
    expect(query).toContain("db.source_breakdown ? r.client");
    expect(query).not.toMatch(/\b(?:INSERT|UPDATE|DELETE|MERGE)\b/i);
  });

  it("fails closed when the database result does not match the report contract", async () => {
    mockExecute.mockResolvedValue([{ report: { coverage: {} } }]);
    await expect(getRatchetCensusReport()).rejects.toThrow();
  });
});

describe("normalizeRatchetCensusCandidateLimit", () => {
  it.each([null, "", "0", "-4", "1.5", "nope"])(
    "uses the default for %j",
    (value) => {
      expect(normalizeRatchetCensusCandidateLimit(value)).toBe(
        RATCHET_CENSUS_DEFAULT_CANDIDATE_LIMIT
      );
    }
  );

  it("accepts positive integers and caps expensive requests", () => {
    expect(normalizeRatchetCensusCandidateLimit("17")).toBe(17);
    expect(normalizeRatchetCensusCandidateLimit("10000")).toBe(
      RATCHET_CENSUS_MAX_CANDIDATE_LIMIT
    );
  });
});
