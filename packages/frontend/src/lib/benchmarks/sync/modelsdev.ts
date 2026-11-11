/**
 * models.dev catalog (https://models.dev/api.json, MIT, github.com/anomalyco/models.dev):
 * specs and list prices per provider. Only models that map to a key we
 * need are kept, and only the lab's own API plus a short list of large
 * providers, so the snapshot stays small and the prices stay comparable.
 */
import { modelKey } from "@/lib/explore/modelKey";
import { labForModel } from "@/lib/explore/modelMeta";
import type { CatalogSnapshot, ModelSpec, Offering } from "../schema";

export const MODELS_DEV_URL = "https://models.dev/api.json";

export interface ModelsDevModel {
  id: string;
  name?: string;
  release_date?: string;
  knowledge?: string;
  modalities?: { input?: string[]; output?: string[] };
  open_weights?: boolean;
  reasoning?: boolean;
  tool_call?: boolean;
  limit?: { context?: number; output?: number };
  cost?: { input?: number; output?: number; cache_read?: number; cache_write?: number };
  canonical_model_id?: string;
}

export interface ModelsDevProvider {
  id?: string;
  name?: string;
  doc?: string;
  models?: Record<string, ModelsDevModel>;
}

export type ModelsDevApi = Record<string, ModelsDevProvider>;

/** Lab slug (see LABS in @/lib/explore/modelMeta) -> the lab's own provider on models.dev. */
export const LAB_PROVIDERS: Record<string, string> = {
  anthropic: "anthropic",
  openai: "openai",
  google: "google",
  xai: "xai",
  "moonshot-ai": "moonshotai",
  alibaba: "alibaba",
  deepseek: "deepseek",
  zai: "zai",
  mistral: "mistral",
  minimax: "minimax",
  meta: "meta",
  nvidia: "nvidia",
  xiaomi: "xiaomi",
  stepfun: "stepfun",
  upstage: "upstage",
  inception: "inception",
  arcee: "arcee",
  meituan: "longcat",
};

/** Other providers whose list prices are shown after the lab's own API. */
export const PRICE_PROVIDERS: readonly string[] = [
  "amazon-bedrock",
  "google-vertex",
  "google-vertex-anthropic",
  "azure",
  "openrouter",
  "vercel",
  "togetherai",
  "fireworks-ai",
  "deepinfra",
  "groq",
  "cerebras",
  "baseten",
];

const FIRST_PARTY = new Set(Object.values(LAB_PROVIDERS));

function isTextModel(model: ModelsDevModel): boolean {
  const input = model.modalities?.input ?? [];
  const output = model.modalities?.output ?? [];
  return input.includes("text") && output.includes("text");
}

/**
 * The model key a models.dev entry describes. The canonical id is used when
 * present ("anthropic/claude-opus-5-5"); first-party ids are trusted as is.
 * Third-party entries without a canonical id are skipped (null).
 */
export function catalogKeyFor(providerId: string, model: ModelsDevModel): string | null {
  if (model.canonical_model_id) return modelKey(model.canonical_model_id);
  return FIRST_PARTY.has(providerId) ? modelKey(model.id) : null;
}

/** Every key models.dev describes for a text model. */
export function catalogKnownKeys(api: ModelsDevApi): Set<string> {
  const keys = new Set<string>();
  for (const [providerId, provider] of Object.entries(api)) {
    for (const model of Object.values(provider.models ?? {})) {
      if (!isTextModel(model)) continue;
      const key = catalogKeyFor(providerId, model);
      if (key) keys.add(key);
    }
  }
  return keys;
}

/** Keys the lab's own API lists with a price (text models only). */
export function firstPartyPricedKeys(api: ModelsDevApi): Set<string> {
  const keys = new Set<string>();
  for (const providerId of FIRST_PARTY) {
    for (const model of Object.values(api[providerId]?.models ?? {})) {
      if (!isTextModel(model) || typeof model.cost?.input !== "number") continue;
      const key = catalogKeyFor(providerId, model);
      if (key) keys.add(key);
    }
  }
  return keys;
}

