// @vitest-environment jsdom
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { EventFeed } from "@/app/(main)/capital/_components/EventFeed";
import ComputePage from "@/app/(main)/capital/compute/page";
import CompanyPage from "@/app/(main)/capital/companies/[slug]/page";
import { ledger, type LedgerModel } from "@/lib/capital/ledger";
import { buildHomeCapital } from "@/lib/capital/ledger/home";

vi.mock("@/app/(main)/capital/_components/CapitalShell", () => ({
  CapitalShell: ({ children }: { children: ReactNode }) => <main>{children}</main>,
}));
vi.mock("@/lib/capital/ledger/usage", () => ({ loadUsageSnapshot: vi.fn().mockResolvedValue(null) }));

const parse = (node: ReactNode) => new DOMParser().parseFromString(renderToStaticMarkup(node), "text/html");
const event = ledger.getEvent("terawulf-anthropic-lease-2026")!;
const expectedSource = event.headline!.amount!.source.url;

function rowSource(document: Document) {
  const row = document.querySelector(`a[data-row-primary][href="${event.href}"]`)!.closest("tr")!;
  return row.querySelector<HTMLAnchorElement>('[data-label="Source"] a')?.href;
}

describe("Capital amount source links", () => {
  it("uses the headline amount source in event feeds", () => {
    const document = parse(<EventFeed events={[event]} />);
    expect(document.querySelector<HTMLAnchorElement>('li a[target="_blank"]')?.href).toBe(expectedSource);
  });

  it("uses the displayed contract source on the compute index", async () => {
    const document = parse(await ComputePage({ searchParams: Promise.resolve({}) }));
    expect(rowSource(document)).toBe(expectedSource);
  });

  it("uses the displayed contract source on company compute tables", async () => {
    const page = await CompanyPage({ params: Promise.resolve({ slug: "terawulf" }) });
    const profile = page.props.children;
    const document = parse(await profile.type(profile.props));
    expect(rowSource(document)).toBe(expectedSource);
  });

  it("serializes the headline amount source for the home Capital preview", () => {
    const model = new Proxy(ledger, {
      get(target, property, receiver) {
        if (property === "getEvents") return () => [event];
        return Reflect.get(target, property, receiver);
      },
    }) as LedgerModel;
    const home = buildHomeCapital({ ledger: model, snapshot: null });
    expect(home.events[0].source?.url).toBe(expectedSource);
  });
});
