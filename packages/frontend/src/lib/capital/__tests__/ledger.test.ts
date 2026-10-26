import { existsSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import nextConfig from "../../../../next.config";
import { quarterLabel } from "@/lib/capital";
import { CAPITAL_REDIRECTS } from "@/lib/capital/redirects";
import { capitalSitemapEntries } from "@/lib/capital/sitemap";
import { ledger, total } from "@/lib/capital/ledger";
import {
  DEFAULT_DIRECTORY_PARAMS,
  directoryQuery,
  filterDirectory,
  matchesQuery,
  parseDirectoryParams,
  sortDirectory,
  type DirectoryRowLike,
} from "@/lib/capital/ledger/directory";
import { usageForLabs, type UsageSnapshot } from "@/lib/capital/ledger/usage";
import { companySlugForLab } from "@/lib/companyLinks";

describe("ledger views", () => {
  it("counts Amazon's April 2026 $5B once: inside Series H, never on its own", () => {
    const amazon = ledger.getEvent("anthropic-amazon-2026-04")!;
    const investment = amazon.amounts.find((a) => a.amount.id === "investment")!;
    expect(investment.amount.includedIn).toBe("evt_anthropic_series_h_2026");
    expect(investment.skipped).toBe("included_in");
    const series = ledger.getEvent("anthropic-series-h-2026")!;
    const all = [...amazon.amounts, ...series.amounts].map((a) => a.amount).filter((a) => a.measure === "equity_investment");
    // Amazon's $5B (included in Series H), the $15B and Amazon's $5B inside Series H (parts of the $65B): nothing new to add.
    expect(total(all, { measure: "equity_investment", currency: "USD" }).value).toBeNull();
    const equity = ledger.equityRaised("org_anthropic").totals.find((t) => t.currency === "USD")!;
    expect(equity.counted.every((a) => a.measure === "equity_raised" && a.status === "closed")).toBe(true);
    expect(equity.counted.some((a) => a.value === 65_000_000_000)).toBe(true);
  });

  it("routes financing to rounds and deals to deal pages", () => {
    expect(ledger.getEvent("anthropic-series-h-2026")!.href).toBe("/capital/rounds/anthropic-series-h-2026");
    expect(ledger.getEvent("akamai-anthropic-2026")!.href).toBe("/capital/deals/akamai-anthropic-2026");
    expect(ledger.getEvent("crusoe-form-d-2026")!.href).toBe("/capital/rounds/crusoe-form-d-2026");
    expect(ledger.getEvent("xai-spacex-merger-2026")!.isCompute).toBe(false);
    expect(ledger.getEvent("terawulf-anthropic-lease-2026")!.isCompute).toBe(true);
  });

  it("picks the largest valuation from a primary source", () => {
    const top = ledger.largestValuation()!;
    expect(top.amount.amount.measure).toBe("valuation_post");
    expect(top.event.primary).toBe(true);
    expect(top.amount.amount.value).toBeGreaterThanOrEqual(965_000_000_000);
  });

  it("adds capex only inside one aligned calendar quarter and one currency", () => {
    const q = ledger.capexCalendarQuarter()!;
    expect(q.key).toMatch(/^\d{4}-q[1-4]$/);
    expect(q.label).toBe(quarterLabel(q.key));
    expect(quarterLabel("2026-q2")).toBe("Q2 2026");
    expect(q.entries.every((e) => e.quarter.calendarQuarter === q.key && e.quarter.currency === "USD")).toBe(true);
    expect(q.total.value).toBe(q.entries.reduce((sum, e) => sum + e.quarter.value, 0));
    expect(q.derivedCount).toBe(q.entries.filter((e) => e.quarter.derived).length);
  });

  it("never adds compute values of different measures", () => {
    const year = ledger.computeYear("2026");
    if (!year.summed.ok) expect(year.summed.reason.length).toBeGreaterThan(0);
    else expect(new Set(year.summed.total.counted.map((a) => a.measure)).size).toBe(1);
  });

  it("computes a step-up only between sourced valuations", () => {
    for (const e of ledger.getRounds()) {
      const step = ledger.stepUp(e);
      if (!step) continue;
      expect(step.ratio).toBeCloseTo(e.valuation!.amount.value! / step.previous.valuation!.amount.value!, 10);
      expect(step.previous.date < e.date).toBe(true);
    }
  });

  it("gives every company a profile and only sourced profiles are indexable", () => {
    for (const org of ledger.getCompanies()) {
      const profile = ledger.getCompany(org.slug)!;
      expect(profile.org.id).toBe(org.id);
      expect(profile.indexable).toBe(profile.factCount > 0);
    }
    expect(ledger.getCompany("anthropic")!.indexable).toBe(true);
    // Full-year cash capex from 20-F filings is a sourced fact even without quarters.
    expect(ledger.getCompany("tsmc")!.capex?.quarters.length).toBe(0);
    expect(ledger.getCompany("tsmc")!.indexable).toBe(true);
  });
});

describe("lab links", () => {
  it("maps Tokscale lab slugs to company pages through orgs.json", () => {
    expect(companySlugForLab("anthropic")).toBe("anthropic");
    expect(companySlugForLab("google")).toBe("alphabet");
    expect(companySlugForLab("xai")).toBe("xai");
    expect(companySlugForLab("mistral")).toBe("mistral-ai");
    expect(companySlugForLab("moonshot-ai")).toBe("moonshot-ai");
    expect(companySlugForLab("openai")).toBe("openai");
    expect(companySlugForLab("stepfun")).toBeNull();
    expect(companySlugForLab(null)).toBeNull();
  });
});

describe("old capital URLs", () => {
  const sources = new Set(CAPITAL_REDIRECTS.map((r) => r.source));

  it("send the removed 13F pages to the nearest live page, permanently", () => {
    for (const r of CAPITAL_REDIRECTS) {
      expect(r.permanent).toBe(true);
      expect(["/capital/investors", "/capital/companies"]).toContain(r.destination);
    }
    for (const source of [
      "/capital/portfolios",
      "/capital/funds/:path*",
      "/capital/investors/:slug/13f/:path*",
      "/capital/investors/thiel-macro",
      "/capital/investors/pershing-square",
      "/capital/companies/netflix",
    ]) {
      expect(sources.has(source), source).toBe(true);
    }
  });

  it("never hide a live company, investor or route", () => {
    for (const org of ledger.getCompanies()) expect(sources.has(`/capital/companies/${org.slug}`), org.slug).toBe(false);
    for (const row of ledger.getInvestors()) expect(sources.has(`/capital/investors/${row.org.slug}`), row.org.slug).toBe(false);
    const app = path.join(process.cwd(), "src/app/(main)/capital");
    for (const dir of ["portfolios", "funds", "investors/[slug]/13f"]) expect(existsSync(path.join(app, dir)), dir).toBe(false);
  });

  it("are served by next.config", async () => {
    const redirects = await nextConfig.redirects!();
    for (const r of CAPITAL_REDIRECTS) expect(redirects).toContainEqual(r);
  });
});

describe("capital sitemap (v2)", () => {
  it("lists v2 pages and leaves out pages without a primary source", () => {
    const urls = capitalSitemapEntries().map((e) => e.url);
    for (const path of ["/capital/companies", "/capital/investors", "/capital/rounds", "/capital/compute", "/capital/spending", "/capital/methodology", "/capital/sectors/ai-labs"]) {
      expect(urls.some((u) => u.endsWith(path))).toBe(true);
    }
    expect(urls.some((u) => u.endsWith("/capital/rounds/anthropic-series-h-2026"))).toBe(true);
    expect(urls.some((u) => u.endsWith("/capital/companies/anthropic"))).toBe(true);
    for (const e of ledger.getEvents().filter((x) => !x.indexable)) expect(urls.some((u) => u.endsWith(e.href))).toBe(false);
    for (const org of ledger.getCompanies()) {
      const p = ledger.getCompany(org.slug)!;
      if (!p.indexable) expect(urls.some((u) => u.endsWith(`/capital/companies/${org.slug}`))).toBe(false);
    }
    expect(urls.some((u) => u.includes("?"))).toBe(false);
  });
});

describe("directory URL state", () => {
  const row = (over: Partial<DirectoryRowLike>): DirectoryRowLike => ({
    name: "Org",
    aliases: [],
    legalNames: [],
    tickers: [],
    categories: ["ai-lab"],
    country: "US",
    status: "private",
    lastActivity: null,
    valuationUsd: null,
    equityUsd: null,
    capexUsd: null,
    dealsCount: 0,
    roundsCount: 0,
    modelsCount: null,
    ...over,
  });

  it("parses and rebuilds query strings, dropping unknown values", () => {
    const params = parseDirectoryParams({ q: " zhipu ", category: "ai-lab", country: "cn", view: "nope", sort: "valuation", has: "models,bogus" });
    expect(params).toEqual({ ...DEFAULT_DIRECTORY_PARAMS, q: "zhipu", category: "ai-lab", country: "CN", sort: "valuation", has: ["models"] });
    expect(directoryQuery(params)).toBe("?q=zhipu&category=ai-lab&country=CN&has=models&sort=valuation");
    expect(directoryQuery(DEFAULT_DIRECTORY_PARAMS)).toBe("");
  });

  it("searches names, legal names, aliases and tickers", () => {
    const zai = row({ name: "Z.ai", aliases: ["Zhipu", "Knowledge Atlas"] });
    expect(matchesQuery(zai, "zhipu")).toBe(true);
    expect(matchesQuery(zai, "knowledge atlas")).toBe(true);
    expect(matchesQuery(row({ name: "Cursor", legalNames: ["Anysphere, Inc."] }), "anysphere")).toBe(true);
    expect(matchesQuery(row({ name: "CoreWeave", tickers: ["CRWV"] }), "crwv")).toBe(true);
    expect(matchesQuery(row({ name: "OpenAI" }), "anthropic")).toBe(false);
  });

  it("filters and sorts without inventing values", () => {
    const rows = [
      row({ name: "B", valuationUsd: 10, dealsCount: 2 }),
      row({ name: "A", valuationUsd: null, country: "CN" }),
      row({ name: "C", valuationUsd: 30, categories: ["neocloud"] }),
    ];
    expect(sortDirectory(rows, "valuation").map((r) => r.name)).toEqual(["C", "B", "A"]);
    expect(sortDirectory(rows, "name").map((r) => r.name)).toEqual(["A", "B", "C"]);
    expect(filterDirectory(rows, { ...DEFAULT_DIRECTORY_PARAMS, has: ["compute"] }).map((r) => r.name)).toEqual(["B"]);
    expect(filterDirectory(rows, { ...DEFAULT_DIRECTORY_PARAMS, country: "CN" }).map((r) => r.name)).toEqual(["A"]);
    expect(filterDirectory(rows, { ...DEFAULT_DIRECTORY_PARAMS, category: "neocloud" }).map((r) => r.name)).toEqual(["C"]);
  });
});

describe("Tokscale usage on lab profiles", () => {
  const snapshot = {
    rows: [
      { key: "claude-opus-5", labSlug: "anthropic", activeUsers: 10, tokens: 600 },
      { key: "claude-haiku-5", labSlug: "anthropic", activeUsers: 0, tokens: 0 },
      { key: "gemini-3", labSlug: "google", activeUsers: 5, tokens: 400 },
    ],
    coverage: { period: "30d", start: "2026-09-20", end: "2026-10-19", accounts: 20, modelIdentifiedShare: 1, lastDataAt: null },
    totalTokens: 1000,
  } as unknown as UsageSnapshot;

  it("keeps the lab's models and its share of identified tokens", () => {
    const usage = usageForLabs(snapshot, ["anthropic"])!;
    expect(usage.models.map((m) => m.key)).toEqual(["claude-opus-5", "claude-haiku-5"]);
    expect(usage.activeModels).toBe(1);
    expect(usage.share).toBe(0.6);
    expect(usageForLabs(snapshot, [])).toBeNull();
  });
});

describe("event and profile display", () => {
  it("leads a Form D with the amount sold, not the total offering size", () => {
    const crusoe = ledger.getEvent("crusoe-form-d-2026")!;
    expect(crusoe.headline?.amount?.amount.measure).toBe("form_d_amount_sold");
    expect(crusoe.headline?.amount?.amount.value).toBe(3_108_543_889);
  });

  it("lists deal counterparties on a profile, not the investors of its rounds", () => {
    const anthropic = ledger.getCompany("anthropic")!;
    const names = anthropic.counterparties.map((o) => o.slug);
    expect(names).toContain("akamai");
    expect(names).toContain("terawulf");
    expect(names).not.toContain("altimeter-capital");
    expect(anthropic.investors.some((i) => i.org.slug === "altimeter-capital")).toBe(true);
  });

  it("lists only investors in the company's own rounds and deals", () => {
    const anthropic = ledger.getCompany("anthropic")!;
    expect(anthropic.investors.find((i) => i.org.slug === "amazon")?.roles).toContain("strategic_investor");
    const microsoft = ledger.getCompany("microsoft")!;
    // SoftBank, NVIDIA and Amazon invested in OpenAI rounds that Microsoft also joined; they are not Microsoft's investors.
    for (const slug of ["softbank-group", "nvidia", "amazon"]) expect(microsoft.investors.some((i) => i.org.slug === slug)).toBe(false);
  });
});
