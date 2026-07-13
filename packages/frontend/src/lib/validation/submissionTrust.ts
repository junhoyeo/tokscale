import type { SubmissionData } from "./submission";

export const SUBMISSION_TRUST_STATE = {
  TRUSTED: "trusted",
  REVIEW_REQUIRED: "review_required",
  REJECTED: "rejected",
} as const;

export type SubmissionTrustState =
  (typeof SUBMISSION_TRUST_STATE)[keyof typeof SUBMISSION_TRUST_STATE];

export const SUBMISSION_REASON_CODE = {
  TIMESTAMP_DAY_MISMATCH: "timestamp_day_mismatch",
  MODEL_PREDATES_PUBLIC_AVAILABILITY: "model_predates_public_availability",
  HISTORICAL_DAY_MISSING_TIMESTAMP: "historical_day_missing_timestamp",
} as const;

export type SubmissionReasonCode =
  (typeof SUBMISSION_REASON_CODE)[keyof typeof SUBMISSION_REASON_CODE];

export interface SubmissionTrustAssessment {
  trustState: SubmissionTrustState;
  reasonCodes: SubmissionReasonCode[];
  rejectionReasonCodes: SubmissionReasonCode[];
  reviewDates: string[];
  errors: string[];
  warnings: string[];
}

const TRUSTED_RETROACTIVE_WINDOW_DAYS = 30;

function getUtcDateStringFromTimestamp(timestampMs: number): string | null {
  if (!Number.isFinite(timestampMs)) {
    return null;
  }

  const timestamp = new Date(timestampMs);
  if (Number.isNaN(timestamp.getTime())) {
    return null;
  }

  return timestamp.toISOString().slice(0, 10);
}

function getRetroactiveThresholdDate(now: Date): string {
  const utcMidnight = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate())
  );
  utcMidnight.setUTCDate(
    utcMidnight.getUTCDate() - TRUSTED_RETROACTIVE_WINDOW_DAYS
  );
  return utcMidnight.toISOString().slice(0, 10);
}

function extractDatedModelAvailability(modelId: string): string | null {
  const match =
    modelId.match(/(?:^|[-_])(20\d{2})(\d{2})(\d{2})(?:$|[-_])/) ??
    modelId.match(/(?:^|[-_])(20\d{2})[-_](\d{2})[-_](\d{2})(?:$|[-_])/);
  if (!match) {
    return null;
  }

  const [, year, month, day] = match;
  const parsedYear = Number(year);
  const parsedMonth = Number(month);
  const parsedDay = Number(day);
  if (parsedMonth < 1 || parsedMonth > 12 || parsedDay < 1 || parsedDay > 31) {
    return null;
  }

  const parsedDate = new Date(Date.UTC(parsedYear, parsedMonth - 1, parsedDay));
  if (
    parsedDate.getUTCFullYear() !== parsedYear ||
    parsedDate.getUTCMonth() + 1 !== parsedMonth ||
    parsedDate.getUTCDate() !== parsedDay
  ) {
    return null;
  }

  return `${year}-${month}-${day}`;
}

