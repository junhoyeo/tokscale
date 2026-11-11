import { describe, expect, it } from "vitest";
import {
  groupModelsByLab,
  isCatalogFiltered,
  LAB_PREVIEW_SIZE,
  LAB_PREVIEW_SLACK,
  previewLabModels,
  toCatalogModel,
} from "@/components/explore/catalog";
import type { ModelUsageRow } from "@/lib/explore";

function row(key: string, activeUsers: number): ModelUsageRow {
  return {
    key,
    name: key.toUpperCase(),
    lab: "Anthropic",
    labSlug: "anthropic",
    logoUrl: "/anthropic.svg",
    rank: 3,
    activeUsers,
    previousActiveUsers: 2,
    adoption: 0.1,
    tokens: activeUsers * 1000,
    estimatedCost: 12.5,
    isNew: true,
    topAgents: [{ key: "claude", name: "Claude Code", logoUrl: null, share: 1 }],
  };
}

const range = (n: number) => Array.from({ length: n }, (_, i) => i);

describe("previewLabModels", () => {
  it("lists the top models and counts the collapsed tail", () => {
    const models = range(LAB_PREVIEW_SIZE + LAB_PREVIEW_SLACK + 1);
    const preview = previewLabModels(models, true);
    expect(preview.shown).toEqual(range(LAB_PREVIEW_SIZE));
    expect(preview.hidden).toBe(LAB_PREVIEW_SLACK + 1);
  });

  it("lists a lab whole when the tail would be short", () => {
    const models = range(LAB_PREVIEW_SIZE + LAB_PREVIEW_SLACK);
    expect(previewLabModels(models, true)).toEqual({ shown: models, hidden: 0 });
  });

  it("lists everything when collapsing is off", () => {
    const models = range(40);
    expect(previewLabModels(models, false)).toEqual({ shown: models, hidden: 0 });
  });

  it("takes custom sizes and never mutates the input", () => {
    const models = [5, 4, 3, 2, 1];
    const preview = previewLabModels(models, true, 2, 0);
    expect(preview).toEqual({ shown: [5, 4], hidden: 3 });
    preview.shown.push(99);
    expect(models).toEqual([5, 4, 3, 2, 1]);
  });
});

describe("isCatalogFiltered", () => {
  it("is true for a search or a single lab", () => {
    expect(isCatalogFiltered("", "all")).toBe(false);
    expect(isCatalogFiltered("   ", "all")).toBe(false);
    expect(isCatalogFiltered("opus", "all")).toBe(true);
    expect(isCatalogFiltered("", "anthropic")).toBe(true);
  });
});

describe("toCatalogModel", () => {
  it("keeps only the fields the catalog renders", () => {
    expect(toCatalogModel(row("opus", 7))).toEqual({
      key: "opus",
      name: "OPUS",
      lab: "Anthropic",
      labSlug: "anthropic",
      logoUrl: "/anthropic.svg",
      activeUsers: 7,
      adoption: 0.1,
      tokens: 7000,
      estimatedCost: 12.5,
    });
  });

  it("groups slim rows the same way as full rows", () => {
    const rows = [row("a", 1), row("b", 9), row("c", 5)];
    const full = groupModelsByLab(rows, "users").map((g) => g.models.map((m) => m.key));
    const slim = groupModelsByLab(rows.map(toCatalogModel), "users").map((g) => g.models.map((m) => m.key));
    expect(slim).toEqual(full);
    expect(slim).toEqual([["b", "c", "a"]]);
  });
});
