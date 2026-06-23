import { format } from "date-fns";

function formatDateFromString(dateStr: string): string {
  const [year, month, day] = dateStr.split("-").map(Number);
  return format(new Date(year, month - 1, day), "MMM d, yyyy");
}

function formatDateFullFromString(dateStr: string): string {
  const [year, month, day] = dateStr.split("-").map(Number);
  return format(new Date(year, month - 1, day), "MMMM d, yyyy");
}

export function getContributionLocalDate(contrib: { date: string; timestampMs?: number | null }): string {
  if (contrib.timestampMs != null) {
    const d = new Date(contrib.timestampMs);
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
  }
  return contrib.date;
}

export function formatContributionDate(contrib: { date: string; timestampMs?: number | null }): string {
  return formatDateFromString(getContributionLocalDate(contrib));
}

export function formatContributionDateFull(contrib: { date: string; timestampMs?: number | null }): string {
  return formatDateFullFromString(getContributionLocalDate(contrib));
}
