/**
 * Pricing data fetcher using LiteLLM as source
 * Features disk caching with 1-hour TTL
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function normalizeModelName(modelId: string): string | null {
  const lower = modelId.toLowerCase();

  if (lower.includes("opus")) {
    if (lower.includes("4.5") || lower.includes("4-5")) {
      return "opus-4-5";
    } else if (lower.includes("4")) {
      return "opus-4";
    }
  }
  if (lower.includes("sonnet")) {
    if (lower.includes("4.5") || lower.includes("4-5")) {
      return "sonnet-4-5";
    } else if (lower.includes("4")) {
      return "sonnet-4";
    } else if (lower.includes("3.7") || lower.includes("3-7")) {
      return "sonnet-3-7";
    } else if (lower.includes("3.5") || lower.includes("3-5")) {
      return "sonnet-3-5";
    }
  }
  if (lower.includes("haiku") && (lower.includes("4.5") || lower.includes("4-5"))) {
    return "haiku-4-5";
  }

  if (lower === "o3") {
    return "o3";
  }
  if (lower.startsWith("gpt-4o") || lower === "gpt-4o") {
    return "gpt-4o";
  }
  if (lower.startsWith("gpt-4.1") || lower.includes("gpt-4.1")) {
    return "gpt-4.1";
  }

  if (lower.includes("gemini-2.5-pro")) {
    return "gemini-2.5-pro";
  }
  if (lower.includes("gemini-2.5-flash")) {
    return "gemini-2.5-flash";
  }
  if (lower.includes("gemini-3-pro-high")) {
    return "gemini-1.5-pro"; // Try simple key
  }

  return null;
}

export function isWordBoundaryMatch(haystack: string, needle: string): boolean {
  const pos = haystack.indexOf(needle);
  if (pos === -1) return false;

  const beforeOk = pos === 0 || !/[a-zA-Z0-9]/.test(haystack[pos - 1]);
  const afterOk =
    pos + needle.length === haystack.length ||
    !/[a-zA-Z0-9]/.test(haystack[pos + needle.length]);

  return beforeOk && afterOk;
}

const LITELLM_PRICING_URL =
  "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

const CACHE_TTL_MS = 60 * 60 * 1000; // 1 hour

export interface LiteLLMModelPricing {
  input_cost_per_token?: number;
  output_cost_per_token?: number;
  cache_creation_input_token_cost?: number;
  cache_read_input_token_cost?: number;
  input_cost_per_token_above_200k_tokens?: number;
  output_cost_per_token_above_200k_tokens?: number;
  cache_creation_input_token_cost_above_200k_tokens?: number;
  cache_read_input_token_cost_above_200k_tokens?: number;
}

export type PricingDataset = Record<string, LiteLLMModelPricing>;

interface CachedPricing {
  timestamp: number;
  data: PricingDataset;
}

/**
 * Format for passing pricing to Rust native module
 * Note: napi-rs expects undefined (not null) for Rust Option<T> fields
 */
export interface PricingEntry {
  modelId: string;
  pricing: {
    inputCostPerToken: number;
    outputCostPerToken: number;
    cacheReadInputTokenCost?: number;
    cacheCreationInputTokenCost?: number;
  };
}

function getCacheDir(): string {
  const cacheHome = process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
  return path.join(cacheHome, "tokscale");
}

function getCachePath(): string {
  return path.join(getCacheDir(), "pricing.json");
}

function loadCachedPricing(): CachedPricing | null {
  try {
    const cachePath = getCachePath();
    if (!fs.existsSync(cachePath)) {
      return null;
    }

    const content = fs.readFileSync(cachePath, "utf-8");
    const cached = JSON.parse(content) as CachedPricing;

    // Check TTL
    const age = Date.now() - cached.timestamp;
    if (age > CACHE_TTL_MS) {
      return null; // Cache expired
    }

    return cached;
  } catch {
    return null;
  }
}

function saveCachedPricing(data: PricingDataset): void {
  try {
    const cacheDir = getCacheDir();
    if (!fs.existsSync(cacheDir)) {
      fs.mkdirSync(cacheDir, { recursive: true });
    }

    const cached: CachedPricing = {
      timestamp: Date.now(),
      data,
    };

    fs.writeFileSync(getCachePath(), JSON.stringify(cached), "utf-8");
  } catch {
    // Ignore cache write errors
  }
}

export class PricingFetcher {
  private pricingData: PricingDataset | null = null;

