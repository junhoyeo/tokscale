// @vitest-environment jsdom
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import SpendingPage from "@/app/(main)/capital/spending/page";
import CompanyPage from "@/app/(main)/capital/companies/[slug]/page";
import { CapexSources } from "@/app/(main)/capital/_components/CapexSources";
import { ledger } from "@/lib/capital/ledger";

vi.mock("@/app/(main)/capital/_components/CapitalShell", () => ({
  CapitalShell: ({ children }: { children: ReactNode }) => <main>{children}</main>,
}));
vi.mock("@/lib/capital/ledger/usage", () => ({ loadUsageSnapshot: vi.fn().mockResolvedValue(null) }));

const documentOf = (html: string) => new DOMParser().parseFromString(html, "text/html");
const latest = ledger.getCapexFor("org_microsoft")!.quarters.at(-1)!;
const urls = (element: Element) => [...element.querySelectorAll("a")].map((a) => a.href);

describe("derived cash-capex evidence", () => {
  it("uses the real Microsoft FY26 Q4 derivation and both its filings", () => {
    expect(latest).toMatchObject({ start: "2026-04-01", end: "2026-06-30", derived: true, value: 35_802_000_000 });
    expect(latest.formula).toContain("$115,948,000,000");
    expect(latest.formula).toContain("$80,146,000,000");
    expect(latest.sources.map((s) => s.accession)).toEqual(["0001193125-26-323660", "0001193125-26-191507"]);
  });

  it("cites both the current 10-K and prior 10-Q beside Microsoft's latest spending figure", () => {
    const document = documentOf(renderToStaticMarkup(<SpendingPage />));
    const row = document.querySelector('tbody a[href="/capital/companies/microsoft#spending"]')!.closest("tr")!;
    const cash = row.querySelector('[data-label="Cash paid"]')!;
    expect(cash.textContent).toContain("$35.8B");
    expect(urls(cash)).toEqual(latest.sources.map((s) => s.url));
    expect(cash.textContent).toContain("10-K");
    expect(cash.textContent).toContain("10-Q");
    expect(row.hasAttribute("data-row-link")).toBe(true);
  });

  it("keeps both filings in the profile header and the full quarterly derivation", async () => {
    const page = await CompanyPage({ params: Promise.resolve({ slug: "microsoft" }) });
    const child = page.props.children;
    const profile = await child.type(child.props);
    const document = documentOf(renderToStaticMarkup(profile));
    const header = document.querySelector('dl[aria-label="Microsoft key facts"]')!;
    const label = [...header.querySelectorAll("dt")].find((dt) => dt.textContent === "Cash capex, FY26 Q4")!;
    expect(urls(label.parentElement!)).toEqual(latest.sources.map((s) => s.url));
    const quarter = document.querySelector("#spending tbody tr")!;
    expect(quarter.textContent).toContain("FY26 Q4");
    expect(quarter.textContent).toContain(latest.formula);
    expect(urls(quarter)).toEqual(latest.sources.map((s) => s.url));
  });

  it("keeps a directly filed quarter's single source and identifies it accessibly", () => {
    const direct = ledger.getCapexFor("org_microsoft")!.quarters.find((q) => !q.derived)!;
    expect(direct.sources).toHaveLength(1);
    const document = documentOf(renderToStaticMarkup(<CapexSources sources={direct.sources} compact />));
    expect(urls(document.body)).toEqual([direct.sources[0].url]);
    expect(document.querySelector("a")?.getAttribute("aria-label")).toContain(direct.sources[0].accession);
  });
});
