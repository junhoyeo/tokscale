/**
 * Terminal-Bench 4.0 official leaderboard (tbench.ai). The page embeds the
 * leaderboard as JSON inside the Next.js flight payload; we read only the
 * result rows (agent, model, effort, accuracy, cost, tokens), never task
 * content. The site's data terms are not stated, so every cell links back.
 */
import { agentClientKey, normalizeEffort } from "../conditions";
import { roundDerived } from "../csv";
import type { BenchmarkResult, SourceSnapshot } from "../schema";
import { mapExternalModel, type SyncContext } from "./context";

export const TBENCH_URL = "https://www.tbench.ai/leaderboard";
export const TBENCH_TITLE = "Terminal-Bench 4.0";

export interface TbenchRow {
  rank: number;
  status?: string;
  created_at?: string;
  n_trials?: number;
  metadata: {
    date?: string;
    reasoning_effort?: string;
    agent_display?: { url?: string; label?: string };
    model_display?: { url?: string; label?: string };
    model_org?: { url?: string; label?: string };
  };
  metrics: {
    accuracy?: number;
    accuracy_ci95_half_width?: number;
    total_cost_usd?: number;
    total_tokens?: number;
  };
}

export interface TbenchLeaderboard {
  title: string;
  updatedAt: string | null;
  rows: TbenchRow[];
}

/** Concatenate the string chunks of `self.__next_f.push([1, "..."])` calls. */
export function decodeFlightPayload(html: string): string {
  const out: string[] = [];
  const pattern = /self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)/g;
  for (const match of html.matchAll(pattern)) out.push(JSON.parse(`"${match[1]}"`) as string);
  return out.join("");
}

/** The balanced JSON array or object that starts at `start`, as text. */
export function sliceBalancedJson(text: string, start: number): string | null {
  const open = text[start];
  if (open !== "[" && open !== "{") return null;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i++) {
    const ch = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "[" || ch === "{") depth++;
    else if (ch === "]" || ch === "}") {
      depth--;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  return null;
}

/** Find the leaderboard titled `title` in the page and return its rows. */
export function extractTbenchLeaderboard(html: string, title: string = TBENCH_TITLE): TbenchLeaderboard {
  const payload = decodeFlightPayload(html);
  const titleAt = payload.indexOf(`"title":${JSON.stringify(title)}`);
  if (titleAt < 0) throw new Error(`${title} leaderboard not found in the page`);
  const rowsKey = payload.indexOf('"rows":[', titleAt);
  if (rowsKey < 0) throw new Error(`${title} leaderboard has no rows`);
  const rowsText = sliceBalancedJson(payload, rowsKey + '"rows":'.length);
  if (!rowsText) throw new Error(`${title} rows are not valid JSON`);
  const between = payload.slice(titleAt, rowsKey);
  const updated = /"updated_at":"([^"]+)"/.exec(between);
  return { title, updatedAt: updated ? updated[1] : null, rows: JSON.parse(rowsText) as TbenchRow[] };
}

/** Last path segment of a model page URL, a hint like "claude-opus-5-5". */
function urlSlug(url: string | undefined): string | null {
  if (!url) return null;
  try {
    const segment = new URL(url).pathname.split("/").filter(Boolean).pop();
    return segment && /^[a-z0-9][a-z0-9._-]*$/i.test(segment) ? segment : null;
  } catch {
    return null;
  }
}

function isoDay(value: string | undefined): string | null {
  const match = /^(\d{4}-\d{2}-\d{2})/.exec(value ?? "");
  return match ? match[1] : null;
}

export function parseTbenchRows(rows: readonly TbenchRow[], ctx: SyncContext): BenchmarkResult[] {
  const out: BenchmarkResult[] = [];
  for (const row of rows) {
    if (row.status && row.status !== "display") continue;
    const label = row.metadata.model_display?.label?.trim();
    const accuracy = row.metrics.accuracy;
    if (!label || typeof accuracy !== "number" || !Number.isFinite(accuracy)) continue;
    const agent = row.metadata.agent_display?.label?.trim() || null;
    const slug = urlSlug(row.metadata.model_display?.url);
    const published = isoDay(row.created_at);
    const resolution = mapExternalModel(
      ctx,
      "tbench",
      { externalId: label, ids: slug ? [slug] : [], names: [label], org: row.metadata.model_org?.label ?? null },
      { benchmark: "terminal-bench-4", name: label, date: published },
      "names",
    );
    const half = row.metrics.accuracy_ci95_half_width;
    const cost = row.metrics.total_cost_usd;
    const tokens = row.metrics.total_tokens;
    out.push({
      externalId: `${agent ?? "unknown agent"} / ${label} / ${row.metadata.reasoning_effort ?? "effort not stated"}`,
      externalModel: label,
      name: label,
      modelKey: resolution.key,
      mapping: resolution.method,
      reviewed: resolution.reviewed,
      effort: normalizeEffort(row.metadata.reasoning_effort),
      thinkingBudget: null,
      agent,
      agentKey: agentClientKey(agent),
      score: roundDerived(accuracy / 100),
      ciLow: typeof half === "number" ? roundDerived((accuracy - half) / 100) : null,
      ciHigh: typeof half === "number" ? roundDerived((accuracy + half) / 100) : null,
      stderr: null,
      votes: null,
      sessions: null,
      rank: Number.isInteger(row.rank) && row.rank > 0 ? row.rank : null,
      // $0 means "not reported" on this board, never free.
      costUsd: typeof cost === "number" && cost > 0 ? cost : null,
      costBasis: typeof cost === "number" && cost > 0 ? "per run" : null,
      tokens: typeof tokens === "number" && tokens > 0 ? tokens : null,
      modelReleaseDate: isoDay(row.metadata.date),
      runDate: null,
      publishedDate: published,
      sourceUrl: TBENCH_URL,
      notes: typeof row.n_trials === "number" ? `${row.n_trials} trials` : null,
    });
  }
  return out;
}

export function buildTbenchSnapshot(board: TbenchLeaderboard, ctx: SyncContext, retrievedAt: string): SourceSnapshot {
  const results = parseTbenchRows(board.rows, ctx);
  let dataDate: string | null = isoDay(board.updatedAt ?? undefined);
  for (const r of results) {
    if (r.publishedDate && (dataDate === null || r.publishedDate > dataDate)) dataDate = r.publishedDate;
  }
  return {
    source: "tbench",
    retrievedAt,
    fetchUrls: [TBENCH_URL],
    dataDate,
    benchmarks: [{ slug: "terminal-bench-4", sourceName: board.title, meta: null, results }],
  };
}
