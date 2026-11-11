/**
 * Seed a LOCAL Postgres with explore data for the stealth prototype.
 *
 * Inputs: public profiles cached by scripts/stealth/fetch-profiles.ts in
 * .stealth-cache/profiles/ (gitignored, never committed).
 *
 * Writes, through the real schema and in the shape the submit route writes:
 * - one account per cached real profile (same username, same daily usage);
 * - synthetic remixes of those profiles (username prefix "synthetic-") with
 *   scaled volumes, shifted dates, dropped days and varied agent/model mixes,
 *   so 7d/30d windows have realistic account counts;
 * - two leaderboard-hidden accounts with absurd volumes and a model nobody
 *   else uses ("synthetic-spoof-model"), to prove exclusion.
 *
 * Safety: refuses to run unless DATABASE_URL points at localhost/127.0.0.1.
 * Idempotent: deletes the accounts it is about to write (cascade) first.
 *
 * Usage (from packages/frontend):
 *   DATABASE_URL=postgresql://tokscale:tokscale@127.0.0.1:5480/tokscale \
 *     bun scripts/stealth/seed.ts [--accounts 300]
 */
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import postgres from "postgres";

const CACHE_DIR = join(import.meta.dir, "..", "..", ".stealth-cache", "profiles");
const MAX_COST_PER_MILLION = 50;
const DAY_MS = 86_400_000;
const CHUNK = 400;

// ---------------------------------------------------------------------------
// Profile shape returned by GET https://tokscale.ai/api/users/<username>
// ---------------------------------------------------------------------------
interface ModelCell {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
}
interface ProfileClient {
  client: string;
  modelId?: string;
  models: Record<string, ModelCell>;
  cost: number;
  messages: number;
}
interface ProfileDay {
  date: string;
  timestampMs: number | null;
  clients: ProfileClient[];
}
interface Profile {
  user: { username: string; displayName: string | null; avatarUrl: string | null; createdAt?: string };
  stats: { totalTokens: number; totalCost: number; submissionCount?: number };
  updatedAt?: string;
  submissionFreshness?: { cliVersion?: string | null };
  contributions: ProfileDay[];
}

// ---------------------------------------------------------------------------
// Output shape: daily_breakdown.source_breakdown as the submit route writes it
// ---------------------------------------------------------------------------
interface ClientBreakdown extends ModelCell {
  models: Record<string, ModelCell>;
  provenance: { schemaVersion: number; messageCount: number; modelCount: number };
}

interface SeedDay {
  date: string;
  timestampMs: number | null;
  breakdown: Record<string, ClientBreakdown>;
}

interface SeedAccount {
  id: string;
  githubId: number;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  hidden: boolean;
  cliVersion: string;
  submitCount: number;
  updatedAt: Date;
  days: SeedDay[];
}

// ---------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function uniform(rng: () => number, lo: number, hi: number): number {
  return lo + (hi - lo) * rng();
}

function intBetween(rng: () => number, lo: number, hi: number): number {
  return Math.floor(uniform(rng, lo, hi + 1));
}

