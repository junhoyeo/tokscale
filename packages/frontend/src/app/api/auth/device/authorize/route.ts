import { NextResponse } from "next/server";
import { db, deviceCodes } from "@/lib/db";
import { eq, and, gt, isNull } from "drizzle-orm";
import { getSession } from "@/lib/auth/session";

export async function POST(request: Request) {
  try {
    // Check if user is authenticated
    const session = await getSession();
    if (!session) {
      return NextResponse.json(
        { error: "Not authenticated" },
        { status: 401 }
      );
    }

    const body = await request.json();
    const { userCode } = body;

    if (!userCode) {
      return NextResponse.json(
        { error: "Missing user code" },
        { status: 400 }
      );
    }

    // Normalize user code (uppercase, handle with/without dash)
    const normalizedCode = userCode.toUpperCase().replace(/[^A-Z0-9]/g, "");
    const formattedCode =
      normalizedCode.length === 8
        ? `${normalizedCode.slice(0, 4)}-${normalizedCode.slice(4)}`
        : userCode.toUpperCase();

    // Find the device code record
    const [record] = await db
      .select()
      .from(deviceCodes)
      .where(
        and(
          eq(deviceCodes.userCode, formattedCode),
          gt(deviceCodes.expiresAt, new Date()),
          isNull(deviceCodes.userId)
        )
      )
      .limit(1);

    if (!record) {
      return NextResponse.json(
        { error: "Invalid or expired code" },
        { status: 400 }
      );
    }

    // Link user to device code
    await db
      .update(deviceCodes)
      .set({ userId: session.id })
      .where(eq(deviceCodes.id, record.id));

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Device authorize error:", error);
    return NextResponse.json(
      { error: "Failed to authorize device" },
      { status: 500 }
    );
  }
}
