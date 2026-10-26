import { describe, expect, it } from "vitest";
import capexJson from "@/content/capital/metrics/capex.json";
import {
  calendarQuarterOf,
  calendarSpanLabel,
  capexPeriods,
  deriveCapex,
  fiscalLabel,
  formatCapexAmount,
  isFiscalYearStart,
  secFilingIndexUrl,
  spanMonths,
} from "@/lib/capital/ledger/capex";
import { capexFileSchema, type CapexFact, type CapexSeries } from "@/lib/capital/ledger/schema";

/* ------------------------------------------------------------ fixtures */

let counter = 0;
function fact(start: string, end: string, value: number, extra: Partial<CapexFact> = {}): CapexFact {
  counter += 1;
  return {
    concept: "us-gaap:PaymentsToAcquirePropertyPlantAndEquipment",
    start,
    end,
    value,
    form: "10-Q",
    accession: `0000000001-26-${String(counter).padStart(6, "0")}`,
    filed: "2026-08-01",
    ...extra,
  };
}

function series(facts: CapexFact[], fiscalYearEnd = "12-31"): CapexSeries {
  return { org: "org_test", cik: "0000000001", currency: "USD", fiscalYearEnd, facts, notes: [] };
}

const summary = (s: CapexSeries) =>
  deriveCapex(s).quarters.map((q) => [fiscalLabel(q), q.start, q.end, q.value, q.derived]);

/* ------------------------------------------------------- synthetic data */

