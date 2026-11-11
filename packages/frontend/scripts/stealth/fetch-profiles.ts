/**
 * Fetch a small sample of public tokscale.ai profiles for the local stealth seed.
 *
 * - Reads the cached leaderboard pages in .stealth-cache/ (fetches them once if missing).
 * - Picks every 5th account from page 1 (top ranks) and page 3 (mid ranks).
 * - Skips accounts whose cost per million tokens is above $50 (known spoofers).
 * - Fetches profiles STRICTLY one at a time, 150 s timeout each, at most 20.
 * - Never refetches a cached profile. Raw data stays in .stealth-cache/ (gitignored).
 *
 * Usage: bun scripts/stealth/fetch-profiles.ts
 */
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const ORIGIN = "https://tokscale.ai";
const CACHE_DIR = join(import.meta.dir, "..", "..", ".stealth-cache");
const PROFILE_DIR = join(CACHE_DIR, "profiles");
const MAX_PROFILES = 20;
const PROFILE_TIMEOUT_MS = 150_000;
const MAX_COST_PER_MILLION = 50;

interface LeaderboardUser {
  rank: number;
  username: string;
  totalTokens: number;
  totalCost: number;
}

async function fetchJson(url: string, timeoutMs: number): Promise<unknown> {
  const res = await fetch(url, {
    signal: AbortSignal.timeout(timeoutMs),
    headers: { accept: "application/json", "user-agent": "tokscale-stealth-seed/1 (local prototype)" },
  });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText} for ${url}`);
  return res.json();
}

function writeAtomic(path: string, data: unknown): void {
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, JSON.stringify(data));
  renameSync(tmp, path);
}

async function leaderboardPage(page: number): Promise<LeaderboardUser[]> {
  const path = join(CACHE_DIR, `lb-p${page}.json`);
  if (!existsSync(path)) {
    const data = await fetchJson(`${ORIGIN}/api/leaderboard?period=all&sortBy=tokens&limit=50&page=${page}`, 60_000);
    writeAtomic(path, data);
  }
  return (JSON.parse(readFileSync(path, "utf8")) as { users: LeaderboardUser[] }).users;
}

function costPerMillion(u: LeaderboardUser): number {
  return u.totalTokens > 0 ? u.totalCost / (u.totalTokens / 1e6) : 0;
}

/** Every 5th account starting at `start`, replacing spoofers with the next honest account. */
export function pickEveryFifth(users: LeaderboardUser[], start: number, count: number): LeaderboardUser[] {
  const honest = users.filter((u) => costPerMillion(u) <= MAX_COST_PER_MILLION);
  const picked: LeaderboardUser[] = [];
  for (let i = start; i < honest.length && picked.length < count; i += 5) picked.push(honest[i]);
  return picked;
}

async function main(): Promise<void> {
  mkdirSync(PROFILE_DIR, { recursive: true });
  const top = pickEveryFifth(await leaderboardPage(1), 1, 10);
  const mid = pickEveryFifth(await leaderboardPage(3), 0, 10);
  const targets = [...top, ...mid].slice(0, MAX_PROFILES);
  console.log(`targets: ${targets.map((u) => `${u.rank}:${u.username}`).join(", ")}`);

  for (const user of targets) {
    const path = join(PROFILE_DIR, `${user.username}.json`);
    if (existsSync(path)) {
      console.log(`cached  ${user.username}`);
      continue;
    }
    const started = Date.now();
    try {
      const data = await fetchJson(`${ORIGIN}/api/users/${encodeURIComponent(user.username)}`, PROFILE_TIMEOUT_MS);
      writeAtomic(path, data);
      console.log(`fetched ${user.username} in ${((Date.now() - started) / 1000).toFixed(1)}s`);
    } catch (error) {
      console.log(`failed  ${user.username} after ${((Date.now() - started) / 1000).toFixed(1)}s: ${String(error)}`);
    }
  }
}

if (import.meta.main) {
  await main();
}
