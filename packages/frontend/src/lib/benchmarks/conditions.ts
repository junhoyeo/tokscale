/**
 * Run conditions of a benchmark result: reasoning effort and the agent or
 * harness that ran the model. Conditions never change which model a result
 * belongs to (see ./mapping.ts); they are shown next to the score.
 */

export const EFFORTS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const;
export type Effort = (typeof EFFORTS)[number];

const EFFORT_ALIASES: Record<string, Effort> = {
  none: "none",
  "no reasoning": "none",
  "non-reasoning": "none",
  "no-thinking": "none",
  off: "none",
  minimal: "minimal",
  low: "low",
  medium: "medium",
  med: "medium",
  high: "high",
  xhigh: "xhigh",
  "x-high": "xhigh",
  "x high": "xhigh",
  "extra high": "xhigh",
  "extra-high": "xhigh",
  max: "max",
  maximum: "max",
};

/**
 * Map a source's effort label to the shared scale. "unknown", "", and labels
 * we do not recognise return null: the result then has no effort tag.
 */
export function normalizeEffort(raw: string | null | undefined): Effort | null {
  if (!raw) return null;
  const key = raw.trim().toLowerCase().replace(/_/g, " ");
  return Object.prototype.hasOwnProperty.call(EFFORT_ALIASES, key) ? EFFORT_ALIASES[key] : null;
}

const EFFORT_LABELS: Record<Effort, string> = {
  none: "No reasoning",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "XHigh",
  max: "Max",
};

export function effortLabel(effort: Effort): string {
  return EFFORT_LABELS[effort];
}

/** Thinking-token budgets written as a suffix: "32K", "16k", "1K". */
const BUDGET_SUFFIX = /^\d+(?:\.\d+)?[kK]$/;

export interface SplitVersion {
  /** The model id without the condition suffix. */
  base: string;
  effort: Effort | null;
  /** Thinking budget such as "32K" when the suffix is a token budget. */
  thinkingBudget: string | null;
  /** True when the source wrote "_unknown": the effort exists but was not reported. */
  effortUnknown: boolean;
}

/**
 * Split an Epoch AI "model version" into model id and conditions:
 * "claude-opus-5-5_max" -> base "claude-opus-5-5", effort "max";
 * "claude-sonnet-4-5_32K" -> thinking budget "32K"; "gpt-6-astra_unknown"
 * -> no effort. A suffix that is not a condition stays part of the id.
 */
export function splitEpochVersion(version: string): SplitVersion {
  const trimmed = version.trim();
  const underscore = trimmed.lastIndexOf("_");
  if (underscore > 0) {
    const base = trimmed.slice(0, underscore);
    const suffix = trimmed.slice(underscore + 1);
    if (suffix.toLowerCase() === "unknown") return { base, effort: null, thinkingBudget: null, effortUnknown: true };
    const effort = normalizeEffort(suffix);
    if (effort) return { base, effort, thinkingBudget: null, effortUnknown: false };
    if (BUDGET_SUFFIX.test(suffix)) return { base, effort: null, thinkingBudget: suffix.toUpperCase(), effortUnknown: false };
  }
  return { base: trimmed, effort: null, thinkingBudget: null, effortUnknown: false };
}

export interface SplitName {
  /** Display name without the trailing condition group. */
  name: string;
  effort: Effort | null;
  /** Thinking budget such as "32K". */
  thinkingBudget: string | null;
  /** Harness named in the group: "(xHigh, Codex Harness)" -> "Codex". */
  agent: string | null;
  /** Reasoning mode words that were removed, e.g. "thinking". */
  qualifier: string | null;
}

const MODE_WORDS = new Set(["thinking", "reasoning", "non-thinking", "instant", "unknown", "unknown thinking"]);

/**
 * Split a display name with a trailing parenthesised condition group:
 * "Claude Opus 5.5 (Max)" -> "Claude Opus 5.5" + max;
 * "GPT-5.5 (xHigh, Codex Harness)" -> "GPT-5.5" + xhigh + agent "Codex";
 * "Claude Opus 4.5 (High, 32K, 20251101)" -> "Claude Opus 4.5-20251101" + high + 32K
 * (the dated snapshot stays in the name; the key folds it later).
 * A group with anything else ("(0902)", "(NVFP4)") is left in the name.
 */
export function splitDisplayName(raw: string): SplitName {
  const trimmed = raw.trim();
  const none: SplitName = { name: trimmed, effort: null, thinkingBudget: null, agent: null, qualifier: null };
  const match = /^(.*\S)\s*\(([^()]+)\)$/.exec(trimmed);
  if (!match) return none;
  let effort: Effort | null = null;
  let thinkingBudget: string | null = null;
  let agent: string | null = null;
  let date: string | null = null;
  const modes: string[] = [];
  for (const part of match[2].split(",").map((p) => p.trim()).filter(Boolean)) {
    const lower = part.toLowerCase();
    const asEffort = normalizeEffort(part);
    const harness = /^(.+?)\s+harness$/i.exec(part);
    if (asEffort && effort === null) effort = asEffort;
    else if (BUDGET_SUFFIX.test(part) && thinkingBudget === null) thinkingBudget = part.toUpperCase();
    else if (/^\d{8}$/.test(part) && date === null) date = part;
    else if (harness && agent === null) agent = harness[1].trim();
    else if (MODE_WORDS.has(lower)) modes.push(lower);
    else return none;
  }
  return {
    name: date ? `${match[1]}-${date}` : match[1],
    effort,
    thinkingBudget,
    agent,
    qualifier: modes.length > 0 ? modes.join(", ") : null,
  };
}

/**
 * Agent products that Tokscale also tracks as clients. The key is the
 * client id used in submissions, so a result run in "Claude Code" can sit
 * next to Claude Code's share of the model's tokens.
 */
const AGENT_CLIENT_KEYS: Record<string, string> = {
  "claude code": "claude",
  codex: "codex",
  "codex cli": "codex",
  "grok build": "grok",
  "muse code": "muse",
  "gemini cli": "gemini",
  opencode: "opencode",
  cursor: "cursor",
  droid: "droid",
  "kimi cli": "kimi",
  "qwen code": "qwen",
  "mistral vibe": "vibe",
};

/** Tokscale client key for an agent label, or null when we do not track it. */
export function agentClientKey(label: string | null | undefined): string | null {
  if (!label) return null;
  const key = label.trim().toLowerCase();
  return Object.prototype.hasOwnProperty.call(AGENT_CLIENT_KEYS, key) ? AGENT_CLIENT_KEYS[key] : null;
}