describe("deriveCapex on synthetic series", () => {
  it("turns a direct Q1 and 6, 9 and 12 month facts into four quarters, Q2 to Q4 derived", () => {
    const q1 = fact("2025-01-01", "2025-03-31", 100);
    const h1 = fact("2025-01-01", "2025-06-30", 250);
    const m9 = fact("2025-01-01", "2025-09-30", 450);
    const fy = fact("2025-01-01", "2025-12-31", 700, { form: "10-K" });
    const view = deriveCapex(series([q1, h1, m9, fy]));

    expect(summary(series([q1, h1, m9, fy]))).toEqual([
      ["FY25 Q1", "2025-01-01", "2025-03-31", 100, false],
      ["FY25 Q2", "2025-04-01", "2025-06-30", 150, true],
      ["FY25 Q3", "2025-07-01", "2025-09-30", 200, true],
      ["FY25 Q4", "2025-10-01", "2025-12-31", 250, true],
    ]);
    expect(view.quarters.map((q) => q.calendarQuarter)).toEqual(["2025-q1", "2025-q2", "2025-q3", "2025-q4"]);
    expect(view.quarters[0].formula).toBeNull();
    expect(view.quarters[1].formula).toBe("Six months to Jun 30, 2025 ($250) minus three months to Mar 31, 2025 ($100)");
    expect(view.quarters[3].formula).toBe("Twelve months to Dec 31, 2025 ($700) minus nine months to Sep 30, 2025 ($450)");
    expect(view.quarters[1].sources.map((s) => s.accession)).toEqual([h1.accession, q1.accession]);
    expect(view.quarters[1].sources[0].url).toBe(secFilingIndexUrl("0000000001", h1.accession));
    expect(view.quarters[0].sources).toEqual([
      { accession: q1.accession, form: "10-Q", filed: "2026-08-01", url: secFilingIndexUrl("0000000001", q1.accession) },
    ]);
    expect(view.years).toEqual([
      {
        org: "org_test",
        fiscalYear: 2025,
        start: "2025-01-01",
        end: "2025-12-31",
        value: 700,
        currency: "USD",
        source: { accession: fy.accession, form: "10-K", filed: "2026-08-01", url: secFilingIndexUrl("0000000001", fy.accession) },
      },
    ]);
    expect(view.warnings).toEqual([]);
    expect(view.lineItem).toBeNull();
  });

  it("handles a fiscal year that ends in June (FY2026 = Jul 2025 to Jun 2026) and uses direct quarters", () => {
    const s = series(
      [
        fact("2025-07-01", "2025-09-30", 10),
        fact("2025-07-01", "2025-12-31", 30),
        fact("2026-01-01", "2026-03-31", 25),
        fact("2025-07-01", "2026-03-31", 55),
        fact("2025-07-01", "2026-06-30", 90, { form: "10-K" }),
      ],
      "06-30"
    );
    expect(summary(s)).toEqual([
      ["FY26 Q1", "2025-07-01", "2025-09-30", 10, false],
      ["FY26 Q2", "2025-10-01", "2025-12-31", 20, true],
      ["FY26 Q3", "2026-01-01", "2026-03-31", 25, false],
      ["FY26 Q4", "2026-04-01", "2026-06-30", 35, true],
    ]);
    const view = deriveCapex(s);
    expect(view.quarters.map((q) => q.calendarQuarter)).toEqual(["2025-q3", "2025-q4", "2026-q1", "2026-q2"]);
    expect(view.years.map((y) => [y.fiscalYear, y.value])).toEqual([[2026, 90]]);
    expect(view.warnings).toEqual([]);
  });

  it("warns when a direct quarter does not add up to the year-to-date facts", () => {
    const s = series(
      [
        fact("2025-07-01", "2025-09-30", 10),
        fact("2025-07-01", "2025-12-31", 30),
        fact("2025-10-01", "2025-12-31", 21),
      ],
      "06-30"
    );
    const view = deriveCapex(s);
    expect(view.quarters.map((q) => [fiscalLabel(q), q.value, q.derived])).toEqual([
      ["FY26 Q1", 10, false],
      ["FY26 Q2", 21, false],
    ]);
    expect(view.warnings).toHaveLength(1);
    expect(view.warnings[0]).toContain("FY26 Q2");
    expect(view.warnings[0]).toContain("does not equal");
  });

  it("handles 52/53-week years (NVIDIA-style ends in late January, Broadcom-style 14-week quarter)", () => {
    const nvidia = series(
      [
        fact("2024-01-29", "2024-04-28", 369),
        fact("2024-01-29", "2024-07-28", 1346),
        fact("2024-01-29", "2024-10-27", 2159),
        fact("2024-01-29", "2025-01-26", 3236, { form: "10-K" }),
        fact("2025-01-27", "2025-04-27", 1227),
      ],
      "01-31"
    );
    expect(summary(nvidia)).toEqual([
      ["FY25 Q1", "2024-01-29", "2024-04-28", 369, false],
      ["FY25 Q2", "2024-04-29", "2024-07-28", 977, true],
      ["FY25 Q3", "2024-07-29", "2024-10-27", 813, true],
      ["FY25 Q4", "2024-10-28", "2025-01-26", 1077, true],
      ["FY26 Q1", "2025-01-27", "2025-04-27", 1227, false],
    ]);
    const view = deriveCapex(nvidia);
    expect(view.quarters.every((q) => q.calendarQuarter === null)).toBe(true);
    expect(calendarSpanLabel(view.quarters[4].start, view.quarters[4].end)).toBe("Feb\u2013Apr 2025");
    expect(view.years.map((y) => [y.fiscalYear, y.end])).toEqual([[2025, "2025-01-26"]]);

    // 53-week year: a 14-week first quarter (97 days) and a 370-day year.
    const broadcom = series(
      [
        fact("2023-10-30", "2024-02-04", 122),
        fact("2023-10-30", "2024-05-05", 254),
        fact("2023-10-30", "2024-08-04", 426),
        fact("2023-10-30", "2024-11-03", 548, { form: "10-K" }),
      ],
      "11-01"
    );
    expect(summary(broadcom)).toEqual([
      ["FY24 Q1", "2023-10-30", "2024-02-04", 122, false],
      ["FY24 Q2", "2024-02-05", "2024-05-05", 132, true],
      ["FY24 Q3", "2024-05-06", "2024-08-04", 172, true],
      ["FY24 Q4", "2024-08-05", "2024-11-03", 122, true],
    ]);
    expect(deriveCapex(broadcom).warnings).toEqual([]);
  });

  it("ignores Amazon-style trailing-twelve-month rows", () => {
    const s = series([
      fact("2025-01-01", "2025-12-31", 131_819, { form: "10-K" }),
      fact("2025-04-01", "2026-03-31", 151_003),
      fact("2026-01-01", "2026-03-31", 44_203),
      fact("2025-07-01", "2026-06-30", 173_028),
      fact("2026-01-01", "2026-06-30", 98_411),
    ]);
    const view = deriveCapex(s);
    expect(summary(s)).toEqual([
      ["FY26 Q1", "2026-01-01", "2026-03-31", 44_203, false],
      ["FY26 Q2", "2026-04-01", "2026-06-30", 54_208, true],
    ]);
    expect(view.years.map((y) => [y.fiscalYear, y.value])).toEqual([[2025, 131_819]]);
    expect(view.warnings).toEqual([]);
    expect(capexPeriods(s).map((p) => p.label)).toEqual([
      "Twelve months to Dec 31, 2025",
      "Three months to Mar 31, 2026",
      "Six months to Jun 30, 2026",
    ]);
  });

  it("never derives across a gap: without the 6-month fact, Q2 and Q3 are absent (not zero)", () => {
    const s = series([
      fact("2025-01-01", "2025-03-31", 100),
      fact("2025-01-01", "2025-09-30", 450),
      fact("2025-01-01", "2025-12-31", 700, { form: "10-K" }),
    ]);
    expect(summary(s)).toEqual([
      ["FY25 Q1", "2025-01-01", "2025-03-31", 100, false],
      ["FY25 Q4", "2025-10-01", "2025-12-31", 250, true],
    ]);
  });

  it("drops a derived value below zero and warns", () => {
    const view = deriveCapex(series([fact("2025-01-01", "2025-03-31", 100), fact("2025-01-01", "2025-06-30", 90)]));
    expect(view.quarters.map((q) => fiscalLabel(q))).toEqual(["FY25 Q1"]);
    expect(view.warnings).toHaveLength(1);
    expect(view.warnings[0]).toContain("FY25 Q2");
    expect(view.warnings[0]).toContain("below zero");
  });

  it("keeps a year-to-date figure that has no quarter yet in capexPeriods (SpaceX-style)", () => {
    const s = series([fact("2026-01-01", "2026-06-30", 28_476_000_000)]);
    expect(deriveCapex(s).quarters).toEqual([]);
    expect(capexPeriods(s)).toMatchObject([{ fiscalYear: 2026, months: 6, label: "Six months to Jun 30, 2026", value: 28_476_000_000 }]);
  });
});

