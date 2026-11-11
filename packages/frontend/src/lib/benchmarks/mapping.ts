/**
 * Map external model ids and names to canonical Tokscale model keys.
 *
 * Rules (see docs/stealth/models-dashboard-plan.md, "Data model"):
 * - The model key is the canonical Tokscale key (`modelKey` in
 *   @/lib/explore/modelKey). Effort and agent are conditions, never identity.
 * - A match needs a key that exists: one seen in Tokscale submissions or in
 *   the models.dev catalog. Nothing is guessed; an id with no candidate in
 *   that set is reported as unmapped.
 * - "exact": the source's own id already is the key. "normalized": the key
 *   was reached by the structural rules below. "manual": a reviewed entry
 *   in src/content/benchmarks/aliases.json, which always wins.
 */
import { modelKey } from "@/lib/explore/modelKey";
import { splitDisplayName } from "./conditions";

export type MappingMethod = "exact" | "normalized" | "manual";

export interface ExternalModelRef {
  /** Stable id inside its source (Epoch base id, LMArena name without effort, ...). */
  externalId: string;
  /** Model ids the source gives for this model, most specific first. */
  ids?: readonly string[];
  /** Display names the source gives, e.g. "Claude Opus 5.5 (Max)". */
  names?: readonly string[];
  /** Organization as written by the source, used for family prefixes. */
  org?: string | null;
}

export interface KeyCandidate {
  key: string;
  method: Exclude<MappingMethod, "manual">;
}

export interface AliasRule {
  source: string;
  externalId: string;
  /** null marks a reviewed "does not match any Tokscale model". */
  modelKey: string | null;
  method: MappingMethod;
  reviewed: boolean;
  note?: string;
}

export interface Resolution {
  key: string | null;
  method: MappingMethod | null;
  reviewed: boolean;
}

const ANTHROPIC_FAMILIES = /^(?:opus|sonnet|haiku|fable|mythos)(?:-|$)/;

/** Lowercase, collapse spaces to "-", drop characters that never appear in ids. */
export function slugifyModelName(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/\s+/g, "-")
    .replace(/[^a-z0-9._/:-]+/g, "")
    .replace(/-{2,}/g, "-")
    .replace(/^-+|-+$/g, "");
}

function isAnthropic(org: string | null | undefined): boolean {
  return Boolean(org && /anthropic/i.test(org));
}

/**
 * Ordered key candidates for an external model. Ids come before names; a
 * name is tried as written, with the Anthropic family prefix when the
 * source drops "Claude", and with the brand joined to its version
 * ("qwen-3.8-max" -> "qwen3.8-max").
 */
export function candidateKeys(ref: ExternalModelRef, prefer: "ids" | "names" = "ids"): KeyCandidate[] {
  const out: KeyCandidate[] = [];
  const seen = new Set<string>();
  const push = (key: string, method: KeyCandidate["method"]) => {
    if (!key || key === "unknown" || seen.has(key)) return;
    seen.add(key);
    out.push({ key, method });
  };

  if (prefer === "names") pushNames(ref, push);
  pushIds(ref, push);
  if (prefer === "ids") pushNames(ref, push);
  return out;
}

type Push = (key: string, method: KeyCandidate["method"]) => void;

function pushIds(ref: ExternalModelRef, push: Push): void {
  // Plain ids before host-prefixed ones ("kimi-k2.5" before "fireworks/kimi-k2p5").
  const ids = [...(ref.ids ?? [])].sort((a, b) => Number(a.includes("/")) - Number(b.includes("/")));
  for (const id of ids) {
    const trimmed = id.trim();
    if (!trimmed) continue;
    const key = modelKey(trimmed);
    push(key, key === trimmed.toLowerCase() ? "exact" : "normalized");
    // Dated snapshots fold into the model: "gpt-5.4-2026-03-05" -> "gpt-5.4".
    const undated = trimmed.replace(/-\d{4}-\d{2}-\d{2}$/, "");
    if (undated !== trimmed) push(modelKey(undated), "normalized");
  }
}

function pushNames(ref: ExternalModelRef, push: Push): void {
  for (const raw of ref.names ?? []) {
    const slug = slugifyModelName(splitDisplayName(raw).name);
    if (!slug) continue;
    const variants = [slug];
    if (isAnthropic(ref.org) && ANTHROPIC_FAMILIES.test(slug)) variants.unshift(`claude-${slug}`);
    for (const variant of [...variants]) {
      const joined = variant.replace(/^([a-z]+)-(\d)/, "$1$2");
      if (joined !== variant) variants.push(joined);
    }
    for (const variant of variants) push(modelKey(variant), "normalized");
  }
}

export function aliasRuleId(source: string, externalId: string): string {
  return `${source}\u0000${externalId}`;
}

/**
 * Resolve one external model. A manual alias wins (including a reviewed
 * "no match"); otherwise the first candidate present in `knownKeys`.
 */
export function resolveModelKey(
  source: string,
  ref: ExternalModelRef,
  knownKeys: ReadonlySet<string>,
  aliases: ReadonlyMap<string, AliasRule>,
  prefer: "ids" | "names" = "ids",
): Resolution {
  const rule = aliases.get(aliasRuleId(source, ref.externalId));
  if (rule && rule.method === "manual") return { key: rule.modelKey, method: rule.modelKey ? "manual" : null, reviewed: rule.reviewed };
  for (const candidate of candidateKeys(ref, prefer)) {
    if (knownKeys.has(candidate.key)) {
      const reviewed = Boolean(rule && rule.reviewed && rule.modelKey === candidate.key);
      return { key: candidate.key, method: candidate.method, reviewed };
    }
  }
  return { key: null, method: null, reviewed: false };
}
