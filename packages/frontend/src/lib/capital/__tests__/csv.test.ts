import { describe, expect, it } from "vitest";
import { COMPANIES_CSV_HEADER, companiesCsv, csvCell, toCsv } from "@/lib/capital/ledger/csv";

describe("companies CSV", () => {
  it("escapes cells and writes missing values as empty cells, never zero", () => {
    expect(csvCell("Anthropic")).toBe("Anthropic");
    expect(csvCell("Neocloud, Data centers")).toBe('"Neocloud, Data centers"');
    expect(csvCell('say "hi"')).toBe('"say ""hi"""');
    expect(csvCell(null)).toBe("");
    expect(csvCell(0)).toBe("0");
    expect(csvCell(true)).toBe("true");
    expect(toCsv(["a", "b"], [[1, null]])).toBe("a,b\r\n1,\r\n");
    expect(() => toCsv(["a", "b"], [[1]])).toThrow(/expected 2/);
  });

  it("exports one row per company with sourced, typed numbers", () => {
    const lines = companiesCsv().trim().split("\r\n");
    expect(lines[0]).toBe(COMPANIES_CSV_HEADER.join(","));
    const header = lines[0].split(",");
    const anthropic = lines.find((l) => l.startsWith("Anthropic,"))!;
    const cells = anthropic.split(",");
    expect(cells[header.indexOf("post_money_value")]).toBe("965000000000");
    expect(cells[header.indexOf("post_money_currency")]).toBe("USD");
    expect(cells[header.indexOf("equity_raised_usd")]).toBe("111500000000");
    expect(cells[header.indexOf("post_money_source_url")]).toMatch(/^https:\/\/www\.anthropic\.com\//);
    const akamai = lines.find((l) => l.startsWith("Akamai,"))!.split(",");
    expect(akamai[header.indexOf("equity_raised_usd")]).toBe("");
  });
});
