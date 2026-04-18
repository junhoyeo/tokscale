import { and, desc, eq } from "drizzle-orm";
import { db, submissionReviews, users } from "./db";
import {
  applyTrustedSubmission,
  type SubmissionTransaction,
} from "./submissionPersistence";
import {
  SUBMISSION_TRUST_STATE,
  type SubmissionTrustState,
} from "./validation/submissionTrust";
import { validateSubmission } from "./validation/submission";

export const REVIEW_ARTIFACT_RESULT_KIND = {
  UPDATED: "updated",
  NOT_FOUND: "not_found",
  CONFLICT: "conflict",
} as const;

export type ReviewArtifactResultKind =
  (typeof REVIEW_ARTIFACT_RESULT_KIND)[keyof typeof REVIEW_ARTIFACT_RESULT_KIND];

export const REVIEW_FILTER_STATE = {
  REVIEW_REQUIRED: SUBMISSION_TRUST_STATE.REVIEW_REQUIRED,
  TRUSTED: SUBMISSION_TRUST_STATE.TRUSTED,
  REJECTED: SUBMISSION_TRUST_STATE.REJECTED,
} as const;

export type ReviewFilterState =
  (typeof REVIEW_FILTER_STATE)[keyof typeof REVIEW_FILTER_STATE];

export interface SubmissionReviewArtifactSummary {
  id: string;
  user: {
    id: string;
    username: string;
    displayName: string | null;
    avatarUrl: string | null;
  };
  trustState: SubmissionTrustState;
  reasonCodes: string[];
  totals: {
    totalTokens: number;
    totalCost: number;
    activeDays: number;
  };
  dateRange: {
    start: string;
    end: string;
  };
  clients: string[];
  models: string[];
  createdAt: string;
  updatedAt: string;
  review: {
    reviewedAt: string | null;
    reviewedByUsername: string | null;
    reviewNote: string | null;
  };
}

export interface SubmissionReviewArtifactDetail
  extends SubmissionReviewArtifactSummary {
  submissionHash: string | null;
  payload: Record<string, unknown>;
  audit: {
    cliVersion: string | null;
    schemaVersion: number;
  };
}

type SubmissionReviewRow = {
  id: string;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  submissionHash: string | null;
  trustState: string;
  reasonCodes: string[] | null;
  payload: Record<string, unknown>;
  totalTokens: number | string;
  totalCost: number | string;
  activeDays: number | string;
  dateStart: string;
  dateEnd: string;
  sourcesUsed: string[] | null;
  modelsUsed: string[] | null;
  cliVersion: string | null;
  schemaVersion: number | null;
  createdAt: Date;
  updatedAt: Date;
  reviewedAt: Date | null;
  reviewedByUsername: string | null;
  reviewNote: string | null;
};

type SubmissionReviewSummaryRow = Omit<
  SubmissionReviewRow,
  "submissionHash" | "payload" | "cliVersion" | "schemaVersion"
>;

function mapSubmissionReviewSummary(
  row: SubmissionReviewSummaryRow
): SubmissionReviewArtifactSummary {
  return {
    id: row.id,
    user: {
      id: row.userId,
      username: row.username,
      displayName: row.displayName,
      avatarUrl: row.avatarUrl,
    },
    trustState: row.trustState as SubmissionTrustState,
    reasonCodes: row.reasonCodes ?? [],
    totals: {
      totalTokens: Number(row.totalTokens) || 0,
      totalCost: Number(row.totalCost) || 0,
      activeDays: Number(row.activeDays) || 0,
    },
    dateRange: {
      start: row.dateStart,
      end: row.dateEnd,
    },
    clients: row.sourcesUsed ?? [],
    models: row.modelsUsed ?? [],
    createdAt: row.createdAt.toISOString(),
    updatedAt: row.updatedAt.toISOString(),
    review: {
      reviewedAt: row.reviewedAt?.toISOString() ?? null,
      reviewedByUsername: row.reviewedByUsername,
      reviewNote: row.reviewNote,
    },
  };
}

