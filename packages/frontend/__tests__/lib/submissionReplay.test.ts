import { describe, expect, it } from "vitest";
import {
  planSubmittedReplayMutations,
  type ClientBreakdownData,
  type ExistingReplayDay,
  type IncomingReplayDay,
} from "../../src/lib/db/helpers";

function createClientBreakdown(
  client: string,
  tokens: number,
  cost: number,
  modelId = `${client}-model`
): Record<string, ClientBreakdownData> {
  return {
    [client]: {
      tokens,
      cost,
      input: tokens,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
      models: {
        [modelId]: {
          tokens,
          cost,
          input: tokens,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 1,
        },
      },
    },
  };
}

function mergeBreakdowns(
  ...breakdowns: Array<Record<string, ClientBreakdownData>>
): Record<string, ClientBreakdownData> {
  return Object.assign({}, ...breakdowns);
}

function createExistingDay(
  id: string,
  date: string,
  sourceBreakdown: Record<string, ClientBreakdownData>,
  timestampMs?: number
): ExistingReplayDay {
  return {
    id,
    date,
    timestampMs: timestampMs ?? null,
    sourceBreakdown,
  };
}

function createIncomingDay(
  date: string,
  sourceBreakdown: Record<string, ClientBreakdownData>,
  timestampMs?: number
): IncomingReplayDay {
  return {
    date,
    timestampMs: timestampMs ?? null,
    sourceBreakdown,
  };
}

describe("planSubmittedReplayMutations", () => {
  it("replaces overlapping in-scope history without stacking and preserves out-of-scope clients", () => {
    const result = planSubmittedReplayMutations({
      existingDays: [
        createExistingDay(
          "day-1",
          "2024-12-02",
          mergeBreakdowns(
            createClientBreakdown("claude", 100, 1),
            createClientBreakdown("cursor", 40, 0.4)
          ),
          Date.parse("2024-12-02T09:00:00.000Z")
        ),
      ],
      incomingDays: [
        createIncomingDay(
          "2024-12-02",
          createClientBreakdown("claude", 30, 0.3),
          Date.parse("2024-12-02T12:00:00.000Z")
        ),
      ],
      submittedClients: new Set(["claude"]),
      replayWindow: { start: "2024-12-02", end: "2024-12-02" },
      submissionId: "submission-1",
    });

    expect(result.inserts).toEqual([]);
    expect(result.deletes).toEqual([]);
    expect(result.updates).toHaveLength(1);
    expect(result.updates[0]).toMatchObject({
      id: "day-1",
      date: "2024-12-02",
      tokens: 70,
      cost: "0.7000",
      timestampMs: Date.parse("2024-12-02T09:00:00.000Z"),
    });
    expect(result.updates[0].sourceBreakdown).toEqual(
      mergeBreakdowns(
        createClientBreakdown("claude", 30, 0.3),
        createClientBreakdown("cursor", 40, 0.4)
      )
    );
  });

  it("removes an intentionally omitted in-scope client from an overlapping day only", () => {
    const result = planSubmittedReplayMutations({
      existingDays: [
        createExistingDay(
          "day-1",
          "2024-12-02",
          mergeBreakdowns(
            createClientBreakdown("claude", 100, 1),
            createClientBreakdown("cursor", 40, 0.4)
          )
        ),
      ],
      incomingDays: [
        createIncomingDay(
          "2024-12-02",
          createClientBreakdown("opencode", 20, 0.2)
        ),
      ],
      submittedClients: new Set(["claude", "opencode"]),
      replayWindow: { start: "2024-12-02", end: "2024-12-02" },
      submissionId: "submission-1",
    });

    expect(result.deletes).toEqual([]);
    expect(result.updates).toHaveLength(1);
    expect(result.updates[0].tokens).toBe(60);
    expect(result.updates[0].sourceBreakdown).toEqual(
      mergeBreakdowns(
        createClientBreakdown("cursor", 40, 0.4),
        createClientBreakdown("opencode", 20, 0.2)
      )
    );
  });

  it("removes omitted in-scope days from the replayed window while preserving untouched dates outside it", () => {
    const result = planSubmittedReplayMutations({
      existingDays: [
        createExistingDay(
          "day-1",
          "2024-12-01",
          createClientBreakdown("claude", 50, 0.5),
          Date.parse("2024-12-01T08:00:00.000Z")
        ),
        createExistingDay(
          "day-2",
          "2024-12-02",
          mergeBreakdowns(
            createClientBreakdown("claude", 60, 0.6),
            createClientBreakdown("cursor", 10, 0.1)
          ),
          Date.parse("2024-12-02T08:00:00.000Z")
        ),
        createExistingDay(
          "day-3",
          "2024-12-03",
          createClientBreakdown("cursor", 70, 0.7),
          Date.parse("2024-12-03T08:00:00.000Z")
        ),
      ],
      incomingDays: [
        createIncomingDay(
          "2024-12-02",
          createClientBreakdown("claude", 30, 0.3),
          Date.parse("2024-12-02T11:00:00.000Z")
        ),
      ],
      submittedClients: new Set(["claude"]),
      replayWindow: { start: "2024-12-01", end: "2024-12-02" },
      submissionId: "submission-1",
    });

    expect(result.inserts).toEqual([]);
    expect(result.deletes).toEqual([{ id: "day-1", date: "2024-12-01" }]);
    expect(result.updates).toHaveLength(1);
    expect(result.updates[0]).toMatchObject({
      id: "day-2",
      date: "2024-12-02",
      tokens: 40,
      cost: "0.4000",
    });
    expect(result.updates[0].sourceBreakdown).toEqual(
      mergeBreakdowns(
        createClientBreakdown("claude", 30, 0.3),
        createClientBreakdown("cursor", 10, 0.1)
      )
    );
    expect(result.updates.some((update) => update.date === "2024-12-03")).toBe(false);
    expect(result.deletes.some((deleted) => deleted.date === "2024-12-03")).toBe(false);
  });

  it("keeps replay totals idempotent when the same logical history is submitted again", () => {
    const existingBreakdown = createClientBreakdown("claude", 120, 1.2);
    const result = planSubmittedReplayMutations({
      existingDays: [
        createExistingDay(
          "day-1",
          "2024-12-04",
          existingBreakdown,
          Date.parse("2024-12-04T10:00:00.000Z")
        ),
      ],
      incomingDays: [
        createIncomingDay(
          "2024-12-04",
          createClientBreakdown("claude", 120, 1.2),
          Date.parse("2024-12-04T10:00:00.000Z")
        ),
      ],
      submittedClients: new Set(["claude"]),
      replayWindow: { start: "2024-12-04", end: "2024-12-04" },
      submissionId: "submission-1",
    });

    expect(result.inserts).toEqual([]);
    expect(result.deletes).toEqual([]);
    expect(result.updates).toHaveLength(1);
    expect(result.updates[0]).toMatchObject({
      id: "day-1",
      tokens: 120,
      cost: "1.2000",
      timestampMs: Date.parse("2024-12-04T10:00:00.000Z"),
      sourceBreakdown: existingBreakdown,
    });
  });
});
