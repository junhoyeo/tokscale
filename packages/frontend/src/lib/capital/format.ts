/** "2026-q2" -> "Q2 2026". */
export function quarterLabel(key: string): string {
  const [year, q] = key.split("-");
  return `${q.toUpperCase()} ${year}`;
}
