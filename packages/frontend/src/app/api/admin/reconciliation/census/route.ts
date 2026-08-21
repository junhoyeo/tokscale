import { NextResponse } from "next/server";
import { requireAdminSession } from "@/lib/moderation/guard";
import {
  getRatchetCensusReport,
  normalizeRatchetCensusCandidateLimit,
} from "@/lib/reconciliation/ratchetCensus";

export async function GET(request: Request) {
  try {
    const auth = await requireAdminSession(request);
    if ("response" in auth) {
      return auth.response;
    }

    const url = new URL(request.url);
    const report = await getRatchetCensusReport({
      candidateLimit: normalizeRatchetCensusCandidateLimit(
        url.searchParams.get("candidateLimit")
      ),
    });

    return NextResponse.json(report, {
      headers: { "Cache-Control": "private, no-store" },
    });
  } catch (error) {
    console.error("Ratchet census report error:", error);
    return NextResponse.json(
      { error: "Failed to load reconciliation census" },
      { status: 500 }
    );
  }
}