/* -------------------------------------------------------------- helpers */

describe("capex helpers", () => {
  it("calendarQuarterOf accepts only calendar quarters (7-day tolerance)", () => {
    expect(calendarQuarterOf("2026-01-01", "2026-03-31")).toBe("2026-q1");
    expect(calendarQuarterOf("2025-10-01", "2025-12-31")).toBe("2025-q4");
    expect(calendarQuarterOf("2026-01-07", "2026-03-25")).toBe("2026-q1");
    expect(calendarQuarterOf("2023-07-02", "2023-09-30")).toBe("2023-q3");
    expect(calendarQuarterOf("2026-04-01", "2026-06-24")).toBe("2026-q2");
    expect(calendarQuarterOf("2026-04-01", "2026-06-23")).toBeNull(); // more than 7 days before the end of June
    expect(calendarQuarterOf("2026-01-08", "2026-03-31")).toBeNull(); // starts on day 8
    expect(calendarQuarterOf("2025-12-28", "2026-03-28")).toBeNull(); // starts in December
    expect(calendarQuarterOf("2026-02-01", "2026-04-30")).toBeNull(); // Feb-Apr
    expect(calendarQuarterOf("2026-01-01", "2026-06-30")).toBeNull(); // six months
    expect(calendarQuarterOf("2025-10-01", "2026-12-31")).toBeNull(); // different years
  });

  it("calendarSpanLabel names the months a span covers, with an en dash", () => {
    expect(calendarSpanLabel("2026-01-01", "2026-03-31")).toBe("Jan\u2013Mar 2026");
    expect(calendarSpanLabel("2025-12-01", "2026-02-28")).toBe("Dec 2025\u2013Feb 2026");
    expect(calendarSpanLabel("2026-01-26", "2026-04-26")).toBe("Feb\u2013Apr 2026");
    expect(calendarSpanLabel("2026-05-04", "2026-08-02")).toBe("May\u2013Jul 2026");
    expect(calendarSpanLabel("2025-12-28", "2026-03-28")).toBe("Jan\u2013Mar 2026");
    expect(calendarSpanLabel("2025-07-01", "2026-06-30")).toBe("Jul 2025\u2013Jun 2026");
    expect(calendarSpanLabel("2026-03-01", "2026-03-31")).toBe("Mar 2026");
    expect(calendarSpanLabel("2026-03-29", "2026-04-02")).toBe("Mar\u2013Apr 2026"); // too short to snap
  });

  it("secFilingIndexUrl strips leading zeros from the CIK and dashes from the folder", () => {
    expect(secFilingIndexUrl("0000789019", "0001193125-26-323660")).toBe(
      "https://www.sec.gov/Archives/edgar/data/789019/000119312526323660/0001193125-26-323660-index.htm"
    );
    expect(secFilingIndexUrl("0000002488", "0000002488-26-000123")).toBe(
      "https://www.sec.gov/Archives/edgar/data/2488/000000248826000123/0000002488-26-000123-index.htm"
    );
  });

  it("fiscalLabel, spanMonths, isFiscalYearStart and formatCapexAmount", () => {
    expect(fiscalLabel({ fiscalYear: 2026, fiscalQuarter: 3 })).toBe("FY26 Q3");
    expect(fiscalLabel({ fiscalYear: 2009, fiscalQuarter: 1 })).toBe("FY09 Q1");
    expect(spanMonths("2026-01-01", "2026-03-31")).toBe(3);
    expect(spanMonths("2023-10-30", "2024-02-04")).toBe(3);
    expect(spanMonths("2026-01-01", "2026-06-30")).toBe(6);
    expect(spanMonths("2025-07-01", "2026-03-31")).toBe(9);
    expect(spanMonths("2021-12-26", "2022-12-31")).toBe(12);
    expect(spanMonths("2023-01-01", "2023-05-17")).toBeNull();
    expect(isFiscalYearStart("2026-01-01", "12-31")).toBe(true);
    expect(isFiscalYearStart("2025-07-01", "12-31")).toBe(false);
    expect(isFiscalYearStart("2025-01-27", "01-31")).toBe(true);
    expect(isFiscalYearStart("2025-12-28", "12-26")).toBe(true);
    expect(formatCapexAmount(28_476_000_000, "USD")).toBe("$28,476,000,000");
    expect(formatCapexAmount(956_006_500_000, "TWD")).toBe("TWD 956,006,500,000");
  });
});

