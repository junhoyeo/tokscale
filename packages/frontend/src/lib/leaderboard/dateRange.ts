const DATE_REGEX = /^\d{4}-\d{2}-\d{2}$/;

export interface CustomDateRange {
  from: string;
  to: string;
}

export function isValidDateString(value: string | null | undefined): value is string {
  if (!value || !DATE_REGEX.test(value)) {
    return false;
  }

  const [year, month, day] = value.split("-").map(Number);
  const parsedDate = new Date(year, month - 1, day);

  return (
    parsedDate.getFullYear() === year &&
    parsedDate.getMonth() === month - 1 &&
    parsedDate.getDate() === day
  );
}

export function isValidCustomDateRange(
  from: string | null | undefined,
  to: string | null | undefined
): boolean {
  return parseCustomDateRange(from, to) !== null;
}

export function parseCustomDateRange(
  from: string | null | undefined,
  to: string | null | undefined
): CustomDateRange | null {
  if (!isValidDateString(from) || !isValidDateString(to)) {
    return null;
  }

  // Lexicographic comparison is correct here because isValidDateString above
  // enforces the YYYY-MM-DD format, making string order identical to date order.
  if (from > to) {
    return null;
  }

  return { from, to };
}
