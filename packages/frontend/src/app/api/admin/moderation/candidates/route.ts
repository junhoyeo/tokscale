import { NextResponse } from "next/server";
import { getModerationCandidates } from "@/lib/moderation/candidates";
import { requireAdminSession } from "@/lib/moderation/guard";

export async function GET(request: Request) {
  try {
    const auth = await requireAdminSession(request);
    if ("response" in auth) {
      return auth.response;
    }

    const candidates = await getModerationCandidates();

    return NextResponse.json({ candidates });
  } catch (error) {
    console.error("Moderation candidates error:", error);
    return NextResponse.json(
      { error: "Failed to load moderation candidates" },
      { status: 500 }
    );
  }
}
