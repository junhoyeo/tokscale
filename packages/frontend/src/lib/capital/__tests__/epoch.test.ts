import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import dataCentersJson from "@/content/capital/inbox/epoch-data-centers.json";
import fundingJson from "@/content/capital/inbox/epoch-funding-rounds.json";
import {
  EPOCH_INBOX_NOTE,
  buildOrgIndex,
  csvRecords,
  epochDataCenterInboxSchema,
  epochFundingInboxSchema,
  findMatchingEvents,
  mapOrgName,
  markdownLinks,
  matchEvent,
  normalizeDataCenterRow,
  normalizeFundingRow,
  parseCsv,
  parseEpochDate,
  parseEpochNumber,
  parseEpochScaled,
  parseEpochUpdated,
  proposeAmounts,
  splitTaggedNames,
  type EpochDataCenterRow,
  type EpochFundingRow,
} from "@/lib/capital/ledger/epoch";
import { eventFileSchema, orgsFileSchema, type CapitalEvent } from "@/lib/capital/ledger/schema";

const CONTENT = fileURLToPath(new URL("../../../content/capital/", import.meta.url));

const ORGS = [
  { id: "org_zai", name: "Z.ai", legalNames: ["Knowledge Atlas Technology Joint Stock Co., Ltd."], aliases: ["Zhipu", "Zhipu AI"] },
  { id: "org_alphabet", name: "Alphabet", legalNames: ["Alphabet Inc."], aliases: ["Google", "Google DeepMind"] },
  { id: "org_anthropic", name: "Anthropic", legalNames: ["Anthropic PBC"], aliases: [] },
  { id: "org_softbank_group", name: "SoftBank Group", legalNames: ["SoftBank Group Corp."], aliases: [] },
  { id: "org_meta", name: "Meta", legalNames: [], aliases: ["Shared Name"] },
  { id: "org_microsoft", name: "Microsoft", legalNames: [], aliases: ["Shared Name"] },
];
const index = buildOrgIndex(ORGS);

const event = (overrides: Partial<CapitalEvent> & Pick<CapitalEvent, "id" | "date">): CapitalEvent => ({
  slug: overrides.id.replace(/^evt_/, "").replace(/_/g, "-"),
  kind: "financing",
  types: ["equity_round"],
  title: "Test round",
  summary: "Test round.",
  dates: { announced: overrides.date },
  status: "closed",
  participants: [{ org: "org_anthropic", roles: ["issuer"] }],
  amounts: [],
  quantities: [],
  facts: [],
  relations: [],
  sourceIds: ["src_test"],
  reviewedAt: "2026-10-21",
  corrections: [],
  ...overrides,
});

const equity = (id: string, value: number, extra: Record<string, unknown> = {}) => ({
  id,
  measure: "equity_raised" as const,
  value,
  currency: "USD",
  basis: "exact" as const,
  status: "closed" as const,
  asOf: "2026-05-28",
  sourceId: "src_test",
  quote: "a quote copied from the source",
  ...extra,
});

describe("parseEpochNumber", () => {
  it("returns null for a blank cell, never 0", () => {
    expect(parseEpochNumber("")).toBeNull();
    expect(parseEpochNumber("   ")).toBeNull();
    expect(parseEpochNumber(undefined)).toBeNull();
    expect(parseEpochNumber(null)).toBeNull();
  });
  it("keeps Epoch's explicit zero and parses plain decimals", () => {
    expect(parseEpochNumber("0.0")).toBe(0);
    expect(parseEpochNumber("100000000000.0")).toBe(100_000_000_000);
    expect(parseEpochNumber(" 6600000000.0 ")).toBe(6_600_000_000);
    expect(parseEpochNumber("1.5e9")).toBe(1_500_000_000);
  });
  it("throws on text that is not a plain number", () => {
    expect(() => parseEpochNumber("1,000")).toThrow();
    expect(() => parseEpochNumber("$5B")).toThrow();
    expect(() => parseEpochNumber("n/a")).toThrow();
  });
});

describe("parseEpochScaled", () => {
  it("scales billions to whole dollars without float noise", () => {
    expect(parseEpochScaled("35.836372", 9)).toBe(35_836_372_000);
    expect(parseEpochScaled("39.283634", 9)).toBe(39_283_634_000);
    expect(parseEpochScaled("2", 9)).toBe(2_000_000_000);
    expect(parseEpochScaled("1.2345678905", 9)).toBe(1_234_567_891);
  });
  it("returns null for a blank cell", () => {
    expect(parseEpochScaled("", 9)).toBeNull();
  });
});

