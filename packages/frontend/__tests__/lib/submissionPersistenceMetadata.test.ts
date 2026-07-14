import { describe, expect, it } from "vitest";
import { shouldApplyLatestSubmissionMetadata } from "../../src/lib/submissionPersistence";

describe("shouldApplyLatestSubmissionMetadata", () => {
  const firstReceipt = new Date("2026-07-14T10:00:00.000Z");
  const secondReceipt = new Date("2026-07-14T11:00:00.000Z");

  it("applies metadata when no earlier receipt watermark exists", () => {
    expect(
      shouldApplyLatestSubmissionMetadata(null, firstReceipt)
    ).toBe(true);
  });

  it("applies metadata received at or after the current watermark", () => {
    expect(
      shouldApplyLatestSubmissionMetadata(firstReceipt, secondReceipt)
    ).toBe(true);
    expect(
      shouldApplyLatestSubmissionMetadata(firstReceipt, firstReceipt)
    ).toBe(true);
  });

  it("preserves metadata received after a delayed review was queued", () => {
    expect(
      shouldApplyLatestSubmissionMetadata(secondReceipt, firstReceipt)
    ).toBe(false);
  });

  it("orders delayed approvals by receipt time rather than approval time", () => {
    const firstApprovedAt = new Date("2026-07-14T13:00:00.000Z");
    expect(firstApprovedAt.getTime()).toBeGreaterThan(secondReceipt.getTime());

    expect(
      shouldApplyLatestSubmissionMetadata(firstReceipt, secondReceipt)
    ).toBe(true);
    expect(
      shouldApplyLatestSubmissionMetadata(secondReceipt, firstReceipt)
    ).toBe(false);
  });
});
