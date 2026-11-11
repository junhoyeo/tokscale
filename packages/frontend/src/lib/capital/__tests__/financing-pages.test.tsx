// @vitest-environment jsdom
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import RoundsPage from "@/app/(main)/capital/rounds/page";
import { generateStaticParams as roundParams } from "@/app/(main)/capital/rounds/[slug]/page";
import { generateStaticParams as dealParams } from "@/app/(main)/capital/deals/[slug]/page";
import CompanyPage from "@/app/(main)/capital/companies/[slug]/page";
import { EQUITY_MEASURES, financingAmount, ledger } from "@/lib/capital/ledger";
import { isFinancing } from "@/lib/capital/ledger/financing";
import { capitalSitemapEntries } from "@/lib/capital/sitemap";

vi.mock("@/app/(main)/capital/_components/CapitalShell", () => ({
  CapitalShell: ({ children }: { children: ReactNode }) => <main>{children}</main>,
}));
vi.mock("@/lib/capital/ledger/usage", () => ({ loadUsageSnapshot: vi.fn().mockResolvedValue(null) }));

const doc = (html: string) => new DOMParser().parseFromString(html, "text/html");
const slug = "cerebras-openai-2025";
const href = `/capital/deals/${slug}`;

describe("mixed-event financing", () => {
  it("lists the actual Cerebras loan once in Debt, using $1B and its closed status", async () => {
    const page = await RoundsPage({ searchParams: Promise.resolve({ type: "debt" }) });
    const document = doc(renderToStaticMarkup(page));
    const links = document.querySelectorAll(`tbody a[data-row-primary][href="${href}"]`);
    expect(links).toHaveLength(1);
    const row = links[0].closest("tr")!;
    expect(row.querySelector('[data-label="Amount"]')?.textContent).toContain("$1B");
    expect(row.querySelector('[data-label="Amount"]')?.textContent).not.toContain("$20B");
    expect(row.querySelector('[data-label="Status"]')?.textContent).toContain("Closed");
    expect(row.hasAttribute("data-row-link")).toBe(true);
  });

  it("shows the facility rather than equity for OpenAI's mixed round in the Debt filter", async () => {
    const event = ledger.getEvent("openai-funding-2026-03")!;
    const page = await RoundsPage({ searchParams: Promise.resolve({ type: "debt" }) });
    const document = doc(renderToStaticMarkup(page));
    const row = document.querySelector(`a[data-row-primary][href="${event.href}"]`)!.closest("tr")!;
    expect(row.querySelector('[data-label="Amount"]')?.textContent).toContain("≈$4.7B");
    expect(row.querySelector('[data-label="Amount"]')?.textContent).not.toContain("$122B");
  });

  it("uses the debt amount on All as well, without changing the compute headline", async () => {
    const page = await RoundsPage({ searchParams: Promise.resolve({}) });
    const document = doc(renderToStaticMarkup(page));
    const row = document.querySelector(`a[data-row-primary][href="${href}"]`)!.closest("tr")!;
    expect(row.querySelector('[data-label="Amount"]')?.textContent).toContain("$1B");
    expect(ledger.getEvent(slug)!.headline?.short).toBe(">$20B");
    expect(financingAmount(ledger.getEvent(slug)!)?.amount.measure).toBe("debt_raised");
  });

  it("includes the loan in the borrower's funding history, but not the lender's", async () => {
    const borrower = ledger.getCompany("cerebras")!;
    expect(borrower.rounds.filter((e) => e.slug === slug)).toHaveLength(1);
    expect(ledger.getCompany("openai")!.rounds.some((e) => e.slug === slug)).toBe(false);
    expect(borrower.compute.filter((e) => e.slug === slug)).toHaveLength(1);
    // Resolve the async profile child inside the page shell before server rendering.
    const element = await CompanyPage({ params: Promise.resolve({ slug: "cerebras" }) });
    const child = element.props.children;
    const profile = await child.type(child.props);
    const document = doc(renderToStaticMarkup(profile));
    const funding = document.querySelector("#funding")!;
    const row = funding.querySelector(`a[data-row-primary][href="${href}"]`)!.closest("tr")!;
    expect(row.querySelector('[data-label="Debt"]')?.textContent).toContain("$1B");
    expect(row.querySelector("a[data-row-primary]")?.textContent).toBe("Debt financing");
    expect(row.querySelector('[data-label="Status"]')?.textContent).toContain("Closed");
  });

  it("recognizes typed financing without relying on a route or the first event type", () => {
    const event = ledger.getEvent(slug)!;
    expect(isFinancing({ ...event.event, types: ["compute_contract"] })).toBe(true);
    expect(isFinancing({ ...event.event, amounts: [] })).toBe(true);
    expect(isFinancing({ ...event.event, types: ["compute_contract"], amounts: [] })).toBe(false);
    expect(financingAmount(event, ["debt_raised"], "org_openai")).toBeNull();
    expect(financingAmount(event, ["contract_value"])).toBeNull();
  });

  it("keeps one canonical detail page and sitemap entry for the mixed event", () => {
    expect(ledger.getFinancings().filter((e) => e.slug === slug)).toHaveLength(1);
    expect(ledger.getRounds().some((e) => e.slug === slug)).toBe(false);
    expect(roundParams()).not.toContainEqual({ slug });
    expect(dealParams().filter((p) => p.slug === slug)).toHaveLength(1);
    const urls = capitalSitemapEntries().filter((entry) => entry.url.endsWith(`/${slug}`));
    expect(urls).toHaveLength(1);
    expect(urls[0].url).toMatch(new RegExp(`${href}$`));
  });

  it.each([undefined, "equity"])("shows Amazon's current $5B investment before its future commitment in the %s index", async (type) => {
    const event = ledger.getEvent("anthropic-amazon-2026-04")!;
    expect(financingAmount(event)?.amount.id).toBe("investment");
    expect(financingAmount(event, EQUITY_MEASURES)?.amount.id).toBe("investment");
    const page = await RoundsPage({ searchParams: Promise.resolve({ type }) });
    const document = doc(renderToStaticMarkup(page));
    const row = document.querySelector(`a[data-row-primary][href="${event.href}"]`)!.closest("tr")!;
    const amount = row.querySelector('[data-label="Amount"]')!;
    expect(amount.textContent).toContain("$5B");
    expect(amount.textContent).not.toContain("$20B");
    expect(amount.textContent).toContain("counted in a later event");
  });

  it("keeps the counted current investment visible in Anthropic's funding history", async () => {
    const event = ledger.getEvent("anthropic-amazon-2026-04")!;
    const amount = financingAmount(event, [...EQUITY_MEASURES, "ipo_proceeds", "form_d_amount_sold"], "org_anthropic");
    expect(amount?.amount.id).toBe("investment");
    const page = await CompanyPage({ params: Promise.resolve({ slug: "anthropic" }) });
    const child = page.props.children;
    const document = doc(renderToStaticMarkup(await child.type(child.props)));
    const row = document.querySelector(`#funding a[data-row-primary][href="${event.href}"]`)!.closest("tr")!;
    expect(row.querySelector('[data-label="Equity"]')?.textContent).toContain("$5B");
    expect(row.querySelector('[data-label="Equity"]')?.textContent).not.toContain("$20B");
    expect(row.textContent).toContain("counted in a later event");
  });

  it("keeps genuine strategic financing discoverable without adding it again to equity totals", () => {
    const amazon = ledger.getEvent("anthropic-amazon-2026-04")!;
    expect(ledger.getFinancings()).toContain(amazon);
    expect(ledger.getCompany("anthropic")!.rounds).toContain(amazon);
    expect(financingAmount(amazon, ["equity_investment"])?.skipped).toBe("included_in");
    const counted = ledger.equityRaised("org_anthropic").totals.flatMap((t) => t.counted);
    expect(counted.every((a) => a.measure === "equity_raised" && !a.includedIn && !a.partOf)).toBe(true);
    expect(counted.filter((a) => a.value === 65_000_000_000)).toHaveLength(1);
    expect(financingAmount(ledger.getEvent("akamai-anthropic-2026")!)).toBeNull();
  });
});
