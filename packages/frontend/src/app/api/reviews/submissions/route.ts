import { NextResponse } from "next/server";
import { authenticateSubmissionReviewOperator } from "@/lib/submissionReviewAuth";
import {
  listSubmissionReviewArtifacts,
  REVIEW_FILTER_STATE,
  type ReviewFilterState,
} from "@/lib/submissionReviews";

function isReviewFilterState(value: string | null): value is ReviewFilterState {
  if (!value) {
    return false;
  }

  return Object.values(REVIEW_FILTER_STATE).includes(value as ReviewFilterState);
}


export async function GET(request: Request) {
  try {
    const operator = authenticateSubmissionReviewOperator(request);
    if (!operator) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const requestedTrustState = new URL(request.url).searchParams.get("trustState");
    const trustState = isReviewFilterState(requestedTrustState)
      ? requestedTrustState
      : REVIEW_FILTER_STATE.REVIEW_REQUIRED;

    const artifacts = await listSubmissionReviewArtifacts({ trustState });

    return NextResponse.json({
      reviews: artifacts,
    });
  } catch (error) {
    console.error("Submission reviews list error:", error);
    return NextResponse.json(
      { error: "Failed to load submission reviews" },
      { status: 500 }
    );
  }
}
