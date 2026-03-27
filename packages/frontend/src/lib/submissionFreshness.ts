export const DEFAULT_SUBMISSION_FRESHNESS_DAYS = 30;

export interface SubmissionFreshness {
  lastUpdated: string;
  cliVersion: string | null;
  schemaVersion: number;
  isStale: boolean;
}

interface SubmissionFreshnessInput {
  updatedAt: Date | string | null | undefined;
  cliVersion?: string | null;
  schemaVersion?: number | null;
}

export function getSubmissionFreshnessWindowDays(): number {
  const rawValue = process.env.SUBMISSION_FRESHNESS_DAYS;
  if (!rawValue) {
    return DEFAULT_SUBMISSION_FRESHNESS_DAYS;
  }

  const parsed = Number(rawValue);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return DEFAULT_SUBMISSION_FRESHNESS_DAYS;
  }

  return Math.max(1, Math.floor(parsed));
}

export function isSubmissionStale(
  updatedAt: Date | string,
  now: Date = new Date(),
  freshnessWindowDays: number = getSubmissionFreshnessWindowDays()
): boolean {
  const updatedAtDate = updatedAt instanceof Date ? updatedAt : new Date(updatedAt);
  const updatedAtTime = updatedAtDate.getTime();

  if (Number.isNaN(updatedAtTime)) {
    return false;
  }

  const freshnessWindowMs = freshnessWindowDays * 24 * 60 * 60 * 1000;
  return now.getTime() - updatedAtTime > freshnessWindowMs;
}

export function buildSubmissionFreshness(
  input: SubmissionFreshnessInput | null | undefined,
  now: Date = new Date(),
  freshnessWindowDays: number = getSubmissionFreshnessWindowDays()
): SubmissionFreshness | null {
  if (!input?.updatedAt) {
    return null;
  }

  const updatedAtDate = input.updatedAt instanceof Date
    ? input.updatedAt
    : new Date(input.updatedAt);

  if (Number.isNaN(updatedAtDate.getTime())) {
    return null;
  }

  const updatedAt = updatedAtDate.toISOString();

  return {
    lastUpdated: updatedAt,
    cliVersion: input.cliVersion ?? null,
    schemaVersion: input.schemaVersion ?? 0,
    isStale: isSubmissionStale(updatedAt, now, freshnessWindowDays),
  };
}
