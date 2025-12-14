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

    // Fuzzy matching - more strict to avoid false positives
    // e.g., "gpt-4" should NOT match "gpt-4o"
    const lowerModelID = modelID.toLowerCase();
    
    // First pass: exact match after normalizing (removing provider prefix)
    for (const [key, pricing] of Object.entries(this.pricingData)) {
      const lowerKey = key.toLowerCase();
      const normalizedKey = lowerKey.replace(/^(anthropic|openai|google|bedrock|vertex_ai)\//, "");
      if (normalizedKey === lowerModelID) {
        return pricing;
      }
    }
    
    // Second pass: match model ID as a complete segment (word boundary)
    // This prevents "gpt-4" from matching "gpt-4o" but allows "gpt-4-turbo" to match "gpt-4"
    for (const [key, pricing] of Object.entries(this.pricingData)) {
      const lowerKey = key.toLowerCase();
      // Check if modelID matches as a prefix followed by a version/variant separator
      const regex = new RegExp(`(^|/)${escapeRegex(lowerModelID)}(-\\d|$|@|:)`, "i");
      if (regex.test(lowerKey)) {
        return pricing;
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
    pricing: LiteLLMModelPricing
  ): number {
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
