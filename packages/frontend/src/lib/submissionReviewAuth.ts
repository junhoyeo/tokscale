import { createHash, timingSafeEqual } from "node:crypto";

const DEFAULT_OPERATOR_USERNAME = "submission-review-operator";

export interface SubmissionReviewOperator {
  username: string;
}

function hashSecret(secret: string): Buffer {
  return createHash("sha256").update(secret).digest();
}

function secretsMatch(candidate: string, configured: string): boolean {
  return timingSafeEqual(hashSecret(candidate), hashSecret(configured));
}

export function authenticateSubmissionReviewOperator(
  request: Request
): SubmissionReviewOperator | null {
  const configuredToken = process.env.SUBMISSION_REVIEW_API_TOKEN;
  if (!configuredToken) {
    return null;
  }

  const authorization = request.headers.get("Authorization");
  if (!authorization?.startsWith("Bearer ")) {
    return null;
  }

  const candidate = authorization.slice("Bearer ".length);
  if (!candidate || !secretsMatch(candidate, configuredToken)) {
    return null;
  }

  const configuredUsername =
    process.env.SUBMISSION_REVIEW_OPERATOR_USERNAME?.trim();
  const username =
    configuredUsername && configuredUsername.length <= 39
      ? configuredUsername
      : DEFAULT_OPERATOR_USERNAME;

  return { username };
}
