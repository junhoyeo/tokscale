import { afterEach, describe, expect, it, vi } from "vitest";
import { authenticateSubmissionReviewOperator } from "../../src/lib/submissionReviewAuth";

const REVIEW_TOKEN = "review-token-with-at-least-32-bytes";

function createRequest(token?: string): Request {
  return new Request("http://localhost:3000/api/reviews/submissions", {
    headers: token ? { Authorization: `Bearer ${token}` } : undefined,
  });
}

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("authenticateSubmissionReviewOperator", () => {
  it("fails closed when the dedicated operator token is not configured", () => {
    vi.stubEnv("SUBMISSION_REVIEW_API_TOKEN", "");
    expect(
      authenticateSubmissionReviewOperator(createRequest(REVIEW_TOKEN))
    ).toBeNull();
  });

  it("rejects missing and incorrect bearer credentials", () => {
    vi.stubEnv("SUBMISSION_REVIEW_API_TOKEN", REVIEW_TOKEN);

    expect(authenticateSubmissionReviewOperator(createRequest())).toBeNull();
    expect(
      authenticateSubmissionReviewOperator(createRequest("wrong-token"))
    ).toBeNull();
  });

  it("returns the configured audit identity for the operator token", () => {
    vi.stubEnv("SUBMISSION_REVIEW_API_TOKEN", REVIEW_TOKEN);
    vi.stubEnv("SUBMISSION_REVIEW_OPERATOR_USERNAME", "release-moderator");

    expect(
      authenticateSubmissionReviewOperator(createRequest(REVIEW_TOKEN))
    ).toEqual({ username: "release-moderator" });
  });

  it("uses a bounded fallback audit identity", () => {
    vi.stubEnv("SUBMISSION_REVIEW_API_TOKEN", REVIEW_TOKEN);
    vi.stubEnv("SUBMISSION_REVIEW_OPERATOR_USERNAME", "x".repeat(40));

    expect(
      authenticateSubmissionReviewOperator(createRequest(REVIEW_TOKEN))
    ).toEqual({ username: "submission-review-operator" });
  });
});
