import { NextResponse } from "next/server";
import { getSession } from "@/lib/auth/session";
import {
  issuePersonalToken,
  listPersonalTokens,
  PersonalTokenNameConflictError,
} from "@/lib/auth/personalTokens";

const MAX_TOKEN_NAME_LENGTH = 100;
const ISO_DATE_TIME_PATTERN =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?(?:Z|[+-]\d{2}:\d{2})$/;

class TokenRequestValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TokenRequestValidationError";
  }
}

function parseExpiresAt(value: unknown): Date | null {
  if (value == null || value === "") {
    return null;
  }

  if (typeof value !== "string") {
    throw new TokenRequestValidationError("Expiration must be an ISO date string");
  }

  if (!ISO_DATE_TIME_PATTERN.test(value)) {
    throw new TokenRequestValidationError("Expiration must be an ISO date string");
  }

  const expiresAt = new Date(value);
  if (Number.isNaN(expiresAt.getTime())) {
    throw new TokenRequestValidationError("Expiration must be a valid date");
  }

  if (expiresAt <= new Date()) {
    throw new TokenRequestValidationError("Expiration must be in the future");
  }

  return expiresAt;
}

export async function GET() {
  try {
    const session = await getSession();
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const tokens = await listPersonalTokens(session.id);

    return NextResponse.json({
      tokens: tokens.map((token) => ({
        id: token.id,
        name: token.name,
        createdAt: token.createdAt,
        lastUsedAt: token.lastUsedAt,
        expiresAt: token.expiresAt,
      })),
    });
  } catch (error) {
    console.error("Tokens list error:", error);
    return NextResponse.json(
      { error: "Failed to fetch tokens" },
      { status: 500 }
    );
  }
}

export async function POST(request: Request) {
  try {
    const session = await getSession();
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    let body: unknown;
    try {
      body = await request.json();
    } catch {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    const input =
      body && typeof body === "object"
        ? (body as Record<string, unknown>)
        : {};
    const rawName = typeof input.name === "string" ? input.name.trim() : "";

    if (!rawName) {
      return NextResponse.json(
        { error: "Token name is required" },
        { status: 422 }
      );
    }

    if (rawName.length > MAX_TOKEN_NAME_LENGTH) {
      return NextResponse.json(
        { error: `Token name must be ${MAX_TOKEN_NAME_LENGTH} characters or fewer` },
        { status: 422 }
      );
    }

    const issuedToken = await issuePersonalToken({
      userId: session.id,
      name: rawName,
      expiresAt: parseExpiresAt(input.expiresAt),
    });

    return NextResponse.json(
      {
        token: {
          id: issuedToken.id,
          name: issuedToken.name,
          createdAt: issuedToken.createdAt,
          lastUsedAt: issuedToken.lastUsedAt,
          expiresAt: issuedToken.expiresAt,
        },
        plainTextToken: issuedToken.token,
      },
      {
        status: 201,
        headers: {
          "Cache-Control": "private, no-store",
        },
      }
    );
  } catch (error) {
    if (error instanceof PersonalTokenNameConflictError) {
      return NextResponse.json(
        { error: "A token with this name already exists" },
        { status: 409 }
      );
    }

    if (error instanceof TokenRequestValidationError) {
      return NextResponse.json({ error: error.message }, { status: 422 });
    }

    console.error("Token create error:", error);
    return NextResponse.json(
      { error: "Failed to create token" },
      { status: 500 }
    );
  }
}
