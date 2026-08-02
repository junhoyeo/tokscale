import { NextResponse } from "next/server";
import { db, deviceCodes, users } from "@/lib/db";
import { eq, and, gt } from "drizzle-orm";
import { issuePersonalTokenInTransaction } from "@/lib/auth/personalTokens";

export async function POST(request: Request) {
  try {
    const body = await request.json();
    const { deviceCode } = body;

    if (!deviceCode) {
      return NextResponse.json(
        { error: "Missing device code" },
        { status: 400 }
      );
    }

    const result = await db.transaction(async (tx) => {
      // Lock the device code row so concurrent polls cannot issue duplicate tokens.
      const [record] = await tx
        .select()
        .from(deviceCodes)
        .where(
          and(
            eq(deviceCodes.deviceCode, deviceCode),
            gt(deviceCodes.expiresAt, new Date())
          )
        )
        .for("update")
        .limit(1);

      if (!record) {
        return { status: "expired" as const };
      }

      // Check if user has authorized
      if (!record.userId) {
        return { status: "pending" as const };
      }

      // User has authorized - create API token
      const [user] = await tx
        .select()
        .from(users)
        .where(eq(users.id, record.userId))
        .limit(1);

      if (!user) {
        return { status: "user_not_found" as const };
      }

      const issuedToken = await issuePersonalTokenInTransaction(tx, {
        userId: user.id,
        name: record.deviceName || "CLI",
        ensureUniqueName: true,
      });

      // Delete the device code (one-time use) before committing the transaction.
      await tx.delete(deviceCodes).where(eq(deviceCodes.id, record.id));

      return {
        status: "complete" as const,
        token: issuedToken.token,
        user: {
          username: user.username,
          avatarUrl: user.avatarUrl,
        },
      };
    });

    if (result.status === "user_not_found") {
      return NextResponse.json(
        { error: "User not found" },
        { status: 500 }
      );
    }

    return NextResponse.json(result);
  } catch (error) {
    console.error("Device poll error:", error);
    return NextResponse.json(
      { error: "Failed to poll device code" },
      { status: 500 }
    );
  }
}
