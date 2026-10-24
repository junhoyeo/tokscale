/**
 * Rankability limits over the daily ledger, not proof of fabrication.
 * Model maps can retain historical entries for cost floors, and active time
 * can cover only part of the submitted history. Neither is a volume source.
 */
export const WITHHOLDING_MAX_DAILY_OUTPUT_TOKENS = 10_000_000_000;
export const WITHHOLDING_MAX_DAILY_TOTAL_TOKENS = 500_000_000_000;
export const WITHHOLDING_MAX_TOTAL_TOKENS = 1_000_000_000_000_000;
/**
 * Hard limit for physically impossible single-message token volumes.
 * Real-world LLM context windows ceiling around 1M-2M tokens (e.g. Gemini 1.5 Pro).
 * A single message reporting 100+ million tokens (or 1+ billion tokens) is
 * undeniably fabricated or corrupted.
 */
export const WITHHOLDING_MAX_TOKENS_PER_MESSAGE = 100_000_000;

export type WithholdingTrigger =
  | "daily_output_limit"
  | "daily_token_limit"
  | "total_token_limit"
  | "per_message_token_limit";

export interface WithholdingResult {
  shouldWithhold: boolean;
  reason: string;
  trigger?: WithholdingTrigger;
}

type TokenCount = number | string | bigint | null | undefined;

export interface WithholdingUsageTotals {
  totalTokens: TokenCount;
  maxDailyTokens: TokenCount;
  maxDailyOutputTokens: TokenCount;
  maxTokensPerMessage?: TokenCount;
}

export interface WithholdingDailyRow {
  date: string;
  tokens: TokenCount;
  outputTokens: TokenCount;
  messages?: TokenCount;
  sourceBreakdown?: unknown;
}

function toCount(value: TokenCount): number {
  const count = Number(value ?? 0);
  return Number.isFinite(count) ? Math.max(0, count) : 0;
}

function sumTokenBreakdown(obj: unknown): number {
  if (!obj || typeof obj !== "object") return 0;
  const rec = obj as Record<string, unknown>;
  return (
    toCount(rec.input as TokenCount) +
    toCount(rec.output as TokenCount) +
    toCount(rec.cacheRead as TokenCount) +
    toCount(rec.cacheWrite as TokenCount) +
    toCount(rec.reasoning as TokenCount)
  );
}

function toTokenCount(value: unknown): number {
  if (value && typeof value === "object") {
    const rec = value as Record<string, unknown>;
    const directCount = toCount(rec.tokens as TokenCount);
    const nestedBreakdown = sumTokenBreakdown(rec.tokens);
    const selfBreakdown = sumTokenBreakdown(rec);
    return Math.max(directCount, nestedBreakdown, selfBreakdown);
  }
  return toCount(value as TokenCount);
}

