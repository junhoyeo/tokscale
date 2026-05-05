import { NextResponse } from "next/server";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import { getBearerToken } from "../../../../lib/auth/bearerToken";

export async function GET(request: Request) {
  try {
    const token = getBearerToken(request.headers.get("Authorization"));
    if (!token) {
      return NextResponse.json(
        { error: "Missing or invalid Authorization header" },
        { status: 401 }
      );
    }

    const authResult = await authenticatePersonalToken(token, {
      touchLastUsedAt: false,
    });

    if (authResult.status === "invalid") {
      return NextResponse.json({ error: "Invalid API token" }, { status: 401 });
    }

    if (authResult.status === "expired") {
      return NextResponse.json(
        { error: "API token has expired" },
        { status: 401 }
      );
    }

    return NextResponse.json({
      user: {
        username: authResult.username,
        displayName: authResult.displayName,
        avatarUrl: authResult.avatarUrl,
      },
    });
  } catch (error) {
    console.error("Token auth error:", error);
    return NextResponse.json(
      { error: "Failed to validate token" },
      { status: 500 }
    );
  }
}