function mapSubmissionReviewArtifact(
  row: SubmissionReviewRow
): SubmissionReviewArtifactDetail {
  return {
    ...mapSubmissionReviewSummary(row),
    submissionHash: row.submissionHash,
    payload: row.payload,
    audit: {
      cliVersion: row.cliVersion,
      schemaVersion: Number(row.schemaVersion) || 0,
    },
  };
}

function buildSubmissionReviewSummarySelect() {
  return {
    id: submissionReviews.id,
    userId: submissionReviews.userId,
    username: users.username,
    displayName: users.displayName,
    avatarUrl: users.avatarUrl,
    trustState: submissionReviews.trustState,
    reasonCodes: submissionReviews.reasonCodes,
    totalTokens: submissionReviews.totalTokens,
    totalCost: submissionReviews.totalCost,
    activeDays: submissionReviews.activeDays,
    dateStart: submissionReviews.dateStart,
    dateEnd: submissionReviews.dateEnd,
    sourcesUsed: submissionReviews.sourcesUsed,
    modelsUsed: submissionReviews.modelsUsed,
    createdAt: submissionReviews.createdAt,
    updatedAt: submissionReviews.updatedAt,
    reviewedAt: submissionReviews.reviewedAt,
    reviewedByUsername: submissionReviews.reviewedByUsername,
    reviewNote: submissionReviews.reviewNote,
  };
}

function buildSubmissionReviewSelect() {
  return {
    id: submissionReviews.id,
    userId: submissionReviews.userId,
    username: users.username,
    displayName: users.displayName,
    avatarUrl: users.avatarUrl,
    submissionHash: submissionReviews.submissionHash,
    trustState: submissionReviews.trustState,
    reasonCodes: submissionReviews.reasonCodes,
    payload: submissionReviews.payload,
    totalTokens: submissionReviews.totalTokens,
    totalCost: submissionReviews.totalCost,
    activeDays: submissionReviews.activeDays,
    dateStart: submissionReviews.dateStart,
    dateEnd: submissionReviews.dateEnd,
    sourcesUsed: submissionReviews.sourcesUsed,
    modelsUsed: submissionReviews.modelsUsed,
    cliVersion: submissionReviews.cliVersion,
    schemaVersion: submissionReviews.schemaVersion,
    createdAt: submissionReviews.createdAt,
    updatedAt: submissionReviews.updatedAt,
    reviewedAt: submissionReviews.reviewedAt,
    reviewedByUsername: submissionReviews.reviewedByUsername,
    reviewNote: submissionReviews.reviewNote,
  };
}