function pick<T>(rng: () => number, items: readonly T[]): T {
  return items[Math.floor(rng() * items.length)];
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function isoDate(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}

function dateMs(iso: string): number {
  return Date.parse(`${iso}T00:00:00.000Z`);
}

function emptyCell(): ModelCell {
  return { tokens: 0, cost: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0, messages: 0 };
}

function scaleCell(cell: ModelCell, factor: number): ModelCell {
  const input = Math.round(cell.input * factor);
  const output = Math.round(cell.output * factor);
  const cacheRead = Math.round(cell.cacheRead * factor);
  const cacheWrite = Math.round(cell.cacheWrite * factor);
  const reasoning = Math.round(cell.reasoning * factor);
  return {
    // Keep tokens = sum of buckets, which is what the CLI submits.
    tokens: input + output + cacheRead + cacheWrite + reasoning,
    cost: Math.round(cell.cost * factor * 1e6) / 1e6,
    input,
    output,
    cacheRead,
    cacheWrite,
    reasoning,
    messages: Math.max(cell.messages > 0 ? 1 : 0, Math.round(cell.messages * factor)),
  };
}

function addCell(target: ModelCell, source: ModelCell): void {
  target.tokens += source.tokens;
  target.cost += source.cost;
  target.input += source.input;
  target.output += source.output;
  target.cacheRead += source.cacheRead;
  target.cacheWrite += source.cacheWrite;
  target.reasoning += source.reasoning;
  target.messages += source.messages;
}

/** Build one client entry the way foldIncomingClientContributions does: client = sum of models. */
function clientEntry(models: Record<string, ModelCell>): ClientBreakdown {
  const total = emptyCell();
  for (const cell of Object.values(models)) addCell(total, cell);
  return {
    ...total,
    models,
    provenance: { schemaVersion: 1, messageCount: total.messages, modelCount: Object.keys(models).length },
  };
}

function profileDayToBreakdown(day: ProfileDay): Record<string, ClientBreakdown> {
  const out: Record<string, ClientBreakdown> = {};
  for (const client of day.clients) {
    const models: Record<string, ModelCell> = {};
    for (const [model, cell] of Object.entries(client.models ?? {})) {
      if (cell.tokens > 0) models[model] = scaleCell(cell, 1);
    }
    if (Object.keys(models).length > 0) out[client.client] = clientEntry(models);
  }
  return out;
}

function githubIdFromAvatar(url: string | null, fallback: number): number {
  const match = url ? /\/u\/(\d+)/.exec(url) : null;
  const id = match ? Number(match[1]) : NaN;
  return Number.isSafeInteger(id) && id > 0 && id < 2_147_483_647 ? id : fallback;
}

// ---------------------------------------------------------------------------
// Real profiles
// ---------------------------------------------------------------------------
function loadProfiles(): Profile[] {
  const files = readdirSync(CACHE_DIR).filter((f) => f.endsWith(".json")).sort();
  const profiles: Profile[] = [];
  for (const file of files) {
    const profile = JSON.parse(readFileSync(join(CACHE_DIR, file), "utf8")) as Profile;
    const { totalTokens, totalCost } = profile.stats;
    const costPerMillion = totalTokens > 0 ? totalCost / (totalTokens / 1e6) : 0;
    if (costPerMillion > MAX_COST_PER_MILLION) {
      console.log(`skip ${profile.user.username}: $${costPerMillion.toFixed(2)} per million tokens`);
      continue;
    }
    profiles.push(profile);
  }
  return profiles;
}

function realAccount(profile: Profile, index: number): SeedAccount {
  const days: SeedDay[] = profile.contributions
    .map((day) => ({ date: day.date, timestampMs: day.timestampMs ?? null, breakdown: profileDayToBreakdown(day) }))
    .filter((day) => Object.keys(day.breakdown).length > 0);
  const last = days[days.length - 1]?.date;
  return {
    id: randomUUID(),
    githubId: githubIdFromAvatar(profile.user.avatarUrl, 800_000_000 + index),
    username: profile.user.username,
    displayName: profile.user.displayName,
    avatarUrl: profile.user.avatarUrl,
    hidden: false,
    cliVersion: (profile.submissionFreshness?.cliVersion ?? "unknown").slice(0, 20),
    submitCount: Math.max(1, profile.stats.submissionCount ?? days.length),
    updatedAt: profile.updatedAt ? new Date(profile.updatedAt) : new Date(dateMs(last ?? isoDate(Date.now())) + 12 * 3_600_000),
    days,
  };
}

// ---------------------------------------------------------------------------
// Synthetic remixes
// ---------------------------------------------------------------------------
/** Plausible "moved to the newer model" swaps, applied per remixed account. */
const MODEL_UPGRADES: Record<string, string> = {
  "claude-opus-4-5": "claude-opus-4-8",
  "claude-opus-4-6": "claude-opus-5",
  "claude-opus-4-7": "claude-opus-5",
  "claude-opus-4-8": "claude-opus-5",
  "claude-opus-5": "claude-opus-5-5",
  "claude-sonnet-4-5": "claude-sonnet-5",
  "claude-sonnet-4-6": "claude-sonnet-5",
  "claude-sonnet-5": "claude-sonnet-5-5",
  "gpt-5.2-codex": "gpt-5.3-codex",
  "gpt-5.4": "gpt-5.5",
  "gpt-5.5": "gpt-5.6-sol",
  "gpt-5.6-sol": "gpt-6.1-sol",
  "gemini-3-flash-preview": "gemini-3.8-flash",
  "gemini-3.5-flash": "gemini-3.8-flash",
};

/**
 * Synthetic accounts have no GitHub account, and the UI falls back to
 * github.com/<username>.png (a 404). Give them a plain monogram instead of
 * borrowing anyone's real avatar.
 */
function monogramAvatar(seed: string, label: string): string {
  let hash = 0;
  for (const ch of seed) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  const hue = hash % 360;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96">` +
    `<rect width="96" height="96" fill="hsl(${hue} 28% 22%)"/>` +
    `<text x="48" y="58" font-family="Arial, sans-serif" font-size="32" font-weight="700" fill="hsl(${hue} 60% 78%)" text-anchor="middle">${label}</text>` +
    `</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

const WORDS_A = ["amber", "brisk", "cedar", "dusk", "ember", "fjord", "glade", "harbor", "iris", "juniper", "kestrel", "lumen", "maple", "north", "onyx", "pine", "quartz", "river", "slate", "tidal"];
const WORDS_B = ["builder", "coder", "dev", "hacker", "maker", "shipper", "tinkerer", "writer"];

function remixAccount(base: SeedAccount, index: number, today: string): SeedAccount {
  const rng = mulberry32(0x5eed + index * 7919);
  // Remixes stay below the real accounts they copy, so real profiles keep the top ranks.
  const scale = Math.exp(uniform(rng, Math.log(0.02), Math.log(0.6)));

  // Where the remixed history ends: most accounts are current, some churned.
  const activity = rng();
  const endOffset = activity < 0.62 ? intBetween(rng, 0, 2) : activity < 0.84 ? intBetween(rng, 3, 24) : intBetween(rng, 25, 160);
  const end = isoDate(dateMs(today) - endOffset * DAY_MS);

  // How much of the base history is kept: some accounts are brand new.
  const keepFraction = rng() < 0.12 ? 0 : uniform(rng, 0.25, 1);
  const baseDays = base.days;
  const keepDays = keepFraction === 0 ? intBetween(rng, 5, 25) : Math.max(5, Math.round(baseDays.length * keepFraction));
  const kept = baseDays.slice(Math.max(0, baseDays.length - keepDays));
  const shiftMs = dateMs(end) - dateMs(kept[kept.length - 1].date);
  const dropRate = uniform(rng, 0, 0.35);

  // Agent mix: drop some clients (never the biggest), rescale the rest.
  const clientTokens = new Map<string, number>();
  for (const day of kept) for (const [client, entry] of Object.entries(day.breakdown)) clientTokens.set(client, (clientTokens.get(client) ?? 0) + entry.tokens);
  const biggest = [...clientTokens.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
  const clientScale = new Map<string, number>();
  for (const client of clientTokens.keys()) {
    if (client === biggest || rng() < 0.72) clientScale.set(client, uniform(rng, 0.4, 1.3));
  }

  // Model mix: some accounts moved to newer models.
  const upgrades = new Map<string, string>();
  for (const [from, to] of Object.entries(MODEL_UPGRADES)) if (rng() < 0.35) upgrades.set(from, to);

  // Long tail: a remix keeps the base account's main models but only some of
  // its rarely used ones, so niche models do not appear in every remix.
  const modelTokens = new Map<string, number>();
  for (const day of kept) {
    for (const entry of Object.values(day.breakdown)) {
      for (const [model, cell] of Object.entries(entry.models)) modelTokens.set(model, (modelTokens.get(model) ?? 0) + cell.tokens);
    }
  }
  const mainModels = new Set([...modelTokens.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5).map(([model]) => model));
  const droppedModels = new Set([...modelTokens.keys()].filter((model) => !mainModels.has(model) && rng() < 0.7));

  const days: SeedDay[] = [];
  for (const day of kept) {
    if (day.date !== kept[kept.length - 1].date && rng() < dropRate) continue;
    const date = isoDate(dateMs(day.date) + shiftMs);
    if (date > today) continue;
    const jitter = uniform(rng, 0.75, 1.25);
    const breakdown: Record<string, ClientBreakdown> = {};
    for (const [client, entry] of Object.entries(day.breakdown)) {
      const factor = clientScale.get(client);
      if (factor === undefined) continue;
      const models: Record<string, ModelCell> = {};
      for (const [model, cell] of Object.entries(entry.models)) {
        if (droppedModels.has(model)) continue;
        const scaled = scaleCell(cell, scale * factor * jitter);
        if (scaled.tokens <= 0) continue;
        const target = upgrades.get(model) ?? model;
        if (models[target]) addCell(models[target], scaled);
        else models[target] = scaled;
      }
      if (Object.keys(models).length > 0) breakdown[client] = clientEntry(models);
    }
    if (Object.keys(breakdown).length > 0) {
      days.push({ date, timestampMs: day.timestampMs === null ? null : day.timestampMs + shiftMs, breakdown });
    }
  }

  const word = `${pick(rng, WORDS_A)}-${pick(rng, WORDS_B)}`;
  const username = `synthetic-${word}-${String(index).padStart(3, "0")}`;
  const lastDate = days[days.length - 1]?.date ?? end;
  return {
    id: randomUUID(),
    githubId: 900_000_000 + index,
    username,
    displayName: `Synthetic ${word.replace("-", " ")} ${index}`,
    avatarUrl: monogramAvatar(username, word.split("-").map((w) => w[0].toUpperCase()).join("")),
    hidden: false,
    cliVersion: "stealth-seed",
    submitCount: Math.max(1, Math.round(days.length * uniform(rng, 1, 6))),
    updatedAt: new Date(Math.min(Date.now(), dateMs(lastDate) + intBetween(rng, 1, 23) * 3_600_000)),
    days,
  };
}

/** Leaderboard-hidden accounts with absurd volumes. They must never show up. */
function hiddenAccount(index: number, today: string): SeedAccount {
  const days: SeedDay[] = [];
  for (let offset = 0; offset < 45; offset++) {
    const date = isoDate(dateMs(today) - offset * DAY_MS);
    const opus = scaleCell({ tokens: 0, cost: 2_000_000, input: 4e11, output: 1e11, cacheRead: 4.4e12, cacheWrite: 1e11, reasoning: 0, messages: 900_000 }, 1);
    const spoof = scaleCell({ tokens: 0, cost: 1_000_000, input: 2e12, output: 5e11, cacheRead: 0, cacheWrite: 0, reasoning: 0, messages: 400_000 }, 1);
    days.push({
      date,
      timestampMs: dateMs(date) + 3_600_000,
      breakdown: {
        claude: clientEntry({ "claude-opus-5": opus }),
        codex: clientEntry({ "synthetic-spoof-model": spoof }),
      },
    });
  }
  days.reverse();
  return {
    id: randomUUID(),
    githubId: 990_000_000 + index,
    username: `synthetic-hidden-${String(index).padStart(2, "0")}`,
    displayName: `Synthetic hidden ${index}`,
    avatarUrl: monogramAvatar(`synthetic-hidden-${index}`, "H"),
    hidden: true,
    cliVersion: "stealth-seed",
    submitCount: 45,
    updatedAt: new Date(Math.min(Date.now(), dateMs(today) + 3_600_000)),
    days,
  };
}

// ---------------------------------------------------------------------------
// Database writes
// ---------------------------------------------------------------------------
function assertLocalDatabase(url: string): void {
  const host = new URL(url).hostname;
  if (host !== "127.0.0.1" && host !== "localhost" && host !== "::1") {
    throw new Error(`Refusing to seed a non-local database (host: ${host}). This script only writes to localhost.`);
  }
}

function dayTotals(breakdown: Record<string, ClientBreakdown>): ModelCell {
  const total = emptyCell();
  for (const entry of Object.values(breakdown)) addCell(total, entry);
  return total;
}

async function writeAccounts(sql: postgres.Sql, accounts: SeedAccount[]): Promise<void> {
  const usernames = accounts.map((a) => a.username);
  const githubIds = accounts.map((a) => a.githubId);
  await sql`DELETE FROM users WHERE username = ANY(${usernames}) OR github_id = ANY(${githubIds})`;

  const userRows = accounts.map((a) => ({
    id: a.id,
    github_id: a.githubId,
    username: a.username,
    display_name: a.displayName,
    avatar_url: a.avatarUrl,
    leaderboard_hidden: a.hidden,
    created_at: new Date(dateMs(a.days[0]?.date ?? isoDate(Date.now()))).toISOString(),
  }));
  for (let i = 0; i < userRows.length; i += CHUNK) {
    await sql`
      INSERT INTO users (id, github_id, username, display_name, avatar_url, leaderboard_hidden, created_at, updated_at)
      SELECT id, github_id, username, display_name, avatar_url, leaderboard_hidden, created_at, created_at
      FROM jsonb_to_recordset(${sql.json(userRows.slice(i, i + CHUNK) as never)}::jsonb)
        AS x(id uuid, github_id int, username varchar, display_name varchar, avatar_url text, leaderboard_hidden boolean, created_at timestamptz)
    `;
  }

  const deviceIds = new Map<string, string>();
  const submissionIds = new Map<string, string>();
  const deviceRows = accounts.map((a) => {
    deviceIds.set(a.id, randomUUID());
    return { id: deviceIds.get(a.id), user_id: a.id, last_submitted_at: a.updatedAt.toISOString() };
  });
  for (let i = 0; i < deviceRows.length; i += CHUNK) {
    await sql`
      INSERT INTO submitted_devices (id, user_id, device_key, display_name, last_submitted_at)
      SELECT id, user_id, 'stealth-seed', 'Seeded data', last_submitted_at
      FROM jsonb_to_recordset(${sql.json(deviceRows.slice(i, i + CHUNK) as never)}::jsonb)
        AS x(id uuid, user_id uuid, last_submitted_at timestamptz)
    `;
  }

  const submissionRows = accounts.map((a) => {
    submissionIds.set(a.id, randomUUID());
    const total = emptyCell();
    const sources = new Set<string>();
    const models = new Set<string>();
    for (const day of a.days) {
      addCell(total, dayTotals(day.breakdown));
      for (const [client, entry] of Object.entries(day.breakdown)) {
        sources.add(client);
        for (const model of Object.keys(entry.models)) models.add(model);
      }
    }
    return {
      id: submissionIds.get(a.id),
      user_id: a.id,
      total_tokens: total.tokens,
      total_cost: total.cost.toFixed(4),
      input_tokens: total.input,
      output_tokens: total.output,
      cache_creation_tokens: total.cacheWrite,
      cache_read_tokens: total.cacheRead,
      reasoning_tokens: total.reasoning,
      date_start: a.days[0]?.date ?? isoDate(Date.now()),
      date_end: a.days[a.days.length - 1]?.date ?? isoDate(Date.now()),
      sources_used: [...sources].sort(),
      models_used: [...models].sort(),
      cli_version: a.cliVersion,
      submit_count: a.submitCount,
      created_at: new Date(dateMs(a.days[0]?.date ?? isoDate(Date.now()))).toISOString(),
      updated_at: a.updatedAt.toISOString(),
    };
  });
  for (let i = 0; i < submissionRows.length; i += CHUNK) {
    await sql`
      INSERT INTO submissions (id, user_id, total_tokens, total_cost, input_tokens, output_tokens, cache_creation_tokens,
        cache_read_tokens, reasoning_tokens, date_start, date_end, sources_used, models_used, cli_version, submit_count,
        schema_version, created_at, updated_at)
      SELECT id, user_id, total_tokens, total_cost, input_tokens, output_tokens, cache_creation_tokens,
        cache_read_tokens, reasoning_tokens, date_start, date_end,
        ARRAY(SELECT jsonb_array_elements_text(sources_used)), ARRAY(SELECT jsonb_array_elements_text(models_used)),
        cli_version, submit_count, 1, created_at, updated_at
      FROM jsonb_to_recordset(${sql.json(submissionRows.slice(i, i + CHUNK) as never)}::jsonb)
        AS x(id uuid, user_id uuid, total_tokens bigint, total_cost numeric, input_tokens bigint, output_tokens bigint,
          cache_creation_tokens bigint, cache_read_tokens bigint, reasoning_tokens bigint, date_start date, date_end date,
          sources_used jsonb, models_used jsonb, cli_version varchar, submit_count int, created_at timestamptz, updated_at timestamptz)
    `;
  }

  const dayRows: Array<Record<string, unknown>> = [];
  for (const a of accounts) {
    for (const day of a.days) {
      const total = dayTotals(day.breakdown);
      dayRows.push({
        submission_id: submissionIds.get(a.id),
        submitted_device_id: deviceIds.get(a.id),
        date: day.date,
        tokens: total.tokens,
        cost: total.cost.toFixed(4),
        input_tokens: total.input,
        output_tokens: total.output,
        timestamp_ms: day.timestampMs,
        source_breakdown: day.breakdown,
      });
    }
  }
  for (let i = 0; i < dayRows.length; i += CHUNK) {
    await sql`
      INSERT INTO daily_breakdown (submission_id, submitted_device_id, date, tokens, cost, input_tokens, output_tokens,
        timestamp_ms, source_breakdown, cost_is_complete)
      SELECT submission_id, submitted_device_id, date, tokens, cost, input_tokens, output_tokens, timestamp_ms, source_breakdown, true
      FROM jsonb_to_recordset(${sql.json(dayRows.slice(i, i + CHUNK) as never)}::jsonb)
        AS x(submission_id uuid, submitted_device_id uuid, date date, tokens bigint, cost numeric, input_tokens bigint,
          output_tokens bigint, timestamp_ms bigint, source_breakdown jsonb)
    `;
  }
  console.log(`wrote ${accounts.length} accounts, ${dayRows.length} daily rows`);
}

function parseAccountsArg(argv: string[]): number {
  const i = argv.indexOf("--accounts");
  const n = i >= 0 ? Number(argv[i + 1]) : 300;
  if (!Number.isInteger(n) || n < 3) throw new Error("--accounts must be an integer >= 3");
  return n;
}

async function main(): Promise<void> {
  const url = process.env.DATABASE_URL;
  if (!url) throw new Error("DATABASE_URL is required (a local database).");
  assertLocalDatabase(url);
  const total = parseAccountsArg(process.argv.slice(2));

  const today = isoDate(Date.now());
  const profiles = loadProfiles();
  if (profiles.length === 0) throw new Error(`No cached profiles in ${CACHE_DIR}. Run scripts/stealth/fetch-profiles.ts first.`);
  const real = profiles.map((p, i) => realAccount(p, i)).filter((a) => a.days.length > 0);
  const hidden = [hiddenAccount(1, today), hiddenAccount(2, today)];
  const syntheticCount = Math.max(0, total - real.length - hidden.length);
  const synthetic: SeedAccount[] = [];
  for (let i = 0; i < syntheticCount; i++) {
    const remix = remixAccount(real[i % real.length], i + 1, today);
    if (remix.days.length > 0) synthetic.push(remix);
  }
  const accounts = [...real, ...synthetic, ...hidden];
  console.log(`real=${real.length} synthetic=${synthetic.length} hidden=${hidden.length} today=${today}`);

  const sql = postgres(url, { ssl: false, max: 1, onnotice: () => {} });
  try {
    await sql.begin(async (tx) => {
      await writeAccounts(tx as unknown as postgres.Sql, accounts);
    });
    // Re-seeding deletes the previous rows; reclaim them so query timings stay comparable.
    await sql`VACUUM ANALYZE users`;
    await sql`VACUUM ANALYZE submissions`;
    await sql`VACUUM ANALYZE submitted_devices`;
    await sql`VACUUM ANALYZE daily_breakdown`;
  } finally {
    await sql.end({ timeout: 5 });
  }
}

if (import.meta.main) {
  await main();
}