describe("dates", () => {
  it("parses Epoch's YYYY-MM-DD cells and rejects other formats", () => {
    expect(parseEpochDate("2026-01-08")).toBe("2026-01-08");
    expect(parseEpochDate("")).toBeNull();
    expect(() => parseEpochDate("Jan 8, 2026")).toThrow();
  });
  it("reads the Updated badge", () => {
    expect(parseEpochUpdated("Updated Oct. 8, 2026")).toBe("2026-10-08");
    expect(parseEpochUpdated("Updated Oct. 20, 2026")).toBe("2026-10-20");
    expect(parseEpochUpdated("Updated Sept. 30, 2026")).toBe("2026-09-30");
    expect(parseEpochUpdated("Updated May 1, 2026")).toBe("2026-05-01");
    expect(() => parseEpochUpdated("Updated recently")).toThrow();
  });
});

describe("CSV", () => {
  it("handles quotes, commas, doubled quotes, line breaks, CRLF and a BOM", () => {
    const text = '\uFEFFId,Notes,Value\r\nA,"He said ""yes"", then\nleft",1\r\nB,,\r\n';
    expect(parseCsv(text)).toEqual([
      ["Id", "Notes", "Value"],
      ["A", 'He said "yes", then\nleft', "1"],
      ["B", "", ""],
    ]);
    expect(csvRecords(text, ["Id", "Value"])).toEqual([
      { Id: "A", Notes: 'He said "yes", then\nleft', Value: "1" },
      { Id: "B", Notes: "", Value: "" },
    ]);
  });
  it("fails on a missing column or an open quote", () => {
    expect(() => csvRecords("Id\nA\n", ["Id", "Status"])).toThrow(/missing columns: Status/);
    expect(() => parseCsv('Id\n"open\n')).toThrow();
  });
});

describe("markdownLinks", () => {
  it("extracts the URLs of a markdown list, with parentheses in URLs", () => {
    const text =
      "- [WSJ profile](https://www.wsj.com/tech/a-b)\u00a0\n- [Wiki](https://en.wikipedia.org/wiki/X_(company))\n- [Again](https://www.wsj.com/tech/a-b)";
    expect(markdownLinks(text)).toEqual(["https://www.wsj.com/tech/a-b", "https://en.wikipedia.org/wiki/X_(company)"]);
  });
});

describe("org mapping", () => {
  it("maps names, aliases and 'Name (Alias)' forms", () => {
    expect(mapOrgName("Z.ai (Zhipu)", index)).toBe("org_zai");
    expect(mapOrgName("Google", index)).toBe("org_alphabet");
    expect(mapOrgName("Google DeepMind", index)).toBe("org_alphabet");
    expect(mapOrgName("anthropic", index)).toBe("org_anthropic");
  });
  it("does not guess", () => {
    expect(mapOrgName("Softbank", index)).toBeNull();
    expect(mapOrgName("SpaceXAI", index)).toBeNull();
    expect(mapOrgName("Shared Name", index)).toBeNull();
  });
  it("splits Epoch's tagged name lists", () => {
    expect(splitTaggedNames("OpenAI #likely, Microsoft #likely")).toEqual([
      { name: "OpenAI", tag: "likely" },
      { name: "Microsoft", tag: "likely" },
    ]);
    expect(splitTaggedNames("Google")).toEqual([{ name: "Google", tag: null }]);
  });
});

describe("proposeAmounts", () => {
  const closed = { status: "Closed", type: "Primary", equityUsd: 65e9, debtUsd: null, postMoneyUsd: 965e9 };
  it("proposes equity and post-money for a closed primary round", () => {
    expect(proposeAmounts(closed)).toEqual([
      { measure: "equity_raised", value: 65e9, currency: "USD", basis: "exact" },
      { measure: "valuation_post", value: 965e9, currency: "USD", basis: "exact" },
    ]);
  });
  it("proposes debt whenever Epoch lists it", () => {
    expect(proposeAmounts({ ...closed, type: "Primary,Debt", debtUsd: 140e6 }).map((a) => a.measure)).toEqual([
      "equity_raised",
      "debt_raised",
      "valuation_post",
    ]);
  });
  it("proposes nothing for tenders, market caps, cancelled deals or talks", () => {
    expect(proposeAmounts({ ...closed, type: "Secondary", equityUsd: 0, postMoneyUsd: 500e9 })).toEqual([]);
    expect(proposeAmounts({ ...closed, type: "Other", equityUsd: null, postMoneyUsd: 39.4e9 })).toEqual([]);
    expect(proposeAmounts({ ...closed, status: "Cancelled" })).toEqual([]);
    expect(proposeAmounts({ ...closed, status: "Late discussions", type: null })).toEqual([]);
  });
  it("never proposes a zero or a blank", () => {
    expect(proposeAmounts({ ...closed, equityUsd: 0, postMoneyUsd: null })).toEqual([]);
  });
});