function price(value: number | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function size(value: number | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

function isoDate(value: string | undefined): string | null {
  return value && /^\d{4}-\d{2}-\d{2}$/.test(value) ? value : null;
}

function knowledge(value: string | undefined): string | null {
  return value && /^\d{4}-\d{2}(?:-\d{2})?$/.test(value) ? value : null;
}

function offering(providerId: string, model: ModelsDevModel): Offering {
  return {
    provider: providerId,
    id: model.id,
    input: price(model.cost?.input),
    output: price(model.cost?.output),
    cacheRead: price(model.cost?.cache_read),
    cacheWrite: price(model.cost?.cache_write),
    context: size(model.limit?.context),
    maxOutput: size(model.limit?.output),
  };
}

/** One entry per provider: the id that is the key itself, else the shortest id. */
function pickEntry(key: string, entries: ModelsDevModel[]): ModelsDevModel | null {
  if (entries.length === 0) return null;
  return [...entries].sort((a, b) => {
    const exactA = modelKey(a.id) === key ? 0 : 1;
    const exactB = modelKey(b.id) === key ? 0 : 1;
    return exactA - exactB || a.id.length - b.id.length || (a.id < b.id ? -1 : 1);
  })[0];
}

export function buildCatalogSnapshot(api: ModelsDevApi, keys: ReadonlySet<string>, retrievedAt: string): CatalogSnapshot {
  // key -> provider -> entries
  const index = new Map<string, Map<string, ModelsDevModel[]>>();
  const wanted = new Set<string>([...FIRST_PARTY, ...PRICE_PROVIDERS]);
  for (const providerId of wanted) {
    for (const model of Object.values(api[providerId]?.models ?? {})) {
      if (!isTextModel(model)) continue;
      const key = catalogKeyFor(providerId, model);
      if (!key || !keys.has(key)) continue;
      const byProvider = index.get(key) ?? new Map<string, ModelsDevModel[]>();
      byProvider.set(providerId, [...(byProvider.get(providerId) ?? []), model]);
      index.set(key, byProvider);
    }
  }

  const providers: CatalogSnapshot["providers"] = {};
  const models: ModelSpec[] = [];
  for (const key of [...index.keys()].sort()) {
    const byProvider = index.get(key) ?? new Map<string, ModelsDevModel[]>();
    const labSlug = labForModel(key)?.slug ?? null;
    const labProvider = labSlug ? (LAB_PROVIDERS[labSlug] ?? null) : null;
    const labEntry = labProvider ? pickEntry(key, byProvider.get(labProvider) ?? []) : null;
    const order = [...(labProvider ? [labProvider] : []), ...PRICE_PROVIDERS.filter((p) => p !== labProvider)];
    const offerings: Offering[] = [];
    for (const providerId of order) {
      const entry = pickEntry(key, byProvider.get(providerId) ?? []);
      if (!entry || (price(entry.cost?.input) === null && price(entry.cost?.output) === null)) continue;
      offerings.push(offering(providerId, entry));
      const provider = api[providerId];
      providers[providerId] = { name: provider?.name ?? providerId, url: provider?.doc?.startsWith("https://") ? provider.doc : null };
    }
    const reference = labEntry ?? pickEntry(key, order.flatMap((p) => byProvider.get(p) ?? []));
    if (!reference) continue;
    models.push({
      modelKey: key,
      name: reference.name ?? key,
      labProvider: labEntry ? labProvider : null,
      releaseDate: isoDate(reference.release_date),
      knowledgeCutoff: knowledge(reference.knowledge),
      context: size(reference.limit?.context),
      maxOutput: size(reference.limit?.output),
      inputModalities: reference.modalities?.input ?? [],
      outputModalities: reference.modalities?.output ?? [],
      openWeights: typeof reference.open_weights === "boolean" ? reference.open_weights : null,
      reasoning: typeof reference.reasoning === "boolean" ? reference.reasoning : null,
      toolCall: typeof reference.tool_call === "boolean" ? reference.tool_call : null,
      offerings,
    });
  }
  return { source: "models.dev", retrievedAt, fetchUrls: [MODELS_DEV_URL], providers, models };
}
