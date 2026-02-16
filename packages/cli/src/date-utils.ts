export function formatDateLocal(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

export function parseDateStringToLocal(dateStr: string): Date | null {
  const match = dateStr.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!match) return null;
  const [, yearStr, monthStr, dayStr] = match;
  const year = parseInt(yearStr);
  const month = parseInt(monthStr) - 1;
  const day = parseInt(dayStr);
  const date = new Date(year, month, day);
  if (date.getFullYear() !== year || date.getMonth() !== month || date.getDate() !== day) {
    return null;
  }
  return date;
}

export function getStartOfDayTimestamp(date: Date): number {
  const start = new Date(date.getFullYear(), date.getMonth(), date.getDate(), 0, 0, 0, 0);
  return start.getTime();
}

export function getEndOfDayTimestamp(date: Date): number {
  const end = new Date(date.getFullYear(), date.getMonth(), date.getDate(), 23, 59, 59, 999);
  return end.getTime();
}

export function getContributionLocalDate(contrib: { date: string; timestamp?: number }): string {
  if (contrib.timestamp != null) {
    return formatDateLocal(new Date(contrib.timestamp));
  }
  return contrib.date;
}

const MIN_VALID_TIMESTAMP_MS = 1_000_000_000_000;
const MAX_VALID_TIMESTAMP_MS = 4_102_444_800_000;

export function validateTimestampMs(ts: number, label: string): number {
  if (!Number.isFinite(ts) || !Number.isSafeInteger(ts)) {
    throw new Error(`${label} must be a finite safe integer, got ${ts}`);
  }
  if (ts < MIN_VALID_TIMESTAMP_MS || ts > MAX_VALID_TIMESTAMP_MS) {
    throw new Error(`${label} out of valid range (${MIN_VALID_TIMESTAMP_MS}..${MAX_VALID_TIMESTAMP_MS}), got ${ts}`);
  }
  return ts;
}
