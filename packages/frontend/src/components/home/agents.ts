import { AGENT_ICONS, agentIconUrl } from "@/lib/agentIcons";
import { SOURCE_DISPLAY_NAMES, SOURCE_LOGOS } from "@/lib/constants";
import { ownValue } from "@/lib/safeRecord";
import type { ClientType } from "@/lib/types";
import type { AgentRef } from "@/lib/explore";

export interface SupportedAgent {
  /** First client id that maps to this name and logo. */
  key: string;
  name: string;
  logoUrl: string;
}

/** Shown first on /tracker; everything else follows alphabetically. */
const FEATURED_AGENT_KEYS: ClientType[] = [
  "claude",
  "codex",
  "cursor",
  "gemini",
  "opencode",
  "copilot",
  "amp",
  "droid",
  "cline",
  "roocode",
  "kilocode",
  "zed",
  "goose",
  "kimi",
  "qwen",
  "warp",
];

/**
 * Every client Tokscale can read, one entry per distinct name and logo.
 * Aliases that share both (for example `kilo` and `kilocode`) collapse into one.
 */
export function listSupportedAgents(
  names: Record<string, string> = SOURCE_DISPLAY_NAMES,
  logos: Record<string, string> = SOURCE_LOGOS,
  featured: readonly string[] = FEATURED_AGENT_KEYS,
  icons: Readonly<Record<string, string>> = AGENT_ICONS,
): SupportedAgent[] {
  const seen = new Set<string>();
  const unique: SupportedAgent[] = [];
  for (const [key, name] of Object.entries(names)) {
    // The README icon when there is one, else the client registry logo.
    const logoUrl = ownValue(icons, key) ?? logos[key];
    if (!logoUrl) continue;
    const identity = `${name.toLowerCase()}\u0000${logoUrl}`;
    if (seen.has(identity)) continue;
    seen.add(identity);
    unique.push({ key, name, logoUrl });
  }

  const rank = new Map(featured.map((key, index) => [key, index]));
  return unique.sort((a, b) => {
    const ra = rank.get(a.key);
    const rb = rank.get(b.key);
    if (ra !== undefined || rb !== undefined) return (ra ?? Infinity) - (rb ?? Infinity);
    return a.name.localeCompare(b.name, "en", { sensitivity: "base" });
  });
}

/** Logo for an agent row: the row's own logo, then the README icon, then the client registry, then none. */
export function agentLogoUrl(agent: Pick<AgentRef, "key" | "logoUrl">): string | null {
  if (agent.logoUrl) return agent.logoUrl;
  const key = agent.key.toLowerCase();
  const icon = agentIconUrl(key);
  if (icon) return icon;
  return Object.prototype.hasOwnProperty.call(SOURCE_LOGOS, key) ? SOURCE_LOGOS[key as ClientType] : null;
}
