import { NextResponse } from "next/server";
import { z } from "zod";
import { MODERATION_ACTION_TYPES } from "@/lib/db/schema";
import {
  applyModerationAction,
  findModerationTarget,
  getModerationHistory,
} from "@/lib/moderation/actions";
import { requireAdminSession } from "@/lib/moderation/guard";

interface RouteParams {
  params: Promise<{ username: string }>;
}

const ActionSchema = z.object({
  action: z.enum(MODERATION_ACTION_TYPES),
  // Required, not optional: an unexplained hide is indefensible six months
  // later, and this text is the only record of why the call was made.
  reason: z.string().trim().min(1, "A reason is required").max(500),
});

export async function GET(request: Request, { params }: RouteParams) {
  try {
    const auth = await requireAdminSession(request);
    if ("response" in auth) {
      return auth.response;
    }

    const { username } = await params;
    const target = await findModerationTarget(username);

    if (!target) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    return NextResponse.json({
      user: {
        username: target.username,
        leaderboardHidden: target.leaderboardHidden,
      },
      history: await getModerationHistory(target.id),
    });
  } catch (error) {
    console.error("Moderation history error:", error);
    return NextResponse.json(
      { error: "Failed to load moderation history" },
      { status: 500 }
    );
  }
}

export async function POST(request: Request, { params }: RouteParams) {
  try {
    const auth = await requireAdminSession(request);
    if ("response" in auth) {
      return auth.response;
    }

    const { username } = await params;

    const body = await request.json().catch(() => null);
    const parsed = ActionSchema.safeParse(body);

    if (!parsed.success) {
      return NextResponse.json(
        { error: parsed.error.issues[0]?.message ?? "Invalid request" },
        { status: 400 }
      );
    }

    const target = await findModerationTarget(username);

    if (!target) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    const result = await applyModerationAction({
      target,
      actorUserId: auth.session.id,
      actorUsername: auth.session.username,
      action: parsed.data.action,
      reason: parsed.data.reason,
    });

    return NextResponse.json({
      username: target.username,
      leaderboardHidden: result.leaderboardHidden,
      // Distinguishes a real state change from a no-op re-submit, so the UI
      // can avoid claiming it did something it did not.
      changed: result.changed,
    });
  } catch (error) {
    console.error("Moderation action error:", error);
    return NextResponse.json(
      { error: "Failed to apply moderation action" },
      { status: 500 }
    );
  }
}
