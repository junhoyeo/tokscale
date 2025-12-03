import { NextResponse } from "next/server";
import { getSession } from "@/lib/auth/session";

export async function GET() {
  const session = await getSession();

  return NextResponse.json({
    user: session,
  });
}
