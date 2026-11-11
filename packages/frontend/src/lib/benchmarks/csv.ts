/**
 * Small RFC 4180 CSV reader for the benchmark sync. No dependency: quoted
 * fields, doubled quotes, commas and newlines inside quotes, CRLF line ends
 * and a UTF-8 byte-order mark are handled. Rows that are entirely empty are
 * dropped.
 */

export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;
  let i = text.charCodeAt(0) === 0xfeff ? 1 : 0;

  const endField = () => {
    row.push(field);
    field = "";
  };
  const endRow = () => {
    endField();
    if (row.some((value) => value.length > 0)) rows.push(row);
    row = [];
  };

  for (; i < text.length; i++) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          inQuotes = false;
        }
      } else {
        field += ch;
      }
      continue;
    }
    if (ch === '"' && field.length === 0) {
      inQuotes = true;
    } else if (ch === ",") {
      endField();
    } else if (ch === "\n") {
      endRow();
    } else if (ch === "\r") {
      if (text[i + 1] === "\n") i++;
      endRow();
    } else {
      field += ch;
    }
  }
  if (field.length > 0 || row.length > 0) endRow();
  return rows;
}

/** Rows as objects keyed by the header row. Missing trailing cells read as "". */
export function parseCsvRecords(text: string): Array<Record<string, string>> {
  const [header, ...body] = parseCsv(text);
  if (!header) return [];
  return body.map((cells) => {
    const record: Record<string, string> = Object.create(null);
    header.forEach((name, index) => {
      record[name] = cells[index] ?? "";
    });
    return record;
  });
}

/** Parse a CSV number cell. Empty, "nan" and non-finite values return null. */
export function csvNumber(value: string | undefined): number | null {
  if (value === undefined) return null;
  const trimmed = value.trim();
  if (trimmed === "" || /^nan$/i.test(trimmed)) return null;
  const n = Number(trimmed);
  return Number.isFinite(n) ? n : null;
}

/** "2026-09-22", "2026-09-22T10:00:00Z" -> "2026-09-22"; anything else -> null. */
export function csvDate(value: string | undefined): string | null {
  const match = /^(\d{4}-\d{2}-\d{2})/.exec(value?.trim() ?? "");
  return match ? match[1] : null;
}

/** Round a value we computed (not one the source wrote) to 6 decimals, dropping float noise. */
export function roundDerived(value: number): number {
  return Math.round(value * 1e6) / 1e6;
}
