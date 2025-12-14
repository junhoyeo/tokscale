import { NextResponse } from "next/server";
import { db, deviceCodes } from "@/lib/db";
import { generateDeviceCode, generateUserCode } from "@/lib/auth/utils";

const DEVICE_CODE_EXPIRY_SECONDS = 900; // 15 minutes
const POLL_INTERVAL_SECONDS = 5;

export async function POST(request: Request) {
  try {
    const body = await request.json().catch(() => ({}));
    const deviceName = body.deviceName || "Unknown Device";

    // Generate codes
    const deviceCode = generateDeviceCode();
    const userCode = generateUserCode();
    const expiresAt = new Date(Date.now() + DEVICE_CODE_EXPIRY_SECONDS * 1000);

    // Store in database
    await db.insert(deviceCodes).values({
      deviceCode,
      userCode,
      deviceName,
      expiresAt,
    });

    const baseUrl = process.env.NEXT_PUBLIC_URL || "http://localhost:3000";

    return NextResponse.json({
      deviceCode,
      userCode,
      verificationUrl: `${baseUrl}/device`,
      expiresIn: DEVICE_CODE_EXPIRY_SECONDS,
      interval: POLL_INTERVAL_SECONDS,
    });
  } catch (error) {
    console.error("Device code generation error:", error);
    return NextResponse.json(
      { error: "Failed to generate device code" },
      { status: 500 }
    );
  }
}