/* ------------------------------------------------------------ real data */

describe("metrics/capex.json (SEC XBRL companyfacts)", () => {
  const file = capexFileSchema.parse(capexJson);
  const byOrg = new Map(file.series.map((s) => [s.org, s]));
  const view = (org: string) => deriveCapex(byOrg.get(org)!);

  it("parses with capexFileSchema and is sorted", () => {
    expect(file.source).toBe("SEC XBRL companyfacts");
    expect(file.series.map((s) => s.org)).toEqual([...file.series.map((s) => s.org)].sort());
    for (const s of file.series) {
      const keys = s.facts.map((f) => `${f.start}/${f.end}`);
      expect(keys).toEqual([...keys].sort());
      expect(new Set(keys).size).toBe(keys.length);
    }
  });

  it("has data for every required company", () => {
    const required = [
      "org_microsoft",
      "org_amazon",
      "org_alphabet",
      "org_meta",
      "org_oracle",
      "org_coreweave",
      "org_nebius",
      "org_spacex",
    ];
    // Nebius tags only its annual 20-F; SpaceX has only six-month facts so far (no quarter can be derived yet).
    const annualOnly = new Set(["org_nebius"]);
    const yearToDateOnly = new Set(["org_spacex"]);
    for (const org of required) {
      const s = byOrg.get(org);
      expect(s, org).toBeDefined();
      const v = deriveCapex(s!);
      if (annualOnly.has(org)) expect(v.years.length, org).toBeGreaterThan(0);
      else if (yearToDateOnly.has(org)) expect(capexPeriods(s!).length, org).toBeGreaterThan(0);
      else expect(v.quarters.length, org).toBeGreaterThan(0);
    }
    for (const s of file.series) {
      const v = deriveCapex(s);
      expect(v.quarters.length + v.years.length + capexPeriods(s).length, s.org).toBeGreaterThan(0);
    }
  });

  const annual: Array<[string, number, number]> = [
    ["org_microsoft", 2026, 115_948_000_000],
    ["org_amazon", 2025, 131_819_000_000],
    ["org_alphabet", 2025, 91_447_000_000],
    ["org_meta", 2025, 69_691_000_000],
    ["org_oracle", 2026, 55_663_000_000],
    ["org_coreweave", 2025, 10_309_000_000],
    ["org_nebius", 2025, 4_066_000_000],
  ];

  it.each(annual)("%s FY%i matches the planning probe when present", (org, fiscalYear, value) => {
    const year = view(org).years.find((y) => y.fiscalYear === fiscalYear);
    if (!year) return;
    expect(year.value).toBe(value);
    expect(year.source.url).toContain(year.source.accession);
  });

  it("this build (2026-10-21) has all of those annual figures", () => {
    for (const [org, fiscalYear] of annual) {
      expect(view(org).years.some((y) => y.fiscalYear === fiscalYear), `${org} FY${fiscalYear}`).toBe(true);
    }
  });

  it("SpaceX: six months to Jun 30, 2026 is $28,476 million when the fact is present", () => {
    const h1 = byOrg.get("org_spacex")?.facts.find((f) => f.start === "2026-01-01" && f.end === "2026-06-30");
    if (h1) expect(h1.value).toBe(28_476_000_000);
  });

  it("derives Microsoft's FY26 Q4 from the 10-K and the Q3 10-Q", () => {
    const q4 = view("org_microsoft").quarters.find((q) => q.fiscalYear === 2026 && q.fiscalQuarter === 4)!;
    expect(q4).toMatchObject({ start: "2026-04-01", end: "2026-06-30", value: 35_802_000_000, derived: true, calendarQuarter: "2026-q2" });
    expect(q4.formula).toBe(
      "Twelve months to Jun 30, 2026 ($115,948,000,000) minus nine months to Mar 31, 2026 ($80,146,000,000)"
    );
    expect(q4.sources).toHaveLength(2);
  });

  it("keeps Amazon's trailing-twelve-month rows out of the file", () => {
    for (const f of byOrg.get("org_amazon")!.facts) {
      if (spanMonths(f.start, f.end) !== 3) expect(f.start.slice(5), `${f.start}..${f.end}`).toBe("01-01");
    }
  });

  it("finds no data errors and no negative or undated quarters", () => {
    for (const s of file.series) {
      const v = deriveCapex(s);
      expect(v.warnings, s.org).toEqual([]);
      for (const q of v.quarters) {
        expect(q.value, `${s.org} ${fiscalLabel(q)}`).toBeGreaterThanOrEqual(0);
        expect(q.start < q.end).toBe(true);
        expect(q.derived ? q.formula !== null && q.sources.length >= 1 : q.formula === null && q.sources.length === 1).toBe(true);
      }
    }
  });
});