export function assessSubmissionTrust(
  submission: SubmissionData,
  now: Date = new Date()
): SubmissionTrustAssessment {
  const errors: string[] = [];
  const warnings: string[] = [];
  const rejectionReasonCodes = new Set<SubmissionReasonCode>();
  const reviewReasonCodes = new Set<SubmissionReasonCode>();
  const reviewDates = new Set<string>();
  const retroactiveThresholdDate = getRetroactiveThresholdDate(now);

  for (const day of submission.contributions) {
    if (day.timestampMs != null) {
      const timestampDate = getUtcDateStringFromTimestamp(day.timestampMs);
      if (timestampDate == null) {
        rejectionReasonCodes.add(SUBMISSION_REASON_CODE.TIMESTAMP_DAY_MISMATCH);
        errors.push(`Day ${day.date} has invalid timestampMs ${day.timestampMs}`);
      } else if (timestampDate !== day.date) {
        rejectionReasonCodes.add(SUBMISSION_REASON_CODE.TIMESTAMP_DAY_MISMATCH);
        errors.push(
          `Day ${day.date} has timestamp ${day.timestampMs} outside its claimed UTC bucket`
        );
      }
    } else if (day.date < retroactiveThresholdDate) {
      reviewDates.add(day.date);
      reviewReasonCodes.add(
        SUBMISSION_REASON_CODE.HISTORICAL_DAY_MISSING_TIMESTAMP
      );
      warnings.push(
        `Day ${day.date} is older than ${TRUSTED_RETROACTIVE_WINDOW_DAYS} days and has no timestampMs audit metadata`
      );
    }

    for (const client of day.clients) {
      const availabilityDate = extractDatedModelAvailability(client.modelId);
      if (availabilityDate && day.date < availabilityDate) {
        reviewDates.add(day.date);
        reviewReasonCodes.add(
          SUBMISSION_REASON_CODE.MODEL_PREDATES_PUBLIC_AVAILABILITY
        );
        warnings.push(
          `Model ${client.modelId} is reported for ${day.date} before its parsed availability date ${availabilityDate}`
        );
      }
    }
  }

  if (rejectionReasonCodes.size > 0) {
    return {
      trustState: SUBMISSION_TRUST_STATE.REJECTED,
      reasonCodes: [],
      rejectionReasonCodes: Array.from(rejectionReasonCodes),
      reviewDates: [],
      errors,
      warnings,
    };
  }

  if (reviewDates.size > 0) {
    return {
      trustState: SUBMISSION_TRUST_STATE.REVIEW_REQUIRED,
      reasonCodes: Array.from(reviewReasonCodes),
      rejectionReasonCodes: [],
      reviewDates: Array.from(reviewDates).sort(),
      errors: [],
      warnings,
    };
  }

  return {
    trustState: SUBMISSION_TRUST_STATE.TRUSTED,
    reasonCodes: [],
    rejectionReasonCodes: [],
    reviewDates: [],
    errors: [],
    warnings: [],
  };
}

type SubmissionDay = SubmissionData["contributions"][number];

export function subsetSubmissionByDates(
  submission: SubmissionData,
  dates: ReadonlySet<string>
): SubmissionData | null {
  const allContributions: SubmissionData["contributions"] =
    submission.contributions;
  const contributions: SubmissionData["contributions"] = allContributions
    .filter((day: SubmissionDay) => dates.has(day.date))
    .sort((a: SubmissionDay, b: SubmissionDay) =>
      a.date.localeCompare(b.date)
    );

  if (contributions.length === 0) {
    return null;
  }

  const totalTokens = contributions.reduce(
    (sum, day) => sum + day.totals.tokens,
    0
  );
  const totalCost = contributions.reduce(
    (sum, day) => sum + day.totals.cost,
    0
  );
  const clients = Array.from(
    new Set(contributions.flatMap((day) => day.clients.map((client) => client.client)))
  ).sort();
  const models = Array.from(
    new Set(contributions.flatMap((day) => day.clients.map((client) => client.modelId)))
  ).sort();
  const years = Array.from(
    new Set(contributions.map((day) => day.date.slice(0, 4)))
  )
    .sort()
    .map((year) => {
      const days = contributions.filter((day) => day.date.startsWith(year));
      return {
        year,
        totalTokens: days.reduce((sum, day) => sum + day.totals.tokens, 0),
        totalCost: days.reduce((sum, day) => sum + day.totals.cost, 0),
        range: {
          start: days[0].date,
          end: days[days.length - 1].date,
        },
      };
    });
  const isWholeSubmission =
    contributions.length === allContributions.length;

  return {
    ...submission,
    meta: {
      ...submission.meta,
      dateRange: {
        start: contributions[0].date,
        end: contributions[contributions.length - 1].date,
      },
    },
    summary: {
      totalTokens,
      totalCost,
      totalDays: contributions.length,
      activeDays: contributions.filter((day) => day.totals.tokens > 0).length,
      averagePerDay: totalTokens / contributions.length,
      maxCostInSingleDay: Math.max(
        ...contributions.map((day) => day.totals.cost)
      ),
      clients,
      models,
    },
    years,
    contributions,
    timeMetrics: isWholeSubmission ? submission.timeMetrics : undefined,
  };
}
