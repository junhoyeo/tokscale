/**
 * Rankability limits over the daily ledger, not proof of fabrication.
 * Model maps can retain historical entries for cost floors, and active time
 * can cover only part of the submitted history. Neither is a volume source.
 */
export const WITHHOLDING_MAX_DAILY_OUTPUT_TOKENS = 10_000_000_000;
export const WITHHOLDING_MAX_DAILY_TOTAL_TOKENS = 500_000_000_000;
export const WITHHOLDING_MAX_TOTAL_TOKENS = 1_000_000_000_000_000;
/**
 * Per-message leaderboard limit for clients whose exported token and message
 * buckets retain individual LLM inference counts through aggregation. Counts
 * remain self-reported; this limit does not establish authenticity.
 */
export const WITHHOLDING_MAX_TOKENS_PER_MESSAGE = 100_000_000;

/**
 * Clients whose exported tokens and `messages` are not guaranteed to describe
 * the same set of individual inferences — e.g. whole-session fallback records
 * with `messages = 1` (Droid when transcript is missing or exceeds
 * `MAX_TRANSCRIPT_BYTES`, Zed `cumulative_token_usage` fallback, Vibe session
 * files, WorkBuddy `session_usage` rows, Kiro session/execution fallbacks),
 * agent-execution or session-level model summaries (`codebuddy`, `fx`,
 * `reasonix`, `grok`, `crush`, `warp`), or session reply counts placed on a
 * single fragment while sibling fragments carry `messages = 0` (`droid`,
 * `copilot`, `hermes`). Codex can emit a retained cumulative total as one
 * message when only total usage is available, and Trae exports session totals.
 * Claude and cc-mirror variants merge count-neutral advisor/tool-result tokens
 * into model buckets that can also contain counted assistant messages. Once
 * aggregated, those supplementary tokens cannot be subtracted reliably.
 *
 * Daily and lifetime token ceilings still apply to these clients, but dividing
 * their cumulative token bucket by `messages` does not measure a single LLM
 * context window.
 */
export const NON_PER_INFERENCE_MESSAGE_CLIENTS = [
  "claude",
  "codebuddy",
  "codex",
  "copilot",
  "crush",
  "devin-desktop",
  "droid",
  "fx",
  "goose",
  "grok",
  "hermes",
  "kiro",
  "mux",
  "reasonix",
  "trae",
  "vibe",
  "warp",
  "workbuddy",
  "zed",
] as const;

// Shared by JavaScript enforcement and PostgreSQL review signals. cc-mirror
// variants use the Claude parser and need the same exclusion as Claude itself.
export const NON_PER_INFERENCE_CLIENT_REGEX = `^(${NON_PER_INFERENCE_MESSAGE_CLIENTS.join("|")})$|^cc-mirror/`;

const NON_PER_INFERENCE_MESSAGE_CLIENT_PATTERN = new RegExp(NON_PER_INFERENCE_CLIENT_REGEX);

export function hasPerInferenceMessageCount(clientName: string): boolean {
  const normalized = clientName.trim().toLowerCase();
  return normalized.length > 0 && !NON_PER_INFERENCE_MESSAGE_CLIENT_PATTERN.test(normalized);
}

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

/**
 * Group calendar dates across devices using daily_breakdown scalars and
 * evaluate per-message volumes over matched, per-inference client/model entries.
 */
export function getWithholdingUsageTotals(
  rows: readonly WithholdingDailyRow[]
): WithholdingUsageTotals {
  const byDate = new Map<string, { tokens: number; output: number }>();
  let totalTokens = 0;
  let maxDailyTokens = 0;
  let maxDailyOutputTokens = 0;
  let maxTokensPerMessage = 0;

  for (const row of rows) {
    const tokens = toCount(row.tokens);
    const output = toCount(row.outputTokens);

    if (
      row.sourceBreakdown &&
      typeof row.sourceBreakdown === "object" &&
      !Array.isArray(row.sourceBreakdown)
    ) {
      for (const [clientKey, clientValue] of Object.entries(
        row.sourceBreakdown as Record<string, unknown>
      )) {
        if (!hasPerInferenceMessageCount(clientKey)) continue;
        if (!clientValue || typeof clientValue !== "object" || Array.isArray(clientValue)) {
          continue;
        }
        const clientObj = clientValue as Record<string, unknown>;
        const clientTokens = Math.max(
          toTokenCount(clientObj),
          toTokenCount(clientObj.tokens)
        );
        const clientMessages = toCount(clientObj.messages as TokenCount);

        let modelTokensSum = 0;
        let zeroMessageModelTokens = 0;
        let maxModelRatio = 0;

        if (
          clientObj.models &&
          typeof clientObj.models === "object" &&
          !Array.isArray(clientObj.models)
        ) {
          for (const modelValue of Object.values(
            clientObj.models as Record<string, unknown>
          )) {
            if (!modelValue || typeof modelValue !== "object" || Array.isArray(modelValue)) {
              continue;
            }
            const modelObj = modelValue as Record<string, unknown>;
            const modelTokens = Math.max(
              toTokenCount(modelObj),
              toTokenCount(modelObj.tokens)
            );
            const modelMessages = toCount(modelObj.messages as TokenCount);
            modelTokensSum += modelTokens;
            if (modelMessages > 0) {
              if (modelTokens > 0) {
                maxModelRatio = Math.max(maxModelRatio, modelTokens / modelMessages);
              }
            } else {
              zeroMessageModelTokens += modelTokens;
            }
          }
        }

        const isOverNested = clientTokens > 0 && modelTokensSum > clientTokens;
        if (!isOverNested && maxModelRatio > 0) {
          maxTokensPerMessage = Math.max(maxTokensPerMessage, maxModelRatio);
        }

        if (clientMessages > 0 && clientTokens > 0) {
          const countedClientTokens = isOverNested
            ? clientTokens
            : Math.max(0, clientTokens - zeroMessageModelTokens);
          if (countedClientTokens > 0) {
            maxTokensPerMessage = Math.max(
              maxTokensPerMessage,
              countedClientTokens / clientMessages
            );
          }
        }
      }
    }

    const day = byDate.get(row.date) ?? { tokens: 0, output: 0 };
    day.tokens += tokens;
    day.output += output;
    byDate.set(row.date, day);

    totalTokens += tokens;
    maxDailyTokens = Math.max(maxDailyTokens, day.tokens);
    maxDailyOutputTokens = Math.max(maxDailyOutputTokens, day.output);
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