describe("matchEvent", () => {
  const row = { org: "org_anthropic", closeDate: "2026-05-28", reportDate: null, equityUsd: 65e9 };
  const seriesH = event({ id: "evt_anthropic_series_h_2026", date: "2026-05-28", amounts: [equity("equity", 65e9)] });

  it("matches the same issuer, a date within 7 days and the same equity amount", () => {
    expect(matchEvent(row, [seriesH])).toBe("evt_anthropic_series_h_2026");
    expect(matchEvent({ ...row, closeDate: "2026-06-04" }, [seriesH])).toBe("evt_anthropic_series_h_2026");
  });
  it("uses the report date when the close date is blank", () => {
    expect(matchEvent({ ...row, closeDate: null, reportDate: "2026-05-30" }, [seriesH])).toBe("evt_anthropic_series_h_2026");
  });
  it("counts a strategic investment amount as equity", () => {
    const amazon = event({
      id: "evt_anthropic_amazon_2026_04",
      date: "2026-04-20",
      kind: "deal",
      amounts: [equity("investment", 5e9, { measure: "equity_investment", includedIn: "evt_anthropic_series_h_2026" })],
    });
    expect(matchEvent({ ...row, closeDate: "2026-04-20", equityUsd: 5e9 }, [amazon, seriesH])).toBe("evt_anthropic_amazon_2026_04");
  });
  it("does not match another date, amount, issuer, or a part of a larger amount", () => {
    expect(matchEvent({ ...row, closeDate: "2026-06-05" }, [seriesH])).toBeNull();
    expect(matchEvent({ ...row, equityUsd: 30e9 }, [seriesH])).toBeNull();
    expect(matchEvent({ ...row, org: "org_openai" }, [seriesH])).toBeNull();
    expect(matchEvent({ ...row, org: null }, [seriesH])).toBeNull();
    expect(matchEvent({ ...row, equityUsd: null }, [seriesH])).toBeNull();
    const withPart = event({
      id: "evt_anthropic_series_h_2026",
      date: "2026-05-28",
      amounts: [equity("equity", 65e9), equity("amazon", 5e9, { measure: "equity_investment", partOf: "equity" })],
    });
    expect(matchEvent({ ...row, equityUsd: 5e9 }, [withPart])).toBeNull();
    const investorOnly = event({
      id: "evt_other_round_2026",
      date: "2026-05-28",
      participants: [{ org: "org_anthropic", roles: ["investor"] }],
      amounts: [equity("equity", 65e9)],
    });
    expect(matchEvent(row, [investorOnly])).toBeNull();
  });
  it("returns null when two events match", () => {
    const twin = event({ id: "evt_anthropic_series_h_2026_copy", date: "2026-05-29", amounts: [equity("equity", 65e9)] });
    expect(findMatchingEvents(row, [seriesH, twin])).toEqual(["evt_anthropic_series_h_2026", "evt_anthropic_series_h_2026_copy"]);
    expect(matchEvent(row, [seriesH, twin])).toBeNull();
  });
});

describe("row normalization", () => {
  it("keeps blanks as null and Epoch's words as written", () => {
    const [record] = csvRecords(
      'Id,Company,Close date,Status,Funding (equity),Funding (debt),Valuation (post-money),Report date,Source 1,Source 2,Source 3,Type,Confidence\n' +
        'Z.ai IPO,Z.ai (Zhipu),2026-01-08,Closed,558000000.0,,6600000000.0,2026-01-07,https://example.com/a, ,,Primary,Confident\n'
    );
    const { row, warnings } = normalizeFundingRow(record, index);
    expect(warnings).toEqual([]);
    expect(row).toMatchObject({
      epochId: "Z.ai IPO",
      company: "Z.ai (Zhipu)",
      org: "org_zai",
      equityUsd: 558_000_000,
      debtUsd: null,
      postMoneyUsd: 6_600_000_000,
      sources: ["https://example.com/a"],
      review: { state: "unreviewed", event: null },
    });
    expect(row.proposed.map((a) => a.measure)).toEqual(["equity_raised", "valuation_post"]);
  });
  it("maps a data center owner only when it is clear", () => {
    const record = {
      Name: "Test DC",
      Owner: "Google #confident",
      Users: "Google DeepMind #speculative",
      Project: "",
      Country: "United States",
      "Current power (MW)": "120",
      "Current H100 equivalents": "",
      "Current total capital cost (2025 USD billions)": "4.5",
      "Selected Sources": "- [Permit](https://example.com/permit)",
      "Calculations sheet": "https://docs.google.com/spreadsheets/d/x",
    };
    const { row } = normalizeDataCenterRow(record, index);
    expect(row).toMatchObject({
      org: "org_alphabet",
      owner: "Google #confident",
      users: "Google DeepMind #speculative",
      project: null,
      currentPowerMw: 120,
      h100Equivalents: null,
      estimatedCapitalCost2025Usd: 4_500_000_000,
      sources: ["https://example.com/permit"],
    });
    expect(normalizeDataCenterRow({ ...record, Owner: "Softbank #confident" }, index).row.org).toBeNull();
  });
});

