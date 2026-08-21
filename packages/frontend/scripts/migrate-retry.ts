// Classify a failed `drizzle-kit migrate` run as transient (worth retrying
// with a fresh process/connection) or not. Extracted from migrate-prod.ts so
// the classification is unit-testable without running the migration side
// effects that module performs on import.
//
// Two transient classes:
//   - Postgres deadlock_detected (SQLSTATE 40P01).
//   - A dropped/reset DB connection. drizzle-kit runs each migration inside a
//     transaction, so a mid-flight connection loss (e.g. a managed-Postgres
//     proxy severing the socket during a slow index build / ADD CONSTRAINT)
//     rolls the migration back with no partial state -- the postgres driver
//     surfaces it as CONNECTION_CLOSED / ECONNRESET / "Connection terminated"
//     rather than a SQLSTATE. Re-running from a fresh process reconnects and
//     re-attempts the still-pending migration idempotently.
// Anything else (a real SQL error) is non-retryable and fails the build.
export function classifyFailure(stderr: string): { retryable: boolean; reason: string } {
  if (/40P01|deadlock detected/i.test(stderr)) {
    return { retryable: true, reason: "deadlock" };
  }
  if (
    // Transient loss of connectivity worth reconnecting for: postgres.js
    // surfaces an unexpectedly dropped socket as CONNECTION_CLOSED and a
    // failed connect as CONNECT_TIMEOUT; Node reports socket-level failures
    // (ECONN* / ETIMEDOUT / EPIPE) and transient DNS failures (ENOTFOUND /
    // EAI_AGAIN) via these errnos; the text variants are server-initiated
    // terminations (restart / failover). Deliberately excluded are
    // CONNECTION_ENDED / CONNECTION_DESTROYED -- postgres.js emits those on a
    // caller-initiated pool shutdown, i.e. deterministic, not a transient
    // outage.
    /CONNECTION_CLOSED|CONNECT_TIMEOUT|ECONNRESET|ECONNREFUSED|ETIMEDOUT|ENOTFOUND|EAI_AGAIN|EPIPE|connection terminated|terminating connection|server closed the connection/i.test(
      stderr
    )
  ) {
    return { retryable: true, reason: "connection error" };
  }
  return { retryable: false, reason: "non-retryable error" };
}
