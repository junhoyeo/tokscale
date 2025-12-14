import { NextResponse } from "next/server";
import { db, deviceCodes, apiTokens, users } from "@/lib/db";
import { eq, and, gt } from "drizzle-orm";
import { generateApiToken } from "@/lib/auth/utils";

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

    // Find the device code record
    const [record] = await db
      .select()
      .from(deviceCodes)
      .where(
        and(
          eq(deviceCodes.deviceCode, deviceCode),
          gt(deviceCodes.expiresAt, new Date())
        )
      )
      .limit(1);

    if (!record) {
      return NextResponse.json({ status: "expired" });
    }

    // Check if user has authorized
    if (!record.userId) {
      return NextResponse.json({ status: "pending" });
    }

    // User has authorized - create API token
    const [user] = await db
      .select()
      .from(users)
      .where(eq(users.id, record.userId))
      .limit(1);

    if (!user) {
      return NextResponse.json(
        { error: "User not found" },
        { status: 500 }
      );
    }

    // Generate API token
    const token = generateApiToken();
    const tokenName = record.deviceName || "CLI";

    // Check if token with same name exists, append number if needed
    const existingTokens = await db
      .select()
      .from(apiTokens)
      .where(eq(apiTokens.userId, user.id));

    let finalName = tokenName;
    let counter = 1;
    while (existingTokens.some((t) => t.name === finalName)) {
      finalName = `${tokenName} (${counter})`;
      counter++;
    }

    // Store API token
    await db.insert(apiTokens).values({
      userId: user.id,
      token,
      name: finalName,
    });

    // Delete the device code (one-time use)
    await db.delete(deviceCodes).where(eq(deviceCodes.id, record.id));

    return NextResponse.json({
      status: "complete",
      token,
      user: {
        username: user.username,
        avatarUrl: user.avatarUrl,
      },
    });
  } catch (error) {
    console.error("Device poll error:", error);
    return NextResponse.json(
      { error: "Failed to poll device code" },
      { status: 500 }
    );
  }
}
