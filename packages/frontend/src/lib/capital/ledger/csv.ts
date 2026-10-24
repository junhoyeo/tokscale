import { absoluteUrl, companyUrl } from "../urls";
import { fiscalLabel } from "./capex";
import { CATEGORY_LABELS, ORG_STATUS_LABELS } from "./labels";
import { ledger as defaultLedger, type LedgerModel } from "./model";

/**
 * CSV export of the companies directory. Missing values are empty cells, never zero. Amounts keep
 * their own currency, basis and status; equity raised is per currency and never converted.
 */
export type CsvCell = string | number | boolean | null;

/** RFC 4180: quote a cell when it holds a comma, a quote or a line break; double inner quotes. */
export function csvCell(value: CsvCell): string {
  if (value === null) return "";
  const text = typeof value === "boolean" ? (value ? "true" : "false") : String(value);
  return /[",\r\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export function toCsv(header: readonly string[], rows: ReadonlyArray<readonly CsvCell[]>): string {
  for (const row of rows) {
    if (row.length !== header.length) throw new Error(`CSV row has ${row.length} cells, expected ${header.length}`);
  }
  return [header, ...rows].map((row) => row.map(csvCell).join(",")).join("\r\n") + "\r\n";
}

export const COMPANIES_CSV_HEADER = [
  "company",
  "slug",
  "page_url",
  "categories",
  "country",
  "status",
  "ticker",
  "last_financing",
  "last_financing_date",
  "last_financing_url",
  "post_money_value",
  "post_money_currency",
  "post_money_basis",
  "post_money_status",
  "post_money_as_of",
  "post_money_source_url",
  "equity_raised_usd",
  "equity_raised_usd_rounds",
  "equity_raised_other_currencies",
  "compute_deals",
  "compute_deals_as_buyer",
  "compute_deals_as_provider",
  "largest_contract_value",
  "largest_contract_currency",
  "largest_contract_basis",
  "largest_contract_source_url",
  "cash_capex_latest_quarter_value",
  "cash_capex_latest_quarter_currency",
  "cash_capex_latest_quarter_fiscal_label",
  "cash_capex_latest_quarter_start",
  "cash_capex_latest_quarter_end",
  "cash_capex_latest_quarter_derived",
  "last_activity",
] as const;

export function companiesCsv(model: LedgerModel = defaultLedger): string {
  const rows = model
    .getDirectoryRows()
    .sort((a, b) => a.org.name.localeCompare(b.org.name))
    .map((row): CsvCell[] => {
      const usd = row.equity.totals.find((t) => t.currency === "USD") ?? null;
      const other = row.equity.totals.filter((t) => t.currency !== "USD");
      const valuation = row.valuation?.amount ?? null;
      const contract = row.largestContract?.amount ?? null;
      const capex = row.capexLatest;
      return [
        row.org.name,
        row.org.slug,
        companyUrl(row.org.slug),
        row.categories.map((c) => CATEGORY_LABELS[c]).join("; "),
        row.country,
        ORG_STATUS_LABELS[row.org.org.status],
        row.org.ticker,
        row.lastFinancing ? (row.lastFinancing.event.roundLabel ?? row.lastFinancing.typeLabels[0]) : null,
        row.lastFinancing?.date ?? null,
        row.lastFinancing ? absoluteUrl(row.lastFinancing.href) : null,
        valuation?.value ?? null,
        valuation?.currency ?? null,
        valuation?.basis ?? null,
        valuation?.status ?? null,
        valuation?.asOf ?? null,
        row.valuation?.source.url ?? null,
        usd?.value ?? null,
        usd ? usd.counted.length : null,
        other.length > 0 ? other.map((t) => `${t.currency} ${t.value}`).join("; ") : null,
        row.dealsCount,
        row.dealsAsBuyer,
        row.dealsAsProvider,
        contract?.value ?? null,
        contract?.currency ?? null,
        contract?.basis ?? null,
        row.largestContract?.source.url ?? null,
        capex?.value ?? null,
        capex?.currency ?? null,
        capex ? fiscalLabel(capex) : null,
        capex?.start ?? null,
        capex?.end ?? null,
        capex ? capex.derived : null,
        row.lastActivity,
      ];
    });
  return toCsv(COMPANIES_CSV_HEADER, rows);
}
