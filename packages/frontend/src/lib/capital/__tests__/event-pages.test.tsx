// @vitest-environment jsdom
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { EventDetail, eventSeoTitle } from "@/app/(main)/capital/_components/EventDetail";
import { generateMetadata } from "@/app/(main)/capital/deals/[slug]/page";
import { ledger } from "@/lib/capital/ledger";

describe("compute event presentation", () => {
  it("renders Mistral's step-up as a lower bound, including the formula", () => {
    const event = ledger.getEvent("mistral-series-d-2026")!;
    const doc = new DOMParser().parseFromString(renderToStaticMarkup(<EventDetail e={event} />), "text/html");
    const label = [...doc.querySelectorAll("dt")].find((node) => node.textContent === "Step-up vs Series C")!;
    expect(label.nextElementSibling?.textContent).toBe(">1.79x");
    expect(label.parentElement?.textContent).toContain("Computed by Tokscale: >€21B ÷ €11.7B");
  });
  it("headlines the Cerebras agreement with its contract, not its working-capital loan", async () => {
    const event = ledger.getEvent("cerebras-openai-2025")!;
    expect(event.headline?.amount?.amount.measure).toBe("contract_value");
    expect(event.headline?.short).toBe(">$20B");
    expect(event.tag).toBe("Compute");
    const title = "OpenAI and Cerebras inference compute agreement: >$20B (Dec 2025)";
    expect(eventSeoTitle(event)).toBe(title);
    const metadata = await generateMetadata({ params: Promise.resolve({ slug: event.slug }) });
    expect(metadata.title).toBe(`${title} | Tokscale`);
    expect(metadata.openGraph?.title).toBe(metadata.title);
    expect(metadata.twitter?.title).toBe(metadata.title);

    const doc = new DOMParser().parseFromString(renderToStaticMarkup(<EventDetail e={event} />), "text/html");
    const stats = doc.querySelector("dl")!;
    expect(stats.querySelector("dt")?.textContent).toBe("Value of the multi-year deal");
    expect(stats.textContent).toContain(">$20B");
    expect(stats.textContent).toContain("$1B");
  });

  it("does not replace the financing headline when a round also has a compute commitment", () => {
    const original = ledger.getEvent("cerebras-openai-2025")!;
    const financing = ledger.eventView({ ...original.event, id: "evt_test_financing", kind: "financing" });
    expect(financing.headline?.amount?.amount.measure).toBe("debt_raised");
    expect(financing.headline?.short).toBe("$1B");
    expect(ledger.getEvent("anthropic-series-h-2026")!.headline?.amount?.amount.measure).toBe("equity_raised");
    expect(ledger.getEvent("crusoe-form-d-2026")!.headline?.amount?.amount.measure).toBe("form_d_amount_sold");
  });
});
