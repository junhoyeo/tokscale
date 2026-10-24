import { BASIS_TEXT, QUANTITY_UNITS } from "./labels";
import type { Amount, Basis, Quantity, Source } from "./schema";

/**
 * Money and quantity formatting for the ledger. Currencies are never converted:
 * a EUR amount prints in EUR. Compact values trim trailing zeros, so a stated
 * "$65 billion" prints as "$65B" and "$3,108,543,889" as "$3.11B".
 */

const SYMBOLS: Record<string, string> = {
  USD: "$",
  EUR: "€",
  GBP: "£",
  JPY: "¥",
  CNY: "CN¥",
  HKD: "HK$",
  TWD: "NT$",
};

export function currencySymbol(currency: string): string {
  return SYMBOLS[currency] ?? `${currency} `;
}

function trimNumber(value: number, decimals: number): string {
  return value.toFixed(decimals).replace(/\.0+$/, "").replace(/(\.\d*?)0+$/, "$1");
}

/** "$65B", "$11.6B", "$3.11B", "$530M", "€1.7B", "$2,226.60" (values under 1,000 keep cents when they have them). */
export function formatMoney(value: number, currency: string): string {
  const symbol = currencySymbol(currency);
  const abs = Math.abs(value);
  const sign = value < 0 ? "\u2212" : "";
  const units: Array<[number, string]> = [
    [1e12, "T"],
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ];
  for (const [size, suffix] of units) {
    if (abs >= size) {
      const scaled = abs / size;
      const decimals = scaled >= 100 ? 1 : 2;
      return `${sign}${symbol}${trimNumber(scaled, decimals)}${suffix}`;
    }
  }
  const cents = Number.isInteger(abs) ? 0 : 2;
  return `${sign}${symbol}${abs.toLocaleString("en-US", { minimumFractionDigits: cents, maximumFractionDigits: cents })}`;
}

/** Full value with thousands separators: "$3,108,543,889". */
export function formatMoneyExact(value: number, currency: string): string {
  const cents = Number.isInteger(value) ? 0 : 2;
  return `${currencySymbol(currency)}${value.toLocaleString("en-US", { minimumFractionDigits: cents, maximumFractionDigits: cents })}`;
}

export function basisSymbol(basis: Basis): string {
  return BASIS_TEXT[basis].symbol;
}

type AmountLike = Pick<Amount, "value" | "currency" | "basis"> & { measure?: Amount["measure"] };

/** Per-share prices print exactly ("$2,226.60"); everything else compactly. */
function money(amount: AmountLike & { value: number }): string {
  return amount.measure === "price_per_share" ? formatMoneyExact(amount.value, amount.currency) : formatMoney(amount.value, amount.currency);
}

/** Dense form for tables: "≈$11.6B", "≤≈$6.5B", ">$100B". */
export function formatAmountShort(amount: AmountLike): string {
  if (amount.value === null) return "Undisclosed";
  return `${basisSymbol(amount.basis)}${money({ ...amount, value: amount.value })}`;
}

/** Words for titles and screen readers: "about $11.6B", "up to about $6.5B", "more than $100B". */
export function formatAmountWords(amount: AmountLike): string {
  if (amount.value === null) return "undisclosed";
  const words = BASIS_TEXT[amount.basis].words;
  return `${words ? `${words} ` : ""}${money({ ...amount, value: amount.value })}`;
}

export function formatQuantity(quantity: Pick<Quantity, "value" | "measure" | "basis">, withLabel = false): string {
  const { unit, label } = QUANTITY_UNITS[quantity.measure];
  const number = quantity.value.toLocaleString("en-US", { maximumFractionDigits: 2 });
  return `${basisSymbol(quantity.basis)}${number} ${unit}${withLabel ? ` ${label}` : ""}`;
}

export function formatQuantityWords(quantity: Pick<Quantity, "value" | "measure" | "basis">): string {
  const { unit, label } = QUANTITY_UNITS[quantity.measure];
  const words = BASIS_TEXT[quantity.basis].words;
  return `${words ? `${words} ` : ""}${quantity.value.toLocaleString("en-US", { maximumFractionDigits: 2 })} ${unit} ${label}`.trim();
}

const SHORT_DATE = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" });
const MONTH_YEAR = new Intl.DateTimeFormat("en-US", { month: "short", year: "numeric", timeZone: "UTC" });

/** "2026-05-28" -> "May 28, 2026"; "2021-01" -> "Jan 2021"; "2021" -> "2021". */
export function formatPartialDate(date: string): string {
  if (/^\d{4}$/.test(date)) return date;
  if (/^\d{4}-\d{2}$/.test(date)) return MONTH_YEAR.format(new Date(`${date}-01T00:00:00Z`));
  return SHORT_DATE.format(new Date(`${date.slice(0, 10)}T00:00:00Z`));
}

/** "2026-05-28" -> "May 2026". */
export function formatMonthYear(date: string): string {
  return MONTH_YEAR.format(new Date(`${date.slice(0, 7)}-01T00:00:00Z`));
}

/** "7 yrs", "through May 31, 2031", "beginning in 2027". */
export function formatTerm(term: { years?: number; through?: string; starts?: string } | undefined): string | null {
  if (!term) return null;
  const parts: string[] = [];
  if (term.years) parts.push(`${term.years} ${term.years === 1 ? "yr" : "yrs"}`);
  if (term.through) parts.push(`through ${formatPartialDate(term.through)}`);
  if (term.starts) parts.push(term.starts);
  return parts.length > 0 ? parts.join(", ") : null;
}

/** "Anthropic" -> "A", "Mistral AI" -> "MA", "Constellation Energy" -> "CE". */
export function monogram(name: string): string {
  const words = name
    .replace(/[^A-Za-z0-9 .-]/g, "")
    .split(/[\s-]+/)
    .filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0].charAt(0).toUpperCase();
  return (words[0].charAt(0) + words[1].charAt(0)).toUpperCase();
}

/** A ledger total: "$95B", "≈$30.6B", ">$100B", "≤$6.5B". null value prints as an em dash. */
export function formatTotal(t: { value: number | null; currency: string; family: "point" | "ceiling" | "floor" | null; approximate: boolean }): string {
  if (t.value === null) return "—";
  const prefix = t.family === "floor" ? ">" : t.family === "ceiling" ? "≤" : t.approximate ? "≈" : "";
  return `${prefix}${formatMoney(t.value, t.currency)}`;
}

/** Short label for a source link: "8-K", "Form D", or the publisher. */
export function sourceShortLabel(source: Pick<Source, "filing" | "publisher">): string {
  if (source.filing) return source.filing.form === "D" ? "Form D" : source.filing.form;
  return source.publisher;
}
