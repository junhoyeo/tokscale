import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_SUBMISSION_FRESHNESS_DAYS,
  buildSubmissionFreshness,
  getSubmissionFreshnessWindowDays,
  isSubmissionStale,
} from "../../src/lib/submissionFreshness";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllEnvs();
});

describe("submission freshness", () => {
  it("marks submissions older than the default window as stale", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-11T12:00:00.000Z"));

    expect(getSubmissionFreshnessWindowDays()).toBe(DEFAULT_SUBMISSION_FRESHNESS_DAYS);
    expect(isSubmissionStale("2026-02-08T11:59:59.000Z")).toBe(true);
    expect(isSubmissionStale("2026-02-09T12:00:00.000Z")).toBe(false);
  });

  it("respects the configured freshness window", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-11T12:00:00.000Z"));
    vi.stubEnv("SUBMISSION_FRESHNESS_DAYS", "7");

    expect(getSubmissionFreshnessWindowDays()).toBe(7);
    expect(isSubmissionStale("2026-03-03T11:59:59.000Z")).toBe(true);
    expect(isSubmissionStale("2026-03-04T12:00:00.000Z")).toBe(false);
  });

  it("clamps fractional positive freshness windows to at least one day", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-11T12:00:00.000Z"));
    vi.stubEnv("SUBMISSION_FRESHNESS_DAYS", "0.5");

    expect(getSubmissionFreshnessWindowDays()).toBe(1);
    expect(isSubmissionStale("2026-03-10T11:59:59.000Z")).toBe(true);
    expect(isSubmissionStale("2026-03-10T12:00:00.000Z")).toBe(false);
  });

  it("builds the submission freshness payload from latest submission metadata", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-11T12:00:00.000Z"));

    expect(
      buildSubmissionFreshness({
        updatedAt: "2026-01-15T10:30:00.000Z",
        cliVersion: "1.4.2",
        schemaVersion: 1,
      })
    ).toEqual({
      lastUpdated: "2026-01-15T10:30:00.000Z",
      cliVersion: "1.4.2",
      schemaVersion: 1,
      isStale: true,
    });
  });
});