/* --------------------------------------------------------- inbox files */

const fundingParsed = epochFundingInboxSchema.safeParse(fundingJson);
const dataCentersParsed = epochDataCenterInboxSchema.safeParse(dataCentersJson);
const funding = fundingParsed.success ? fundingParsed.data : { rows: [] as EpochFundingRow[] };
const dataCenters = dataCentersParsed.success ? dataCentersParsed.data : { rows: [] as EpochDataCenterRow[] };

describe("inbox files", () => {
  it("both inbox files parse and carry the license, attribution and the review note", () => {
    expect(fundingParsed.success ? "ok" : fundingParsed.error.issues[0]).toBe("ok");
    expect(dataCentersParsed.success ? "ok" : dataCentersParsed.error.issues[0]).toBe("ok");
    if (!fundingParsed.success || !dataCentersParsed.success) return;
    for (const file of [fundingParsed.data, dataCentersParsed.data]) {
      expect(file.license).toBe("CC BY 4.0");
      expect(file.attribution).toMatch(/epoch\.ai/);
      expect(file.note).toBe(EPOCH_INBOX_NOTE);
      expect(file.rows.length).toBeGreaterThan(0);
    }
  });

  it("no row has 0 where Epoch's cell was blank", () => {
    const check = (value: number | null, cell: string, scaled = false) => {
      if (cell.trim() === "") expect(value).toBeNull();
      else expect(value).toBe(scaled ? parseEpochScaled(cell, 9) : parseEpochNumber(cell));
    };
    for (const row of funding.rows) {
      check(row.equityUsd, row.epochCells["Funding (equity)"]);
      check(row.debtUsd, row.epochCells["Funding (debt)"]);
      check(row.postMoneyUsd, row.epochCells["Valuation (post-money)"]);
      for (const amount of row.proposed) expect(amount.value).toBeGreaterThan(0);
    }
    for (const row of dataCenters.rows) {
      check(row.currentPowerMw, row.epochCells["Current power (MW)"]);
      check(row.h100Equivalents, row.epochCells["Current H100 equivalents"]);
      check(row.estimatedCapitalCost2025Usd, row.epochCells["Current total capital cost (2025 USD billions)"], true);
    }
  });

  it("proposed amounts follow the rules in proposeAmounts", () => {
    for (const row of funding.rows) expect(row.proposed).toEqual(proposeAmounts(row));
  });

  it("every org id exists in the org files", () => {
    const orgIds = new Set(
      readdirSync(CONTENT)
        .filter((name) => /^orgs(?:-[a-z0-9-]+)?\.json$/.test(name))
        .flatMap((name) => orgsFileSchema.parse(JSON.parse(readFileSync(join(CONTENT, name), "utf8"))).orgs.map((o) => o.id))
    );
    for (const row of [...funding.rows, ...dataCenters.rows]) if (row.org) expect(orgIds.has(row.org), row.org).toBe(true);
  });

  it("a published row points at an event that still matches it", () => {
    const published = funding.rows.filter((row) => row.review.state === "published");
    if (published.length === 0) return;
    const events = readdirSync(join(CONTENT, "events"))
      .filter((name) => name.endsWith(".json"))
      .map((name) => eventFileSchema.safeParse(JSON.parse(readFileSync(join(CONTENT, "events", name), "utf8"))))
      .flatMap((parsed) => (parsed.success ? [parsed.data.event] : []));
    for (const row of published) expect(matchEvent(row, events), row.epochId).toBe(row.review.event);
    for (const row of dataCenters.rows) expect(row.review.state).toBe("unreviewed");
  });
});
