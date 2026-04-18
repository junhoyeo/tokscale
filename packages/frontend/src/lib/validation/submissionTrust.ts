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
  PARTIAL_TIMESTAMP_COVERAGE: "partial_timestamp_coverage",
} as const;

export type SubmissionReasonCode =
  (typeof SUBMISSION_REASON_CODE)[keyof typeof SUBMISSION_REASON_CODE];

export interface SubmissionTrustAssessment {
  trustState: SubmissionTrustState;
  reasonCodes: SubmissionReasonCode[];
  rejectionReasonCodes: SubmissionReasonCode[];
  errors: string[];
  warnings: string[];
}

const TRUSTED_RETROACTIVE_WINDOW_DAYS = 30;

function getUtcDateStringFromTimestamp(timestampMs: number): string {
  return new Date(timestampMs).toISOString().slice(0, 10);
}

function getRetroactiveThresholdDate(now: Date): string {
  const utcMidnight = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate())
  );
  utcMidnight.setUTCDate(utcMidnight.getUTCDate() - TRUSTED_RETROACTIVE_WINDOW_DAYS);
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
  const retroactiveThresholdDate = getRetroactiveThresholdDate(now);

  let sawTimestampedDay = false;
  let sawUntimestampedDay = false;

  for (const day of submission.contributions) {
    if (day.timestampMs != null) {
      sawTimestampedDay = true;
      const timestampDate = getUtcDateStringFromTimestamp(day.timestampMs);
      if (timestampDate !== day.date) {
        rejectionReasonCodes.add(SUBMISSION_REASON_CODE.TIMESTAMP_DAY_MISMATCH);
        errors.push(
          `Day ${day.date} has timestamp ${day.timestampMs} outside its claimed UTC bucket`
        );
      }
    } else {
      sawUntimestampedDay = true;
      if (day.date < retroactiveThresholdDate) {
        reviewReasonCodes.add(
          SUBMISSION_REASON_CODE.HISTORICAL_DAY_MISSING_TIMESTAMP
        );
        warnings.push(
          `Day ${day.date} is older than ${TRUSTED_RETROACTIVE_WINDOW_DAYS} days and has no timestampMs audit metadata`
        );
      }
    }

    for (const client of day.clients) {
      const availabilityDate = extractDatedModelAvailability(client.modelId);
      if (availabilityDate && day.date < availabilityDate) {
        rejectionReasonCodes.add(
          SUBMISSION_REASON_CODE.MODEL_PREDATES_PUBLIC_AVAILABILITY
        );
        errors.push(
          `Model ${client.modelId} cannot be submitted for ${day.date} before ${availabilityDate}`
        );
      }
    }
  }

  if (sawTimestampedDay && sawUntimestampedDay) {
    reviewReasonCodes.add(SUBMISSION_REASON_CODE.PARTIAL_TIMESTAMP_COVERAGE);
    warnings.push(
      "Submission mixes timestamped and untimestamped contribution days; review is required before trusting the full history"
    );
  }

  if (rejectionReasonCodes.size > 0) {
    return {
      trustState: SUBMISSION_TRUST_STATE.REJECTED,
      reasonCodes: [],
      rejectionReasonCodes: Array.from(rejectionReasonCodes),
      errors,
      warnings,
    };
  }

  if (reviewReasonCodes.size > 0) {
    return {
      trustState: SUBMISSION_TRUST_STATE.REVIEW_REQUIRED,
      reasonCodes: Array.from(reviewReasonCodes),
      rejectionReasonCodes: [],
      errors: [],
      warnings,
    };
  }

  return {
    trustState: SUBMISSION_TRUST_STATE.TRUSTED,
    reasonCodes: [],
    rejectionReasonCodes: [],
    errors: [],
    warnings: [],
  };
}
