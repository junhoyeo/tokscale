/**
 * Model identity for the explore surfaces.
 *
 * `canonicalModelId` is a line-for-line port of `normalize_syntactic` /
 * `canonical_model_id` in crates/tokscale-core/src/lib.rs. It is structural
 * only (no alias folding), so the server groups a model id the same way the
 * CLI that submitted it does. Keep the two in sync: the parity cases in
 * __tests__/lib/explore/modelKey.test.ts mirror the Rust tests.
 *
 * `modelKey` adds two explore-only steps on top of the canonical id so it can
 * be used as a URL segment and as one row per model:
 * - drop a routing prefix ("openai/gpt-5.5", "accounts/fireworks/routers/x"),
 *   keeping the last path segment;
 * - replace every character outside [a-z0-9._-] with "-".
 */

const REASONING_TIERS = new Set(["minimal", "low", "medium", "high", "xhigh", "auto", "none"]);

/** Port of `strip_parenthesized_reasoning_tier`. Expects a lowercased id. */
export function stripParenthesizedReasoningTier(modelId: string): string | null {
  if (!modelId.endsWith(")")) return null;
  const withoutClosingParen = modelId.slice(0, -1);
  const open = withoutClosingParen.lastIndexOf("(");
  if (open < 0) return null;
  const baseModel = withoutClosingParen.slice(0, open);
  const tier = withoutClosingParen.slice(open + 1);
  if (baseModel.length === 0 || baseModel.trim() !== baseModel) return null;
  if (!REASONING_TIERS.has(tier)) return null;
  return baseModel;
}

const ASCII_DIGIT = /^[0-9]$/;

function isAsciiDigit(ch: string | undefined): boolean {
  return ch !== undefined && ASCII_DIGIT.test(ch);
}

/** Port of `normalize_anthropic_prefixed_claude_model`. */
function normalizeAnthropicPrefixedClaudeModel(modelId: string): string | null {
  const prefix = "anthropic/claude-";
  if (!modelId.startsWith(prefix)) return null;
  const parts = modelId.slice(prefix.length).split("-");
  if (parts.length !== 3) return null;
  const [major, minor, family] = parts;
  if (family !== "opus" && family !== "sonnet" && family !== "haiku") return null;
  return `claude-${family}-${major}-${minor}`;
}

/**
 * Port of `normalize_syntactic` (= `canonical_model_id`): lowercase, strip a
 * `(reasoning-tier)` suffix, strip a trailing `-YYYYMMDD` date, rewrite `.` to
 * `-` between digits inside claude ids, and fold `anthropic/claude-M-m-family`.
 */
export function canonicalModelId(modelId: string): string {
  let name = modelId.toLowerCase();

  const withoutTier = stripParenthesizedReasoningTier(name);
  if (withoutTier !== null) name = withoutTier;

  const dash = name.lastIndexOf("-");
  if (dash >= 0) {
    const baseModel = name.slice(0, dash);
    const potentialDate = name.slice(dash + 1);
    if (baseModel.length > 0 && /^[0-9]{8}$/.test(potentialDate)) {
      name = baseModel;
    }
  }

  if (name.includes("claude")) {
    // Iterate by code point, like Rust's `chars()`.
    const chars = Array.from(name);
    let result = "";
    for (let i = 0; i < chars.length; i++) {
      if (chars[i] === "." && i > 0 && i < chars.length - 1 && isAsciiDigit(chars[i - 1]) && isAsciiDigit(chars[i + 1])) {
        result += "-";
      } else {
        result += chars[i];
      }
    }
    name = result;
  }

  const folded = normalizeAnthropicPrefixedClaudeModel(name);
  if (folded !== null) name = folded;

  return name;
}

const MAX_KEY_LENGTH = 96;

/** URL-safe explore key for a raw submitted model id. */
export function modelKey(rawModelId: string): string {
  let name = canonicalModelId(rawModelId.trim());
  const slash = name.lastIndexOf("/");
  if (slash >= 0) name = name.slice(slash + 1);
  name = name
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/-{2,}/g, "-")
    .replace(/^[-.]+|[-.]+$/g, "");
  if (name.length > MAX_KEY_LENGTH) name = name.slice(0, MAX_KEY_LENGTH).replace(/[-.]+$/, "");
  return name.length > 0 ? name : UNKNOWN_MODEL_KEY;
}

export const UNKNOWN_MODEL_KEY = "unknown";

/**
 * Keys that do not name a model: missing ids, Claude Code's `<synthetic>`
 * placeholder messages, and router choices such as "auto" or "free". Their
 * tokens count toward coverage as unidentified and never get a model row.
 */
const UNIDENTIFIED_KEYS = new Set([
  UNKNOWN_MODEL_KEY,
  "synthetic",
  "auto",
  "auto-balanced",
  "auto-intelligence",
  "default",
  "free",
  "none",
  "null",
  "undefined",
]);

export function isIdentifiedModelKey(key: string): boolean {
  return !UNIDENTIFIED_KEYS.has(key);
}
