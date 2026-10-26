/**
 * Deterministic fixture data for the explore contract. Used until the
 * database rollups land, and by tests. Numbers are illustrative only.
 */
import type {
  AgentRef,
  AgentUsageRow,
  CommunitySnapshot,
  Coverage,
  ExplorePeriod,
  HumanRow,
  ModelDetail,
  ModelRef,
  ModelUsageRow,
} from "./types";
import { modelLogoUrl } from "./modelMeta";

const M = (key: string, name: string, lab: string | null, labSlug: string | null): ModelRef => ({
  key,
  name,
  lab,
  labSlug,
  logoUrl: modelLogoUrl(key),
});
const A = (key: string, name: string): AgentRef => ({ key, name, logoUrl: null });

export const FIXTURE_MODELS: ModelRef[] = [
  M("claude-opus-5", "Claude Opus 5", "Anthropic", "anthropic"),
  M("gpt-5.6-sol", "GPT-5.6 Sol", "OpenAI", "openai"),
  M("gpt-5.6-terra", "GPT-5.6 Terra", "OpenAI", "openai"),
  M("claude-sonnet-5", "Claude Sonnet 5", "Anthropic", "anthropic"),
  M("gemini-3-pro", "Gemini 3 Pro", "Google", "google"),
  M("claude-fable-5", "Claude Fable 5", "Anthropic", "anthropic"),
  M("kimi-k3", "Kimi K3", "Moonshot AI", "moonshot-ai"),
  M("qwen-4-coder", "Qwen 4 Coder", "Alibaba", "alibaba"),
];

export const FIXTURE_AGENTS: AgentRef[] = [
  A("claude", "Claude Code"),
  A("codex", "Codex CLI"),
  A("opencode", "OpenCode"),
  A("cursor", "Cursor"),
  A("gemini", "Gemini CLI"),
  A("droid", "Droid"),
];

export function fixtureCoverage(period: ExplorePeriod): Coverage {
  const days = period === "7d" ? 7 : period === "30d" ? 30 : 400;
  const end = "2026-10-18";
  const startDate = new Date(Date.UTC(2026, 9, 18 - (days - 1)));
  return {
    period,
    start: startDate.toISOString().slice(0, 10),
    end,
    accounts: period === "7d" ? 812 : period === "30d" ? 1406 : 2337,
    modelIdentifiedShare: 0.987,
    lastDataAt: "2026-10-19T04:12:00.000Z",
  };
}

export function fixtureModels(period: ExplorePeriod): ModelUsageRow[] {
  const accounts = fixtureCoverage(period).accounts;
  const users = [512, 488, 301, 280, 166, 142, 77, 51];
  const prev = [470, 402, 120, 301, 150, 0, 60, 49];
  return FIXTURE_MODELS.map((m, i) => ({
    ...m,
    rank: i + 1,
    activeUsers: users[i],
    previousActiveUsers: period === "all" ? null : prev[i],
    adoption: users[i] / accounts,
    tokens: Math.round(9.1e12 / (i + 1.4)),
    estimatedCost: Math.round(4.2e6 / (i + 1.2)),
    isNew: period !== "all" && prev[i] === 0,
    topAgents: [
      { ...FIXTURE_AGENTS[i % 3], share: 0.58 },
      { ...FIXTURE_AGENTS[(i + 1) % 3], share: 0.27 },
      { ...FIXTURE_AGENTS[3], share: 0.09 },
    ],
  }));
}

export function fixtureAgents(period: ExplorePeriod): AgentUsageRow[] {
  const accounts = fixtureCoverage(period).accounts;
  const users = [633, 540, 402, 210, 96, 58];
  return FIXTURE_AGENTS.map((a, i) => ({
    ...a,
    rank: i + 1,
    activeUsers: users[i],
    previousActiveUsers: period === "all" ? null : Math.round(users[i] * 0.9),
    adoption: users[i] / accounts,
    tokens: Math.round(1.2e13 / (i + 1.3)),
    estimatedCost: Math.round(5.1e6 / (i + 1.1)),
    isNew: false,
    topModels: [
      { ...FIXTURE_MODELS[i % 4], share: 0.51 },
      { ...FIXTURE_MODELS[(i + 1) % 4], share: 0.3 },
    ],
  }));
}

const PEOPLE: Array<[string, string]> = [
  ["P-0001", "PT"],
  ["Dicklesworthstone", "Jeff Emanuel"],
  ["code-yeongyu", "YeonGyu-Kim"],
  ["kaptinlin", "kaptinlin"],
  ["vincentkoc", "Vincent Koc"],
  ["junhoyeo", "Junho Yeo"],
];

export function fixtureHumans(period: ExplorePeriod): HumanRow[] {
  return PEOPLE.map(([username, displayName], i) => ({
    rank: i + 1,
    username,
    displayName,
    avatarUrl: `https://github.com/${username}.png?size=96`,
    tokens: Math.round((period === "7d" ? 3.1e11 : 1.4e12) / (i + 1)),
    estimatedCost: Math.round((period === "7d" ? 61000 : 260000) / (i + 1)),
    activeDays: period === "7d" ? 7 - (i % 3) : 28 - i,
    topModels: [FIXTURE_MODELS[i % 4], FIXTURE_MODELS[(i + 2) % 5]],
    topAgents: [FIXTURE_AGENTS[i % 3], FIXTURE_AGENTS[(i + 1) % 4]],
  }));
}

export function fixtureSnapshot(period: ExplorePeriod): CommunitySnapshot {
  return {
    humans: fixtureHumans(period).slice(0, 5),
    models: fixtureModels(period).slice(0, 5),
    agents: fixtureAgents(period).slice(0, 5),
    coverage: fixtureCoverage(period),
  };
}

export function fixtureModelDetail(key: string, period: ExplorePeriod): ModelDetail | null {
  const rows = fixtureModels(period);
  const row = rows.find((r) => r.key === key);
  if (!row) return null;
  const coverage = fixtureCoverage(period);
  const days = period === "7d" ? 7 : period === "30d" ? 30 : 120;
  const trend = Array.from({ length: days }, (_, d) => {
    const date = new Date(Date.UTC(2026, 9, 18 - (days - 1 - d))).toISOString().slice(0, 10);
    const wave = 0.75 + 0.25 * Math.sin(d / 3);
    return { date, activeUsers: Math.round((row.activeUsers / 3) * wave), tokens: Math.round((row.tokens / days) * wave) };
  });
  return {
    model: { ...row, aliases: [`${row.key}-20260801`] },
    coverage,
    summary: {
      rank: row.rank,
      activeUsers: row.activeUsers,
      previousActiveUsers: row.previousActiveUsers,
      adoption: row.adoption,
      tokens: row.tokens,
      estimatedCost: row.estimatedCost,
      tokenMix: { input: 0.031, output: 0.004, cacheRead: 0.93, cacheWrite: 0.034, reasoning: 0.001 },
      firstSeen: "2026-08-01",
    },
    trend,
    agents: FIXTURE_AGENTS.slice(0, 4).map((a, i) => ({ ...a, activeUsers: Math.round(row.activeUsers / (i + 1.5)), share: [0.52, 0.26, 0.14, 0.08][i] })),
    humans: fixtureHumans(period).slice(0, 6),
    related: rows.filter((r) => r.labSlug === row.labSlug && r.key !== row.key),
  };
}