function normalizeReviewNote(reviewNote: string | null | undefined): string | null {
  if (typeof reviewNote !== "string") {
    return null;
  }

  const trimmed = reviewNote.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export async function listSubmissionReviewArtifacts(options?: {
  trustState?: ReviewFilterState;
}): Promise<SubmissionReviewArtifactSummary[]> {
  const baseQuery = db
    .select(buildSubmissionReviewSummarySelect())
    .from(submissionReviews)
    .innerJoin(users, eq(submissionReviews.userId, users.id));

  const rows = options?.trustState
    ? await baseQuery
        .where(eq(submissionReviews.trustState, options.trustState))
        .orderBy(desc(submissionReviews.updatedAt))
    : await baseQuery.orderBy(desc(submissionReviews.updatedAt));

  return rows.map(mapSubmissionReviewSummary);
}

export async function getSubmissionReviewArtifact(
  reviewId: string
): Promise<SubmissionReviewArtifactDetail | null> {
  const [row] = await db
    .select(buildSubmissionReviewSelect())
    .from(submissionReviews)
    .innerJoin(users, eq(submissionReviews.userId, users.id))
    .where(eq(submissionReviews.id, reviewId))
    .limit(1);

  return row ? mapSubmissionReviewArtifact(row) : null;
}

export type SubmissionReviewAdjudicationResult =
  | {
      kind: typeof REVIEW_ARTIFACT_RESULT_KIND.UPDATED;
      artifact: SubmissionReviewArtifactDetail;
      competitiveWriteApplied: boolean;
    }
  | {
      kind: typeof REVIEW_ARTIFACT_RESULT_KIND.NOT_FOUND;
    }
  | {
      kind: typeof REVIEW_ARTIFACT_RESULT_KIND.CONFLICT;
      currentTrustState: SubmissionTrustState;
    };

export async function adjudicateSubmissionReview({
  reviewId,
  trustState,
  reviewedByUsername,
  reviewNote,
}: {
  reviewId: string;
  trustState: typeof SUBMISSION_TRUST_STATE.TRUSTED | typeof SUBMISSION_TRUST_STATE.REJECTED;
  reviewedByUsername: string;
  reviewNote?: string | null;
}): Promise<SubmissionReviewAdjudicationResult> {
  return db.transaction(async (tx: SubmissionTransaction) => {
    const [currentReview] = await tx
      .select(buildSubmissionReviewSelect())
      .from(submissionReviews)
      .innerJoin(users, eq(submissionReviews.userId, users.id))
      .where(eq(submissionReviews.id, reviewId))
      .for("update")
      .limit(1);

    if (!currentReview) {
      return {
        kind: REVIEW_ARTIFACT_RESULT_KIND.NOT_FOUND,
      };
    }

    const currentTrustState =
      currentReview.trustState as SubmissionTrustState;
    if (currentTrustState !== SUBMISSION_TRUST_STATE.REVIEW_REQUIRED) {
      return {
        kind: REVIEW_ARTIFACT_RESULT_KIND.CONFLICT,
        currentTrustState,
      };
    }

    if (trustState === SUBMISSION_TRUST_STATE.TRUSTED) {
      const validation = validateSubmission(currentReview.payload);
      if (!validation.valid || !validation.data) {
        throw new Error("Stored submission review payload failed validation");
      }

      const rawMcpServers = currentReview.payload.mcpServers;
      const mcpServers = Array.isArray(rawMcpServers)
        ? rawMcpServers.filter(
            (server): server is string => typeof server === "string"
          )
        : null;

      await applyTrustedSubmission(tx, {
        userId: currentReview.userId,
        data: validation.data,
        mcpServers,
      });
    }

    const adjudicatedAt = new Date();
    const [updatedReview] = await tx
      .update(submissionReviews)
      .set({
        trustState,
        reviewedAt: adjudicatedAt,
        reviewedByUsername,
        reviewNote: normalizeReviewNote(reviewNote),
        updatedAt: adjudicatedAt,
      })
      .where(
        and(
          eq(submissionReviews.id, reviewId),
          eq(
            submissionReviews.trustState,
            SUBMISSION_TRUST_STATE.REVIEW_REQUIRED
          )
        )
      )
      .returning({
        trustState: submissionReviews.trustState,
        updatedAt: submissionReviews.updatedAt,
        reviewedAt: submissionReviews.reviewedAt,
        reviewedByUsername: submissionReviews.reviewedByUsername,
        reviewNote: submissionReviews.reviewNote,
      });

    if (!updatedReview) {
      throw new Error("Submission review changed during adjudication");
    }

    const hydratedReview: SubmissionReviewRow = {
      ...currentReview,
      trustState: updatedReview.trustState,
      updatedAt: updatedReview.updatedAt,
      reviewedAt: updatedReview.reviewedAt,
      reviewedByUsername: updatedReview.reviewedByUsername,
      reviewNote: updatedReview.reviewNote,
    };

    return {
      kind: REVIEW_ARTIFACT_RESULT_KIND.UPDATED,
      artifact: mapSubmissionReviewArtifact(hydratedReview),
      competitiveWriteApplied: trustState === SUBMISSION_TRUST_STATE.TRUSTED,
    };
  });
}
