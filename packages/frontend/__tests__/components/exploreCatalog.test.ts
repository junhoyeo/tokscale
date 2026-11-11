import { describe, expect, it } from "vitest";
import { groupModelsByLab, matchesModelQuery, UNKNOWN_LAB_KEY } from "@/components/explore/catalog";
import type { ModelUsageRow } from "@/lib/explore";

function row(key: string, lab: string | null, labSlug: string | null, activeUsers: number, tokens: number): ModelUsageRow {
  return {
    key,
    name: key.toUpperCase(),
    lab,
    labSlug,
    logoUrl: null,
    rank: 0,
    activeUsers,
    previousActiveUsers: null,
    adoption: 0,
    tokens,
    estimatedCost: 0,
    isNew: false,
    topAgents: [],
  };
}

const ROWS = [
  row("a-small", "Anthropic", "anthropic", 10, 900),
  row("o-big", "OpenAI", "openai", 500, 100),
  row("a-big", "Anthropic", "anthropic", 400, 50),
  row("mystery", null, null, 999, 999),
];

describe("groupModelsByLab", () => {
  it("orders labs by their strongest model and keeps unknown last", () => {
    const groups = groupModelsByLab(ROWS, "users");
    expect(groups.map((g) => g.key)).toEqual(["openai", "anthropic", UNKNOWN_LAB_KEY]);
    expect(groups[1].models.map((m) => m.key)).toEqual(["a-big", "a-small"]);
    expect(groups[2].name).toBe("Lab unknown");
    expect(groups[2].slug).toBeNull();
  });

  it("follows the token sort inside and across labs", () => {
    const groups = groupModelsByLab(ROWS, "tokens");
    expect(groups.map((g) => g.key)).toEqual(["anthropic", "openai", UNKNOWN_LAB_KEY]);
    expect(groups[0].models.map((m) => m.key)).toEqual(["a-small", "a-big"]);
  });

  it("does not mutate the input", () => {
    const copy = ROWS.map((r) => r.key);
    groupModelsByLab(ROWS, "users");
    expect(ROWS.map((r) => r.key)).toEqual(copy);
  });
});

describe("matchesModelQuery", () => {
  it("matches every word against name, key and lab", () => {
    const model = { name: "Claude Opus 5", key: "claude-opus-5", lab: "Anthropic" };
    expect(matchesModelQuery(model, "")).toBe(true);
    expect(matchesModelQuery(model, "opus")).toBe(true);
    expect(matchesModelQuery(model, "ANTHROPIC opus")).toBe(true);
    expect(matchesModelQuery(model, "claude-opus")).toBe(true);
    expect(matchesModelQuery(model, "gpt")).toBe(false);
    expect(matchesModelQuery({ name: "X", key: "x", lab: null }, "unknown")).toBe(true);
  });
});
