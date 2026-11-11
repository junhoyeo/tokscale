import { describe, expect, it } from "vitest";
import { titleCase } from "@/lib/text/titleCase";

describe("titleCase", () => {
  it("capitalizes principal words and keeps small words lowercase", () => {
    expect(titleCase("How it works")).toBe("How It Works");
    expect(titleCase("Trusted by professionals at")).toBe("Trusted by Professionals at");
    expect(titleCase("Largest valuation")).toBe("Largest Valuation");
  });

  it("starts every dotted segment with a capital", () => {
    expect(titleCase("Equity raised · 2026 so far")).toBe("Equity Raised · 2026 So Far");
    expect(titleCase("Series H · Closed · May 28, 2026")).toBe("Series H · Closed · May 28, 2026");
    expect(titleCase("AI lab · Private · Public benefit corporation · San Francisco, US")).toBe(
      "AI Lab · Private · Public Benefit Corporation · San Francisco, US",
    );
  });

  it("keeps acronyms, numbers, mixed case and domains as written", () => {
    expect(titleCase("README embed")).toBe("README Embed");
    expect(titleCase("Cash capex · Q2 2026")).toBe("Cash Capex · Q2 2026");
    expect(titleCase("tokscale.ai · public")).toBe("tokscale.ai · Public");
    expect(titleCase("xAI and SpaceX")).toBe("xAI and SpaceX");
  });
});
