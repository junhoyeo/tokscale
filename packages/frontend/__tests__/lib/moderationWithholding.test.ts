import { describe, expect, it } from "vitest";
import {
  evaluateSubmissionForWithholding,
  getWithholdingUsageTotals,
  hasPerInferenceMessageCount,
  NON_PER_INFERENCE_MESSAGE_CLIENTS,
  WITHHOLDING_MAX_DAILY_OUTPUT_TOKENS as OUTPUT_LIMIT,
  WITHHOLDING_MAX_DAILY_TOTAL_TOKENS as DAILY_LIMIT,
  WITHHOLDING_MAX_TOTAL_TOKENS as TOTAL_LIMIT,
  WITHHOLDING_MAX_TOKENS_PER_MESSAGE as PER_MESSAGE_LIMIT,
} from "@/lib/moderation/withholding";
import { foldIncomingClientContributions } from "@/app/api/submit/route";
import { recalculateDayTotals } from "@/lib/db/helpers";

describe("ledger volume withholding", () => {
  it.each([
    { name: "daily output at the limit", totalTokens: OUTPUT_LIMIT, maxDailyTokens: OUTPUT_LIMIT, maxDailyOutputTokens: OUTPUT_LIMIT, maxTokensPerMessage: 100, trigger: undefined },
    { name: "daily output above the limit", totalTokens: OUTPUT_LIMIT + 1, maxDailyTokens: OUTPUT_LIMIT + 1, maxDailyOutputTokens: OUTPUT_LIMIT + 1, maxTokensPerMessage: 100, trigger: "daily_output_limit" },
    { name: "daily cached volume at the limit", totalTokens: DAILY_LIMIT, maxDailyTokens: DAILY_LIMIT, maxDailyOutputTokens: 100, maxTokensPerMessage: 100, trigger: undefined },
    { name: "daily cached volume above the limit", totalTokens: DAILY_LIMIT + 1, maxDailyTokens: DAILY_LIMIT + 1, maxDailyOutputTokens: 100, maxTokensPerMessage: 100, trigger: "daily_token_limit" },
    { name: "per-message volume at the limit", totalTokens: 10_000_000, maxDailyTokens: 10_000_000, maxDailyOutputTokens: 100, maxTokensPerMessage: PER_MESSAGE_LIMIT, trigger: undefined },
    { name: "per-message volume above the limit", totalTokens: 10_000_000, maxDailyTokens: 10_000_000, maxDailyOutputTokens: 100, maxTokensPerMessage: PER_MESSAGE_LIMIT + 1, trigger: "per_message_token_limit" },
    { name: "single message with 1+ billion tokens", totalTokens: 1_200_000_000, maxDailyTokens: 1_200_000_000, maxDailyOutputTokens: 100, maxTokensPerMessage: 1_200_000_000, trigger: "per_message_token_limit" },
    { name: "lifetime volume below the limit", totalTokens: TOTAL_LIMIT - 1, maxDailyTokens: DAILY_LIMIT, maxDailyOutputTokens: 100, maxTokensPerMessage: 100, trigger: undefined },
    { name: "lifetime volume at the limit", totalTokens: TOTAL_LIMIT, maxDailyTokens: DAILY_LIMIT, maxDailyOutputTokens: 100, maxTokensPerMessage: 100, trigger: "total_token_limit" },
  ])("$name", ({ totalTokens, maxDailyTokens, maxDailyOutputTokens, maxTokensPerMessage, trigger }) => {
    const result = evaluateSubmissionForWithholding({ totalTokens, maxDailyTokens, maxDailyOutputTokens, maxTokensPerMessage });
    expect(result.shouldWithhold).toBe(trigger !== undefined);
    expect(result.trigger).toBe(trigger);
    if (trigger === "per_message_token_limit") {
      expect(result.reason).toContain("Per-message volume");
      expect(result.reason).toContain(PER_MESSAGE_LIMIT.toLocaleString("en-US"));
    }
  });

  it("extracts and flags impossible per-message volume from client contributions in sourceBreakdown", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 1_000_000_000,
        outputTokens: 500,
        sourceBreakdown: {
          claude: {
            tokens: 1_000_000_000,
            messages: 1,
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(1_000_000_000);
    const result = evaluateSubmissionForWithholding(usage);
    expect(result.shouldWithhold).toBe(true);
    expect(result.trigger).toBe("per_message_token_limit");
    expect(result.reason).toBe(
      `Per-message volume 1,000,000,000 tokens exceeds the leaderboard limit of ${PER_MESSAGE_LIMIT.toLocaleString("en-US")} tokens`
    );
  });

  it("extracts and flags impossible per-message volume from nested model breakdowns", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 1_500_000_000,
        outputTokens: 500,
        sourceBreakdown: {
          claude: {
            tokens: 0,
            messages: 0,
            models: {
              "spoof-model": {
                tokens: 1_500_000_000,
                messages: 1,
              },
            },
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(1_500_000_000);
    expect(evaluateSubmissionForWithholding(usage).trigger).toBe("per_message_token_limit");
  });

  it("prevents dilution across multiple devices on the same date", () => {
    // Device 1 submits 1.5B tokens in 1 message; Device 2 submits 1,000 normal messages with 10k tokens
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 1_500_000_000,
        outputTokens: 500,
        sourceBreakdown: {
          claude: { tokens: 1_500_000_000, messages: 1 },
        },
      },
      {
        date: "2026-04-30",
        tokens: 10_000,
        outputTokens: 5_000,
        sourceBreakdown: {
          codex: { tokens: 10_000, messages: 1_000 },
        },
      },
    ]);
    // Device 1's per-message volume is not masked by Device 2's high message count
    expect(usage.maxTokensPerMessage).toBe(1_500_000_000);
    expect(evaluateSubmissionForWithholding(usage).trigger).toBe("per_message_token_limit");
  });

  it("keeps a 200M-token count-neutral Droid fragment visible alone", () => {
    const breakdown = foldIncomingClientContributions([
      {
        client: "droid",
        modelId: "claude-opus-4-5",
        tokens: { input: 200_000_000, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 10,
        messages: 0,
      },
    ]);
    const dayTotals = recalculateDayTotals(breakdown);
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: dayTotals.tokens,
        outputTokens: dayTotals.outputTokens,
        sourceBreakdown: breakdown,
      },
    ]);
    expect(usage.maxTokensPerMessage).toBeUndefined();
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("does not mix a 200M-token count-neutral fragment into an ordinary 100-token/1-message Codex contribution on the same day", () => {
    const breakdown = foldIncomingClientContributions([
      {
        client: "droid",
        modelId: "claude-opus-4-5",
        tokens: { input: 200_000_000, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 10,
        messages: 0,
      },
      {
        client: "codex",
        modelId: "gpt-5",
        tokens: { input: 80, output: 20, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 0.01,
        messages: 1,
      },
    ]);
    const dayTotals = recalculateDayTotals(breakdown);
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: dayTotals.tokens,
        outputTokens: dayTotals.outputTokens,
        sourceBreakdown: breakdown,
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(100);
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("does not mix a 200M-token count-neutral fragment on one device with a 100-token/1-message Codex contribution on another device on the same date", () => {
    const device1Breakdown = foldIncomingClientContributions([
      {
        client: "droid",
        modelId: "claude-opus-4-5",
        tokens: { input: 200_000_000, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 10,
        messages: 0,
      },
    ]);
    const device1Totals = recalculateDayTotals(device1Breakdown);

    const device2Breakdown = foldIncomingClientContributions([
      {
        client: "codex",
        modelId: "gpt-5",
        tokens: { input: 80, output: 20, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 0.01,
        messages: 1,
      },
    ]);
    const device2Totals = recalculateDayTotals(device2Breakdown);

    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: device1Totals.tokens,
        outputTokens: device1Totals.outputTokens,
        sourceBreakdown: device1Breakdown,
      },
      {
        date: "2026-04-30",
        tokens: device2Totals.tokens,
        outputTokens: device2Totals.outputTokens,
        sourceBreakdown: device2Breakdown,
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(100);
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("does not treat a 200M-token Droid cumulative session fallback record (messages = 1) as a single inference", () => {
    const breakdown = foldIncomingClientContributions([
      {
        client: "droid",
        modelId: "claude-opus-4-5",
        tokens: { input: 199_999_000, output: 1_000, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 10,
        messages: 1,
      },
    ]);
    const dayTotals = recalculateDayTotals(breakdown);
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: dayTotals.tokens,
        outputTokens: dayTotals.outputTokens,
        sourceBreakdown: breakdown,
      },
    ]);
    expect(usage.maxTokensPerMessage).toBeUndefined();
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("does not mix a count-neutral model fragment into a sibling model's message count under the same per-inference client", () => {
    // Claude Code advisor models carry messages = 0 alongside the main turn's model with messages = 1
    const breakdown = foldIncomingClientContributions([
      {
        client: "claude",
        modelId: "claude-opus-4",
        tokens: { input: 200_000_000, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 10,
        messages: 0,
      },
      {
        client: "claude",
        modelId: "claude-sonnet-4",
        tokens: { input: 80, output: 20, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: 0.01,
        messages: 1,
      },
    ]);
    const dayTotals = recalculateDayTotals(breakdown);
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: dayTotals.tokens,
        outputTokens: dayTotals.outputTokens,
        sourceBreakdown: breakdown,
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(100);
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it.each(NON_PER_INFERENCE_MESSAGE_CLIENTS)(
    "skips non-per-inference client %s for per-message enforcement while preserving daily limits",
    (clientName) => {
      expect(hasPerInferenceMessageCount(clientName)).toBe(false);
      expect(hasPerInferenceMessageCount(`  ${clientName.toUpperCase()}  `)).toBe(false);
      const usage = getWithholdingUsageTotals([
        {
          date: "2026-04-30",
          tokens: 200_000_000,
          outputTokens: 1_000,
          sourceBreakdown: {
            [clientName]: {
              tokens: 200_000_000,
              messages: 1,
              models: {
                "model-a": { tokens: 200_000_000, messages: 1 },
              },
            },
          },
        },
      ]);
      expect(usage.maxTokensPerMessage).toBeUndefined();
      expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
    }
  );

  it("handles zero messages without dividing by zero", () => {
    const usage = getWithholdingUsageTotals([
      { date: "2026-04-30", tokens: 1_000_000_000, outputTokens: 500 },
      {
        date: "2026-04-30",
        tokens: 500_000_000,
        outputTokens: 500,
        sourceBreakdown: { claude: { tokens: 500_000_000, messages: 0 } },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBeUndefined();
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("handles negative, null, undefined, or string debris messages safely", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 1_000_000_000,
        outputTokens: 500,
        sourceBreakdown: {
          claude: { tokens: 1_000_000_000, messages: -5 },
          codex: { tokens: 1_000_000_000, messages: null },
          gemini: { tokens: 1_000_000_000, messages: "not-a-number" as unknown as number },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBeUndefined();
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("handles legitimate heavy usage users without false positives", () => {
    // 50 billion tokens across 20 million messages (avg 2,500 tokens/message)
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 50_000_000_000,
        outputTokens: 5_000_000_000,
        sourceBreakdown: {
          claude: {
            tokens: 50_000_000_000,
            messages: 20_000_000,
            models: {
              "claude-sonnet-4": { tokens: 50_000_000_000, messages: 20_000_000 },
            },
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(2500);
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("handles object token breakdowns in sourceBreakdown", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 1_000_000_000,
        outputTokens: 0,
        sourceBreakdown: {
          claude: {
            tokens: { input: 1_000_000_000, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
            messages: 1,
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(1_000_000_000);
    expect(evaluateSubmissionForWithholding(usage).trigger).toBe("per_message_token_limit");
  });

  it("does not flag Cursor legacy carve-outs with zero tokens", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 0,
        outputTokens: 0,
        sourceBreakdown: {
          cursor: {
            tokens: 0,
            messages: 10,
            cost: 5.0,
            models: { "premium-tool-call": { tokens: 0, messages: 10, cost: 5.0 } },
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBeUndefined();
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("extracts tokens when placed directly on client breakdown object without a tokens field", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 1_000_010_000,
        outputTokens: 500,
        sourceBreakdown: {
          claude: {
            input: 1_000_000_000,
            output: 0,
            messages: 1,
          },
          codex: {
            tokens: 10_000,
            messages: 1_000,
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(1_000_000_000);
    expect(evaluateSubmissionForWithholding(usage).trigger).toBe("per_message_token_limit");
  });

  it("handles scientific notation strings in tokens", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: "1e9",
        outputTokens: 500,
        sourceBreakdown: {
          claude: {
            tokens: "1e9",
            messages: "1",
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(1_000_000_000);
    expect(evaluateSubmissionForWithholding(usage).trigger).toBe("per_message_token_limit");
  });

  it("evaluates nested model breakdown when client messages is 0", () => {
    const usage = getWithholdingUsageTotals([
      {
        date: "2026-04-30",
        tokens: 1_200_000_000,
        outputTokens: 500,
        sourceBreakdown: {
          claude: {
            tokens: 1_200_000_000,
            messages: 0,
            models: {
              "claude-3-5": { tokens: 1_200_000_000, messages: 1 },
            },
          },
        },
      },
    ]);
    expect(usage.maxTokensPerMessage).toBe(1_200_000_000);
    expect(evaluateSubmissionForWithholding(usage).trigger).toBe("per_message_token_limit");
  });

  it("combines the same calendar day across devices", () => {
    const usage = getWithholdingUsageTotals([
      { date: "2026-04-30", tokens: 6_000_000_000, outputTokens: 6_000_000_000 },
      { date: "2026-04-30", tokens: "6000000000", outputTokens: 6_000_000_000n },
    ]);
    expect(usage).toEqual({ totalTokens: 12_000_000_000, maxDailyTokens: 12_000_000_000, maxDailyOutputTokens: 12_000_000_000 });
    expect(evaluateSubmissionForWithholding(usage).trigger).toBe("daily_output_limit");
  });

  it("does not treat separate days as one implied day", () => {
    const usage = getWithholdingUsageTotals([
      { date: "2026-04-29", tokens: DAILY_LIMIT, outputTokens: OUTPUT_LIMIT },
      { date: "2026-04-30", tokens: DAILY_LIMIT, outputTokens: OUTPUT_LIMIT },
      { date: "2026-05-01", tokens: DAILY_LIMIT, outputTokens: OUTPUT_LIMIT },
    ]);
    expect(evaluateSubmissionForWithholding(usage)).toEqual({ shouldWithhold: false, reason: "" });
  });

  it("does not inflate ledger scalars or per-message ratios from retained over-nested model maps", () => {
    const rows = [{
      date: "2026-04-30", tokens: 1_600_000, outputTokens: 600_000,
      sourceBreakdown: {
        claude: {
          tokens: 1_600_000,
          messages: 100,
          models: {
            old: { tokens: TOTAL_LIMIT, output: OUTPUT_LIMIT + 1, messages: 1 },
          },
        },
      },
    }];
    expect(getWithholdingUsageTotals(rows)).toEqual({
      totalTokens: 1_600_000,
      maxDailyTokens: 1_600_000,
      maxDailyOutputTokens: 600_000,
      maxTokensPerMessage: 16_000,
    });

    const overNestedZeroMsgModel = [{
      date: "2026-04-30", tokens: 1_500_000_000, outputTokens: 0,
      sourceBreakdown: {
        claude: {
          tokens: 1_500_000_000,
          messages: 1,
          models: {
            old: { tokens: 2_000_000_000, messages: 0 },
          },
        },
      },
    }];
    expect(getWithholdingUsageTotals(overNestedZeroMsgModel).maxTokensPerMessage).toBe(1_500_000_000);
  });

  it("handles an empty ledger without inferring usage from request summaries", () => {
    expect(evaluateSubmissionForWithholding(getWithholdingUsageTotals([]))).toEqual({
      shouldWithhold: false, reason: "",
    });
  });

  it("recognizes totals beyond the JavaScript safe-integer range", () => {
    expect(evaluateSubmissionForWithholding({
      totalTokens: "9223372036854775807", maxDailyTokens: 0, maxDailyOutputTokens: 0,
    }).trigger).toBe("total_token_limit");
  });
});
