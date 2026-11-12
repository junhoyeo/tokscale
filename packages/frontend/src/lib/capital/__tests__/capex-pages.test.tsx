// @vitest-environment jsdom
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import SpendingPage from "@/app/(main)/capital/spending/page";
import CompanyPage from "@/app/(main)/capital/companies/[slug]/page";
import { CapexSources } from "@/app/(main)/capital/_components/CapexSources";
import { formatMoney, ledger } from "@/lib/capital/ledger";
import { fiscalLabel } from "@/lib/capital/ledger/capex";

vi.mock("@/app/(main)/capital/_components/CapitalShell", () => ({
  CapitalShell: ({ children }: { children: ReactNode }) => <main>{children}</main>,
}));
vi.mock("@/lib/capital/ledger/usage", () => ({ loadUsageSnapshot: vi.fn().mockResolvedValue(null) }));

const documentOf = (html: string) => new DOMParser().parseFromString(html, "text/html");
const microsoftQuarters = ledger.getCapexFor("org_microsoft")!.quarters;
const derived = microsoftQuarters.find((quarter) => quarter.fiscalYear === 2026 && quarter.fiscalQuarter === 4)!;
const latest = microsoftQuarters.at(-1)!;
const urls = (element: Element) => [...element.querySelectorAll("a")].map((a) => a.href);

describe("derived cash-capex evidence", () => {
  it("uses the real Microsoft FY26 Q4 derivation and both its filings", () => {
    expect(derived).toMatchObject({ start: "2026-04-01", end: "2026-06-30", derived: true, value: 35_802_000_000 });
    expect(derived.formula).toContain("$115,948,000,000");
    expect(derived.formula).toContain("$80,146,000,000");
    expect(derived.sources.map((s) => s.accession)).toEqual(["0001193125-26-323660", "0001193125-26-191507"]);
  });

  it("cites the latest filing beside Microsoft's latest spending figure", () => {
    const document = documentOf(renderToStaticMarkup(<SpendingPage />));
    const row = document.querySelector('tbody a[href="/capital/companies/microsoft#spending"]')!.closest("tr")!;
    const cash = row.querySelector('[data-label="Cash paid"]')!;
    expect(cash.textContent).toContain(formatMoney(latest.value, latest.currency));
    expect(urls(cash)).toEqual(latest.sources.map((s) => s.url));
    for (const source of latest.sources) expect(cash.textContent).toContain(source.form);
    expect(row.hasAttribute("data-row-link")).toBe(true);
  });

  it("keeps the latest sources in the profile header and the full older quarterly derivation", async () => {
    const page = await CompanyPage({ params: Promise.resolve({ slug: "microsoft" }) });
    const child = page.props.children;
    const profile = await child.type(child.props);
    const document = documentOf(renderToStaticMarkup(profile));
    const header = document.querySelector('dl[aria-label="Microsoft key facts"]')!;
    const label = [...header.querySelectorAll("dt")].find((dt) => dt.textContent === `Cash capex, ${fiscalLabel(latest)}`)!;
    expect(urls(label.parentElement!)).toEqual(latest.sources.map((s) => s.url));
    const quarter = [...document.querySelectorAll("#spending tbody tr")].find((row) => row.textContent?.includes(fiscalLabel(derived)))!;
    expect(quarter.textContent).toContain(derived.formula);
    expect(urls(quarter)).toEqual(derived.sources.map((s) => s.url));
  });

  it("keeps a directly filed quarter's single source and identifies it accessibly", () => {
    const direct = ledger.getCapexFor("org_microsoft")!.quarters.find((q) => !q.derived)!;
    expect(direct.sources).toHaveLength(1);
    const document = documentOf(renderToStaticMarkup(<CapexSources sources={direct.sources} compact />));
    expect(urls(document.body)).toEqual([direct.sources[0].url]);
    expect(document.querySelector("a")?.getAttribute("aria-label")).toContain(direct.sources[0].accession);
  });
});
