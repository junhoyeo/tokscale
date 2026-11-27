import { sql } from "drizzle-orm";
import { db } from "@/lib/db";

type DbExecutor = Pick<typeof db, "execute">;

export interface PeriodSubmissionStats {
  /**
   * Number of accepted submissions in the requested UTC date window, or `null`
   * when the window predates daily submission tracking for this account.
   */
  count: number | null;
  /** Earliest UTC date (`YYYY-MM-DD`) tracked for this submission, if any. */
  trackingStartDate: string | null;
}

export function toUtcSubmissionDate(instant: Date): string {
  return instant.toISOString().slice(0, 10);
}

/**
 * Resolve whether a period window has a reliable submission count or predates
 * tracking. A zero sum inside a window starting before the account's first
 * tracked day is treated as unavailable (`null`) so historical months do not
 * falsely report `0` submissions.
 */
export function resolvePeriodSubmissionStats(
  rawPeriodCount: number,
  earliestTrackedDate: string | null,
  windowStart: string,
): PeriodSubmissionStats {
  const safeCount =
    Number.isFinite(rawPeriodCount) && rawPeriodCount > 0
      ? Math.trunc(rawPeriodCount)
      : 0;

  if (safeCount > 0) {
    return {
      count: safeCount,
      trackingStartDate: earliestTrackedDate,
    };
  }

  if (earliestTrackedDate !== null && earliestTrackedDate <= windowStart) {
    return {
      count: 0,
      trackingStartDate: earliestTrackedDate,
    };
  }

  return {
    count: null,
    trackingStartDate: earliestTrackedDate,
  };
}

/**
 * Increment the per-UTC-day submission counter inside the submit transaction.
 */
export async function recordDailySubmissionCount(
  executor: DbExecutor,
  submissionId: string,
  submittedAt: Date,
): Promise<void> {
  const utcDate = toUtcSubmissionDate(submittedAt);
  await executor.execute(sql`
    INSERT INTO submission_daily_counts (submission_id, date, submit_count)
    VALUES (${submissionId}, ${utcDate}, 1)
    ON CONFLICT (submission_id, date)
    DO UPDATE SET submit_count = submission_daily_counts.submit_count + 1
  `);
}

/**
 * Query period-scoped submission counts over an inclusive UTC date window
 * `[windowStart, windowEnd]` using the primary key `(submission_id, date)`.
 */
export async function getPeriodSubmissionStats(
  submissionId: string,
  windowStart: string,
  windowEnd: string,
  executor: DbExecutor = db,
): Promise<PeriodSubmissionStats> {
  const rows = await executor.execute<{
    period_count: number | string | null;
    earliest_tracked_date: string | null;
  }>(sql`
    SELECT
      COALESCE(
        SUM(
          CASE
            WHEN date >= ${windowStart} AND date <= ${windowEnd} THEN submit_count
            ELSE 0
          END
        ),
        0
      )::int AS period_count,
      MIN(date)::text AS earliest_tracked_date
    FROM submission_daily_counts
    WHERE submission_id = ${submissionId}
  `);

  const row = (
    rows as unknown as Array<{
      period_count?: number | string | null;
      earliest_tracked_date?: string | null;
    }>
  )?.[0];

  const periodCount = Number(row?.period_count ?? 0);
  const earliestTrackedDate =
    typeof row?.earliest_tracked_date === "string" &&
    row.earliest_tracked_date.length > 0
      ? row.earliest_tracked_date.slice(0, 10)
      : null;

  return resolvePeriodSubmissionStats(
    periodCount,
    earliestTrackedDate,
    windowStart,
  );
}