/** Group calendar dates across devices using daily_breakdown scalars and evaluate per-message volumes. */
export function getWithholdingUsageTotals(
  rows: readonly WithholdingDailyRow[]
): WithholdingUsageTotals {
  const byDate = new Map<string, { tokens: number; output: number; messages: number }>();
  let totalTokens = 0;
  let maxDailyTokens = 0;
  let maxDailyOutputTokens = 0;
  let maxTokensPerMessage = 0;

  for (const row of rows) {
    const tokens = toCount(row.tokens);
    const output = toCount(row.outputTokens);
    let rowMessages = toCount(row.messages);

    if (row.sourceBreakdown && typeof row.sourceBreakdown === "object") {
      let breakdownMessagesSum = 0;
      for (const clientValue of Object.values(row.sourceBreakdown as Record<string, unknown>)) {
        if (!clientValue || typeof clientValue !== "object") continue;
        const clientObj = clientValue as Record<string, unknown>;
        const clientTokens = Math.max(toTokenCount(clientObj), toTokenCount(clientObj.tokens));
        let clientMessages = toCount(clientObj.messages as TokenCount);
        let modelMessagesSum = 0;

        if (clientObj.models && typeof clientObj.models === "object") {
          for (const modelValue of Object.values(clientObj.models as Record<string, unknown>)) {
            if (!modelValue || typeof modelValue !== "object") continue;
            const modelObj = modelValue as Record<string, unknown>;
            const modelTokens = Math.max(toTokenCount(modelObj), toTokenCount(modelObj.tokens));
            const modelMessages = toCount(modelObj.messages as TokenCount);
            if (modelMessages > 0) {
              modelMessagesSum += modelMessages;
              if (modelTokens > 0) {
                maxTokensPerMessage = Math.max(maxTokensPerMessage, modelTokens / modelMessages);
              }
            }
          }
        }

        if (clientMessages === 0 && modelMessagesSum > 0) {
          clientMessages = modelMessagesSum;
        }

        if (clientMessages > 0) {
          breakdownMessagesSum += clientMessages;
          if (clientTokens > 0) {
            maxTokensPerMessage = Math.max(maxTokensPerMessage, clientTokens / clientMessages);
          }
        }
      }
      if (rowMessages === 0 && breakdownMessagesSum > 0) {
        rowMessages = breakdownMessagesSum;
      }
    }

    if (rowMessages > 0 && tokens > 0) {
      maxTokensPerMessage = Math.max(maxTokensPerMessage, tokens / rowMessages);
    }

    const day = byDate.get(row.date) ?? { tokens: 0, output: 0, messages: 0 };
    day.tokens += tokens;
    day.output += output;
    day.messages += rowMessages;
    byDate.set(row.date, day);

    totalTokens += tokens;
    maxDailyTokens = Math.max(maxDailyTokens, day.tokens);
    maxDailyOutputTokens = Math.max(maxDailyOutputTokens, day.output);
  }

  for (const day of byDate.values()) {
    if (day.messages > 0 && day.tokens > 0) {
      maxTokensPerMessage = Math.max(maxTokensPerMessage, day.tokens / day.messages);
    }
  }

  return {
    totalTokens,
    maxDailyTokens,
    maxDailyOutputTokens,
    ...(maxTokensPerMessage > 0 ? { maxTokensPerMessage } : {}),
  };
}

/** The caller supplies final ledger totals, never raw request summaries. */
export function evaluateSubmissionForWithholding(
  usage: WithholdingUsageTotals
): WithholdingResult {
  const output = toCount(usage.maxDailyOutputTokens);
  if (output > WITHHOLDING_MAX_DAILY_OUTPUT_TOKENS) {
    return {
      shouldWithhold: true,
      trigger: "daily_output_limit",
      reason: `Daily output ${output.toLocaleString("en-US")} exceeds the leaderboard limit of ${WITHHOLDING_MAX_DAILY_OUTPUT_TOKENS.toLocaleString("en-US")} tokens`,
    };
  }
  const daily = toCount(usage.maxDailyTokens);
  if (daily > WITHHOLDING_MAX_DAILY_TOTAL_TOKENS) {
    return {
      shouldWithhold: true,
      trigger: "daily_token_limit",
      reason: `Daily volume ${daily.toLocaleString("en-US")} exceeds the leaderboard limit of ${WITHHOLDING_MAX_DAILY_TOTAL_TOKENS.toLocaleString("en-US")} tokens`,
    };
  }
  const perMessage = toCount(usage.maxTokensPerMessage);
  if (perMessage > WITHHOLDING_MAX_TOKENS_PER_MESSAGE) {
    return {
      shouldWithhold: true,
      trigger: "per_message_token_limit",
      reason: `Per-message volume ${Math.round(perMessage).toLocaleString("en-US")} tokens exceeds the leaderboard limit of ${WITHHOLDING_MAX_TOKENS_PER_MESSAGE.toLocaleString("en-US")} tokens`,
    };
  }
  const total = toCount(usage.totalTokens);
  if (total >= WITHHOLDING_MAX_TOTAL_TOKENS) {
    return {
      shouldWithhold: true,
      trigger: "total_token_limit",
      reason: `Lifetime volume ${total.toLocaleString("en-US")} meets the leaderboard limit of ${WITHHOLDING_MAX_TOTAL_TOKENS.toLocaleString("en-US")} tokens`,
    };
  }
  return { shouldWithhold: false, reason: "" };
}

export type SubmissionWithholdingMode = "off" | "observe" | "enforce";

/** Explicit opt-in is required to change account state; unset means observe. */
export function getSubmissionWithholdingMode(
  value: string | undefined
): SubmissionWithholdingMode {
  if (value?.trim() === "0") return "off";
  if (value?.trim() === "1") return "enforce";
  return "observe";
}
