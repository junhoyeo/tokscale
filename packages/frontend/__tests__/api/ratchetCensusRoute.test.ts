import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NextResponse } from "next/server";

const { mockRequireAdminSession, mockGetRatchetCensusReport } = vi.hoisted(() => ({
  mockRequireAdminSession: vi.fn(),
  mockGetRatchetCensusReport: vi.fn(),
}));

vi.mock("@/lib/moderation/guard", () => ({
  requireAdminSession: mockRequireAdminSession,
}));

vi.mock("@/lib/reconciliation/ratchetCensus", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/lib/reconciliation/ratchetCensus")>();

  return {
    ...actual,
    getRatchetCensusReport: mockGetRatchetCensusReport,
  };
});

import { GET } from "@/app/api/admin/reconciliation/census/route";

describe("GET /api/admin/reconciliation/census", () => {
  beforeEach(() => {
    mockRequireAdminSession.mockReset();
    mockGetRatchetCensusReport.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("returns the concealed admin response without touching census data", async () => {
    const concealed = NextResponse.json({ error: "Not found" }, { status: 404 });
    mockRequireAdminSession.mockResolvedValue({ response: concealed });

    const response = await GET(
      new Request("https://tokscale.ai/api/admin/reconciliation/census")
    );

    expect(response.status).toBe(404);
    expect(mockGetRatchetCensusReport).not.toHaveBeenCalled();
  });

  it("returns a private non-cacheable report to an admin", async () => {
    mockRequireAdminSession.mockResolvedValue({ session: { id: "admin" } });
    mockGetRatchetCensusReport.mockResolvedValue({
      generatedAt: "2026-08-08T12:00:00.000Z",
      coverage: { totalUsers: 0 },
    });

    const response = await GET(
      new Request(
        "https://tokscale.ai/api/admin/reconciliation/census?candidateLimit=40"
      )
    );

    expect(response.status).toBe(200);
    expect(response.headers.get("Cache-Control")).toBe("private, no-store");
    expect(mockGetRatchetCensusReport).toHaveBeenCalledWith({ candidateLimit: 40 });
  });

  it.each([
    ["missing", "", 25],
    ["empty", "?candidateLimit=", 25],
    ["zero", "?candidateLimit=0", 25],
    ["negative", "?candidateLimit=-4", 25],
    ["fractional", "?candidateLimit=1.5", 25],
    ["non-numeric", "?candidateLimit=nope", 25],
    ["over the cap", "?candidateLimit=10000", 100],
  ])(
    "normalizes a %s candidate limit before querying",
    async (_label, query, expectedLimit) => {
      mockRequireAdminSession.mockResolvedValue({ session: { id: "admin" } });
      mockGetRatchetCensusReport.mockResolvedValue({
        generatedAt: "2026-08-08T12:00:00.000Z",
      });

      const response = await GET(
        new Request(
          `https://tokscale.ai/api/admin/reconciliation/census${query}`
        )
      );

      expect(response.status).toBe(200);
      expect(mockGetRatchetCensusReport).toHaveBeenCalledWith({
        candidateLimit: expectedLimit,
      });
    }
  );

  it("does not leak database errors", async () => {
    mockRequireAdminSession.mockResolvedValue({ session: { id: "admin" } });
    mockGetRatchetCensusReport.mockRejectedValue(new Error("private db detail"));
    vi.spyOn(console, "error").mockImplementation(() => {});

    const response = await GET(
      new Request("https://tokscale.ai/api/admin/reconciliation/census")
    );

    expect(response.status).toBe(500);
    expect(await response.json()).toEqual({
      error: "Failed to load reconciliation census",
    });
  });
});