  /**
   * Fetch pricing data (with disk cache, 1-hour TTL)
   */
  async fetchPricing(): Promise<PricingDataset> {
    if (this.pricingData) return this.pricingData;

    // Try to load from cache first
    const cached = loadCachedPricing();
    if (cached) {
      this.pricingData = cached.data;
      return this.pricingData;
    }

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), 15000);
    
    let response: Response;
    try {
      response = await fetch(LITELLM_PRICING_URL, { signal: controller.signal });
    } finally {
      clearTimeout(timeoutId);
    }
    
    if (!response.ok) {
      throw new Error(`Failed to fetch pricing: ${response.status}`);
    }

    this.pricingData = (await response.json()) as PricingDataset;

    // Save to cache
    saveCachedPricing(this.pricingData);

    return this.pricingData;
  }

  /**
   * Get raw pricing dataset
   */
  getPricingData(): PricingDataset | null {
    return this.pricingData;
  }

  /**
   * Convert pricing data to format expected by Rust native module
   */
  toPricingEntries(): PricingEntry[] {
    if (!this.pricingData) return [];

    return Object.entries(this.pricingData).map(([modelId, pricing]) => ({
      modelId,
      pricing: {
        inputCostPerToken: pricing.input_cost_per_token ?? 0,
        outputCostPerToken: pricing.output_cost_per_token ?? 0,
        // napi-rs expects undefined (not null) for Option<T> fields
        cacheReadInputTokenCost: pricing.cache_read_input_token_cost,
        cacheCreationInputTokenCost: pricing.cache_creation_input_token_cost,
      },
    }));
  }

  getModelPricing(modelID: string): LiteLLMModelPricing | null {
    if (!this.pricingData) return null;

    // Direct lookup
    if (this.pricingData[modelID]) {
      return this.pricingData[modelID];
    }

    // Try with provider prefix
    const prefixes = ["anthropic/", "openai/", "google/", "bedrock/"];
    for (const prefix of prefixes) {
      if (this.pricingData[prefix + modelID]) {
        return this.pricingData[prefix + modelID];
      }
    }

    const normalized = normalizeModelName(modelID);
    if (normalized) {
      if (this.pricingData[normalized]) {
        return this.pricingData[normalized];
      }
      for (const prefix of prefixes) {
        if (this.pricingData[prefix + normalized]) {
          return this.pricingData[prefix + normalized];
        }
      }
    }

    const lowerModelID = modelID.toLowerCase();
    const lowerNormalized = normalized?.toLowerCase();
    const sortedKeys = Object.keys(this.pricingData).sort();

    for (const key of sortedKeys) {
      const lowerKey = key.toLowerCase();
      if (isWordBoundaryMatch(lowerKey, lowerModelID)) {
        return this.pricingData[key];
      }
      if (lowerNormalized && isWordBoundaryMatch(lowerKey, lowerNormalized)) {
        return this.pricingData[key];
      }
    }

    for (const key of sortedKeys) {
      const lowerKey = key.toLowerCase();
      if (isWordBoundaryMatch(lowerModelID, lowerKey)) {
        return this.pricingData[key];
      }
      if (lowerNormalized && isWordBoundaryMatch(lowerNormalized, lowerKey)) {
        return this.pricingData[key];
      }
    }

    return null;
  }

  calculateCost(
    tokens: {
      input: number;
      output: number;
      reasoning?: number;
      cacheRead: number;
      cacheWrite: number;
    },
    pricing: LiteLLMModelPricing,
    modelId?: string // Add modelId parameter
  ): number {
    // Hardcode pricing for Gemini 3 Pro High
    if (modelId && modelId.includes("gemini-3-pro-high")) {
        const inputCost = tokens.input * (3.5 / 1000000);
        const outputCost = (tokens.output + (tokens.reasoning ?? 0)) * (10.5 / 1000000);
        // Assuming cache discount is similar to others (e.g. 50% or free read)
        // Let's be conservative and treat cache read as 10% of input cost
        const cacheReadCost = tokens.cacheRead * (0.35 / 1000000); 
        return inputCost + outputCost + cacheReadCost;
    }

    const inputCost = tokens.input * (pricing.input_cost_per_token ?? 0);
    const outputCost =
      (tokens.output + (tokens.reasoning ?? 0)) * (pricing.output_cost_per_token ?? 0);
    const cacheWriteCost =
      tokens.cacheWrite * (pricing.cache_creation_input_token_cost ?? 0);
    const cacheReadCost =
      tokens.cacheRead * (pricing.cache_read_input_token_cost ?? 0);

    return inputCost + outputCost + cacheWriteCost + cacheReadCost;
  }
}

/**
 * Clear pricing cache (for testing or forced refresh)
 */
export function clearPricingCache(): void {
  try {
    const cachePath = getCachePath();
    if (fs.existsSync(cachePath)) {
      fs.unlinkSync(cachePath);
    }
  } catch {
    // Ignore errors
  }
}
