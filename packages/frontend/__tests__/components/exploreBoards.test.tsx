import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentBoard, ModelBoard } from "@/components/explore/UsageBoards";
import { fixtureAgents, fixtureCoverage, fixtureModels } from "@/lib/explore/fixtures";
import type { ModelUsageRow } from "@/lib/explore";

const coverage = fixtureCoverage("30d");

function text(markup: string): string {
  return markup.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
}

describe("ModelBoard", () => {
  it("labels cost as an estimate and links each model page", () => {
    const rows = fixtureModels("30d");
    const markup = renderToStaticMarkup(<ModelBoard rows={rows} coverage={coverage} period="30d" sort="users" />);
    expect(markup).toContain("Est. cost");
    expect(markup).not.toMatch(/>Cost</);
    expect(markup).toContain('href="/models/claude-opus-5"');
    expect(markup).toContain('aria-sort="descending"');
    expect(text(markup)).toContain("add up to");
  });

  it("shows added users and a New badge instead of a percentage for new models", () => {
    const row: ModelUsageRow = { ...fixtureModels("30d")[5], activeUsers: 142, previousActiveUsers: 0, isNew: true };
    const markup = text(renderToStaticMarkup(<ModelBoard rows={[row]} coverage={coverage} period="30d" sort="users" />));
    expect(markup).toContain("New");
    expect(markup).toContain("+142");
    expect(markup).not.toMatch(/\+\d[\d,.]*%/);
  });

  it("shows no change column content for all time", () => {
    const rows = fixtureModels("all");
    const markup = text(renderToStaticMarkup(<ModelBoard rows={rows} coverage={fixtureCoverage("all")} period="all" sort="tokens" />));
    expect(markup).not.toContain("New");
    expect(markup).not.toContain("previous");
  });

  it("renders an empty state", () => {
    const markup = renderToStaticMarkup(<ModelBoard rows={[]} coverage={coverage} period="7d" sort="users" />);
    expect(markup).toContain("No model usage in this period yet.");
  });
});

describe("AgentBoard", () => {
  it("keeps complete long agent names in both responsive layouts", () => {
    const rows = [{ ...fixtureAgents("30d")[0], name: "A long descriptive agent name that needs wrapping" }];
    const markup = renderToStaticMarkup(<AgentBoard rows={rows} coverage={coverage} period="30d" sort="users" />);
    expect(markup.match(/A long descriptive agent name that needs wrapping/g)?.length).toBeGreaterThanOrEqual(2);
    expect(markup).toContain("Client id");
    expect(markup).toContain("Top models:");
    const stacked = markup.slice(markup.indexOf("<ol"));
    for (const model of rows[0].topModels) {
      expect(stacked).toContain(`href="/models/${model.key}"`);
    }
  });

  it("lists top models as links to model pages", () => {
    const markup = renderToStaticMarkup(<AgentBoard rows={fixtureAgents("30d")} coverage={coverage} period="30d" sort="tokens" />);
    expect(markup).toContain("Claude Code");
    expect(markup).toContain('href="/models/claude-opus-5"');
    expect(markup).toContain("Est. cost");
  });
});
