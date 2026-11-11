/**
 * Display metadata for models and agents on the explore surfaces.
 *
 * Lab attribution is by key prefix only. It never guesses: a key that does
 * not start with a known family prefix gets lab = null.
 */
import { SOURCE_DISPLAY_NAMES, SOURCE_LOGOS } from "@/lib/constants";
import { agentIconUrl } from "@/lib/agentIcons";
import { ownValue } from "@/lib/safeRecord";
import type { AgentRef, ModelRef } from "./types";

export interface Lab {
  name: string;
  slug: string;
}

export const LABS = {
  anthropic: { name: "Anthropic", slug: "anthropic" },
  openai: { name: "OpenAI", slug: "openai" },
  google: { name: "Google", slug: "google" },
  xai: { name: "xAI", slug: "xai" },
  moonshot: { name: "Moonshot AI", slug: "moonshot-ai" },
  alibaba: { name: "Alibaba", slug: "alibaba" },
  deepseek: { name: "DeepSeek", slug: "deepseek" },
  zai: { name: "Z.ai", slug: "zai" },
  mistral: { name: "Mistral", slug: "mistral" },
  minimax: { name: "MiniMax", slug: "minimax" },
  meta: { name: "Meta", slug: "meta" },
  nvidia: { name: "NVIDIA", slug: "nvidia" },
  xiaomi: { name: "Xiaomi", slug: "xiaomi" },
  cursor: { name: "Cursor", slug: "cursor" },
  stepfun: { name: "StepFun", slug: "stepfun" },
  meituan: { name: "Meituan", slug: "meituan" },
  upstage: { name: "Upstage", slug: "upstage" },
  bytedance: { name: "ByteDance", slug: "bytedance" },
  kwaipilot: { name: "Kwaipilot", slug: "kwaipilot" },
  inception: { name: "Inception", slug: "inception" },
  arcee: { name: "Arcee AI", slug: "arcee-ai" },
} as const satisfies Record<string, Lab>;

/**
 * Model family marks vendored from LobeHub Icons (MIT) under
 * public/assets/v2/models/. A family mark can differ from its lab's mark
 * (Gemini and Gemma are both Google).
 */
export type ModelLogo =
  | "claude"
  | "openai"
  | "gemini"
  | "gemma"
  | "grok"
  | "kimi"
  | "qwen"
  | "deepseek"
  | "zai"
  | "mistral"
  | "minimax"
  | "meta"
  | "nvidia"
  | "xiaomimimo"
  | "cursor"
  | "stepfun"
  | "longcat"
  | "upstage"
  | "bytedance"
  | "kwaipilot"
  | "inception"
  | "arcee";

interface FamilyRule {
  pattern: RegExp;
  lab: Lab;
  logo: ModelLogo;
}

/** Ordered: the first matching rule wins. Each pattern is anchored at the start of the key. */
const FAMILY_RULES: FamilyRule[] = [
  { pattern: /^claude(?:-|$)/, lab: LABS.anthropic, logo: "claude" },
  { pattern: /^(?:gpt|chatgpt|codex)(?:-|$)/, lab: LABS.openai, logo: "openai" },
  { pattern: /^o\d+(?:-|$)/, lab: LABS.openai, logo: "openai" },
  { pattern: /^openai[.-]/, lab: LABS.openai, logo: "openai" },
  { pattern: /^gemini/, lab: LABS.google, logo: "gemini" },
  { pattern: /^gemma/, lab: LABS.google, logo: "gemma" },
  { pattern: /^grok(?:-|$)/, lab: LABS.xai, logo: "grok" },
  { pattern: /^kimi(?:-|$)/, lab: LABS.moonshot, logo: "kimi" },
  { pattern: /^qwen/, lab: LABS.alibaba, logo: "qwen" },
  { pattern: /^deepseek(?:-|$)/, lab: LABS.deepseek, logo: "deepseek" },
  { pattern: /^(?:zai-)?glm(?:-|\d|$)/, lab: LABS.zai, logo: "zai" },
  { pattern: /^(?:mistral|devstral|codestral|magistral)(?:-|$)/, lab: LABS.mistral, logo: "mistral" },
  { pattern: /^minimax(?:-|$)/, lab: LABS.minimax, logo: "minimax" },
  { pattern: /^llama[\d-]/, lab: LABS.meta, logo: "meta" },
  // Meta's Muse models (models.dev lists them under the Meta provider; Epoch lists "Meta AI").
  { pattern: /^muse-(?:spark|wonder)(?:-|$)/, lab: LABS.meta, logo: "meta" },
  { pattern: /^(?:nvidia-)?nemotron(?:-|$)/, lab: LABS.nvidia, logo: "nvidia" },
  { pattern: /^mimo(?:-|$)/, lab: LABS.xiaomi, logo: "xiaomimimo" },
  { pattern: /^composer-\d/, lab: LABS.cursor, logo: "cursor" },
  { pattern: /^step-\d/, lab: LABS.stepfun, logo: "stepfun" },
  { pattern: /^longcat(?:-|$)/, lab: LABS.meituan, logo: "longcat" },
  { pattern: /^solar-/, lab: LABS.upstage, logo: "upstage" },
  { pattern: /^seed-(?:\d|oss)/, lab: LABS.bytedance, logo: "bytedance" },
  { pattern: /^kat-coder(?:-|$)/, lab: LABS.kwaipilot, logo: "kwaipilot" },
  { pattern: /^mercury-\d/, lab: LABS.inception, logo: "inception" },
  { pattern: /^trinity-/, lab: LABS.arcee, logo: "arcee" },
];

