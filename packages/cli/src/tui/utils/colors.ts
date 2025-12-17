import type { SourceType } from "../types/index.js";

export const PROVIDER_COLORS = {
  anthropic: "#FF6B35",
  openai: "#10B981",
  google: "#3B82F6",
  cursor: "#8B5CF6",
  opencode: "#6B7280",
  deepseek: "#06B6D4",
  xai: "#EAB308",
  meta: "#6366F1",
  unknown: "#FFFFFF",
} as const;

export type ProviderType = keyof typeof PROVIDER_COLORS;

const PROVIDER_PATTERNS: readonly [RegExp, ProviderType][] = [
  [/claude|sonnet|opus|haiku/i, "anthropic"],
  [/gpt|^o1|^o3|codex|text-embedding|dall-e|whisper|tts/i, "openai"],
  [/gemini/i, "google"],
  [/deepseek/i, "deepseek"],
  [/grok/i, "xai"],
  [/llama|mixtral/i, "meta"],
  [/^auto$|cursor/i, "cursor"],
] as const;

const providerCache = new Map<string, ProviderType>();
const colorCache = new Map<string, string>();

export function getProviderFromModel(modelId: string): ProviderType {
  const cached = providerCache.get(modelId);
  if (cached) return cached;

  let provider: ProviderType = "unknown";
  for (const [pattern, type] of PROVIDER_PATTERNS) {
    if (pattern.test(modelId)) {
      provider = type;
      break;
    }
  }

  providerCache.set(modelId, provider);
  return provider;
}

export function getModelColor(modelId: string): string {
  const cached = colorCache.get(modelId);
  if (cached) return cached;

  const color = PROVIDER_COLORS[getProviderFromModel(modelId)];
  colorCache.set(modelId, color);
  return color;
}

export const SOURCE_COLORS: Record<SourceType, string> = {
  opencode: "#22c55e",
  claude: "#f97316",
  codex: "#3b82f6",
  cursor: "#a855f7",
  gemini: "#06b6d4",
};

export function getSourceColor(source: SourceType | string): string {
  return SOURCE_COLORS[source as SourceType] || "#888888";
}
