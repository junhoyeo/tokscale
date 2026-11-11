import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import capexJson from "@/content/capital/metrics/capex.json";
import { ledgerContent, validateLedgerContent, LedgerContentError, type RawLedgerContent } from "@/lib/capital/ledger/content";
import { EVENT_FILES, ORG_FILES, STATEMENT_FILES } from "@/lib/capital/ledger/contentIndex.generated";

describe("ledger content", () => {
  const raw = (): RawLedgerContent => ({
    orgFiles: ORG_FILES.map((f) => ({ file: f.file, data: structuredClone(f.data) })),
    eventFiles: EVENT_FILES.map((f) => ({ file: f.file, data: structuredClone(f.data) })),
    statementFiles: STATEMENT_FILES.map((f) => ({ file: f.file, data: structuredClone(f.data) })),
    capex: structuredClone(capexJson),
  });
  type EventFileData = {
    sources: Array<Record<string, unknown> & { id?: string; filing?: Record<string, unknown> }>;
    event: Record<string, unknown> & {
      amounts: Array<Record<string, unknown>>;
      participants: Array<Record<string, unknown>>;
      relations: Array<Record<string, unknown>>;
      title: string;
      status: string;
    };
  };
  const eventFile = (r: RawLedgerContent, file: string) => r.eventFiles.find((f) => f.file === file)!.data as EventFileData;

  it("accepts the shipped content", () => {
    expect(() => validateLedgerContent(raw())).not.toThrow();
  });

  it("rejects a repeated source when a nested filing field differs", () => {
    const broken = raw();
    const source = eventFile(broken, "spacex-ipo-2026.json").sources.find((item) => item.id === "src_spacex_10q_2026_06_30")!;
    source.filing!.form = "10-Q/A";
    expect(() => validateLedgerContent(broken)).toThrow(/source src_spacex_10q_2026_06_30 is defined twice with different fields/);
  });

  it("indexes every content file on disk", () => {
    const dir = join(process.cwd(), "src/content/capital");
    const events = readdirSync(join(dir, "events")).filter((n) => n.endsWith(".json")).sort();
    expect(EVENT_FILES.map((f) => f.file).sort()).toEqual(events);
    const orgFiles = readdirSync(dir).filter((n) => /^orgs(?:-[a-z0-9-]+)?\.json$/.test(n)).sort();
    expect(ORG_FILES.map((f) => f.file).sort()).toEqual(orgFiles);
    const statements = readdirSync(join(dir, "metrics")).filter((n) => /^statements(?:-[a-z0-9-]+)?\.json$/.test(n)).sort();
    expect(STATEMENT_FILES.map((f) => f.file).sort()).toEqual(statements);
    expect(readFileSync(join(process.cwd(), "src/lib/capital/ledger/contentIndex.generated.ts"), "utf8")).toContain("Do not edit by hand");
  });

  it("rejects an amount without a quote or with an unknown source", () => {
    const noQuote = raw();
    eventFile(noQuote, "anthropic-series-h-2026.json").event.amounts[0].quote = "";
    expect(() => validateLedgerContent(noQuote)).toThrow(LedgerContentError);
    const unknownSource = raw();
    eventFile(unknownSource, "anthropic-series-h-2026.json").event.amounts[0].sourceId = "src_nowhere";
    expect(() => validateLedgerContent(unknownSource)).toThrow(/src_nowhere/);
  });

  it("rejects null values unless undisclosed, and zero is never a stand-in", () => {
    const broken = raw();
    eventFile(broken, "anthropic-series-h-2026.json").event.amounts[0].value = null;
    expect(() => validateLedgerContent(broken)).toThrow(/undisclosed/);
  });

  it("ties the reported status to news sources", () => {
    const broken = raw();
    eventFile(broken, "anthropic-series-h-2026.json").event.amounts[0].status = "reported";
    expect(() => validateLedgerContent(broken)).toThrow(/news/);
  });

  it("requires an includes relation on the event that already counts an amount", () => {
    const broken = raw();
    eventFile(broken, "anthropic-series-h-2026.json").event.relations = [];
    expect(() => validateLedgerContent(broken)).toThrow(/includes/);
  });

  it("rejects duplicate ids and slugs, unknown participants and rumor wording", () => {
    const dup = raw();
    dup.eventFiles = [...dup.eventFiles, { file: "anthropic-series-h-2026.json", data: structuredClone(dup.eventFiles.find((f) => f.file === "anthropic-series-h-2026.json")!.data) }];
    expect(() => validateLedgerContent(dup)).toThrow(/duplicate/);
    const ghost = raw();
    eventFile(ghost, "anthropic-series-h-2026.json").event.participants.push({ org: "org_nobody", roles: ["investor"] });
    expect(() => validateLedgerContent(ghost)).toThrow(/org_nobody/);
    const rumor = raw();
    eventFile(rumor, "anthropic-series-h-2026.json").event.title = "Anthropic in talks to raise";
    expect(() => validateLedgerContent(rumor)).toThrow(/rumor/);
    const rumored = raw();
    eventFile(rumored, "anthropic-series-h-2026.json").event.status = "rumored";
    expect(() => validateLedgerContent(rumored)).toThrow();
  });

  it("gives every amount, quantity and fact a quote and a cited source", () => {
    for (const event of ledgerContent.events.values()) {
      expect(event.sourceIds.length).toBeGreaterThan(0);
      for (const item of [...event.amounts, ...event.quantities, ...event.facts]) {
        expect(item.quote.trim().length, `${event.id}`).toBeGreaterThanOrEqual(8);
        expect(event.sourceIds, `${event.id}`).toContain(item.sourceId);
        expect(ledgerContent.sources.has(item.sourceId)).toBe(true);
      }
    }
    for (const statement of ledgerContent.statements) {
      expect(ledgerContent.sources.has(statement.sourceId)).toBe(true);
      expect(statement.quote.length).toBeGreaterThanOrEqual(8);
    }
  });

  it("keeps ids and slugs unique across orgs and events", () => {
    const orgSlugs = [...ledgerContent.orgs.values()].map((o) => o.slug);
    expect(new Set(orgSlugs).size).toBe(orgSlugs.length);
    const eventSlugs = [...ledgerContent.events.values()].map((e) => e.slug);
    expect(new Set(eventSlugs).size).toBe(eventSlugs.length);
    // The owner-approved seed list: 33 companies and 16 investors. Other orgs are named participants of events.
    expect([...ledgerContent.orgs.values()].filter((o) => o.seed && o.kinds.includes("company"))).toHaveLength(33);
    const seedInvestors = [
      "amazon", "alphabet", "microsoft", "nvidia", "softbank-group", "sequoia-capital", "andreessen-horowitz",
      "lightspeed-venture-partners", "iconiq", "coatue", "thrive-capital", "founders-fund", "gic", "mgx", "fidelity", "ark-venture-fund",
    ];
    for (const slug of seedInvestors) {
      const org = [...ledgerContent.orgs.values()].find((o) => o.slug === slug);
      expect(org?.seed && org.kinds.includes("investor"), slug).toBe(true);
    }
  });
});