/**
 * Agent products that prefix the model id they route to
 * ("antigravity-gemini-3.8-flash", "cursor-grok-4.6-xhigh"). Stripped only for
 * family lookup; the key itself is never rewritten.
 */
const PRODUCT_PREFIXES = ["antigravity-", "cursor-"];

function familyForModel(key: string): FamilyRule | null {
  const candidates = [key, ...PRODUCT_PREFIXES.filter((p) => key.startsWith(p)).map((p) => key.slice(p.length))];
  for (const candidate of candidates) {
    for (const rule of FAMILY_RULES) {
      if (rule.pattern.test(candidate)) return rule;
    }
  }
  return null;
}

/** Path of the family mark for a model key, or null when the family is unknown. */
export function modelLogoUrl(key: string): string | null {
  const family = familyForModel(key);
  return family ? `/assets/v2/models/${family.logo}.svg` : null;
}

export function labForModel(key: string): Lab | null {
  return familyForModel(key)?.lab ?? null;
}

const BRAND_TOKENS: Record<string, string> = {
  claude: "Claude",
  gpt: "GPT",
  chatgpt: "ChatGPT",
  codex: "Codex",
  gemini: "Gemini",
  gemma: "Gemma",
  grok: "Grok",
  kimi: "Kimi",
  qwen: "Qwen",
  deepseek: "DeepSeek",
  glm: "GLM",
  mistral: "Mistral",
  devstral: "Devstral",
  codestral: "Codestral",
  minimax: "MiniMax",
  mimo: "MiMo",
  longcat: "LongCat",
  kat: "KAT",
  qwq: "QwQ",
};

const UPPER_TOKENS = new Set(["oss", "vl", "it", "mlx", "gguf", "bf16", "fp8", "nvfp4"]);

const WORD_TOKENS: Record<string, string> = { xhigh: "XHigh" };

/** Brands written with a hyphen before their version: "GPT-5.6", "GLM-5.3". */
const HYPHENATED_BRANDS = new Set(["gpt", "glm"]);

function prettifyToken(token: string): string {
  const brand = ownValue(BRAND_TOKENS, token);
  if (brand) return brand;
  // "qwen3.6" -> "Qwen3.6"
  const brandWithVersion = /^([a-z]+)(\d[\d.]*)$/.exec(token);
  if (brandWithVersion && ownValue(BRAND_TOKENS, brandWithVersion[1])) {
    return `${BRAND_TOKENS[brandWithVersion[1]]}${brandWithVersion[2]}`;
  }
  if (UPPER_TOKENS.has(token)) return token.toUpperCase();
  const word = ownValue(WORD_TOKENS, token);
  if (word) return word;
  // Parameter sizes: "20b" -> "20B", "a17b" -> "A17B", "128k" -> "128K".
  if (/^a?\d+(?:\.\d+)?[bkmt]$/.test(token)) return token.toUpperCase();
  // Version-like tokens: "k3" -> "K3", "v4" -> "V4", "m2.7" -> "M2.7".
  if (/^[a-z]\d[\d.]*[a-z]?$/.test(token)) return token.charAt(0).toUpperCase() + token.slice(1);
  if (/^\d/.test(token)) return token;
  return token.charAt(0).toUpperCase() + token.slice(1);
}

/**
 * Human display name for a canonical key: "claude-opus-4-5" -> "Claude Opus 4.5",
 * "gpt-5.6-sol" -> "GPT-5.6 Sol", "kimi-k3" -> "Kimi K3".
 */
export function modelDisplayName(key: string): string {
  // OpenAI o-series keeps its lowercase brand: "o3", "o4-mini".
  if (/^o\d+(?:-|$)/.test(key)) return key;
  const tokens = key.split("-").filter((t) => t.length > 0);
  if (tokens.length === 0) return key;

  // Claude splits versions across tokens ("opus-4-5"); join digit runs with ".".
  // Also when a product prefix comes first ("antigravity-claude-sonnet-4-6").
  const merged: string[] = [];
  const isClaude = tokens.includes("claude");
  for (const token of tokens) {
    const previous = merged[merged.length - 1];
    if (isClaude && previous !== undefined && /^\d+$/.test(token) && /^\d+(?:\.\d+)*$/.test(previous)) {
      merged[merged.length - 1] = `${previous}.${token}`;
    } else {
      merged.push(token);
    }
  }

  const words: string[] = [];
  for (let i = 0; i < merged.length; i++) {
    const token = merged[i];
    const next = merged[i + 1];
    if (i === 0 && HYPHENATED_BRANDS.has(token) && next !== undefined && /^\d/.test(next)) {
      words.push(`${prettifyToken(token)}-${next}`);
      i++;
      continue;
    }
    words.push(prettifyToken(token));
  }
  return words.join(" ");
}

export function modelRef(key: string): ModelRef {
  const lab = labForModel(key);
  return {
    key,
    name: modelDisplayName(key),
    lab: lab?.name ?? null,
    labSlug: lab?.slug ?? null,
    logoUrl: modelLogoUrl(key),
  };
}

/** Legacy client ids folded into their current id (mirrors the submit path). */
const LEGACY_CLIENT_ALIASES: Record<string, string> = { kilocode: "kilo" };

export function agentKey(client: string): string {
  return ownValue(LEGACY_CLIENT_ALIASES, client) ?? client;
}

export function agentRef(client: string): AgentRef {
  const key = agentKey(client);
  const names = SOURCE_DISPLAY_NAMES as Record<string, string>;
  const logos = SOURCE_LOGOS as Record<string, string>;
  return { key, name: ownValue(names, key) ?? key, logoUrl: agentIconUrl(key) ?? ownValue(logos, key) ?? null };
}
