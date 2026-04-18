import { and, desc, eq, or, sql } from "drizzle-orm";
import { db, apiTokens, users } from "@/lib/db";
import { generateApiToken, hashToken } from "@/lib/auth/utils";

export interface PersonalTokenListItem {
  id: string;
  userId: string;
  name: string;
  createdAt: Date;
  lastUsedAt: Date | null;
  expiresAt: Date | null;
}

export interface IssuePersonalTokenInput {
  userId: string;
  name: string;
  expiresAt?: Date | null;
  ensureUniqueName?: boolean;
}

export interface IssuedPersonalToken extends PersonalTokenListItem {
  token: string;
}

export interface AuthenticatedPersonalToken {
  tokenId: string;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  isAdmin: boolean;
  expiresAt: Date | null;
  needsLegacyTokenHashUpgrade: boolean;
}

export type PersonalTokenAuthResult =
  | { status: "invalid" }
  | { status: "expired" }
  | ({ status: "valid" } & AuthenticatedPersonalToken);

export interface AuthenticatePersonalTokenOptions {
  touchLastUsedAt?: boolean;
  upgradeLegacyTokenHash?: boolean;
}

const TOKEN_NAME_LOCK_NAMESPACE = "personal_token_names";

function getUniqueTokenName(baseName: string, existingNames: Iterable<string>): string {
  const names = new Set(existingNames);
  let finalName = baseName;
  let counter = 1;

  while (names.has(finalName)) {
    finalName = `${baseName} (${counter})`;
    counter++;
  }

  return finalName;
}

export async function issuePersonalToken({
  userId,
  name,
  expiresAt = null,
  ensureUniqueName = false,
}: IssuePersonalTokenInput): Promise<IssuedPersonalToken> {
  if (!ensureUniqueName) {
    const token = generateApiToken();
    const tokenHashed = hashToken(token);
    const [createdToken] = await db
      .insert(apiTokens)
      .values({
        userId,
        token: tokenHashed,
        name,
        expiresAt,
      })
      .returning({
        id: apiTokens.id,
        userId: apiTokens.userId,
        name: apiTokens.name,
        createdAt: apiTokens.createdAt,
        lastUsedAt: apiTokens.lastUsedAt,
        expiresAt: apiTokens.expiresAt,
      });

    return {
      ...createdToken,
      token,
    };
  }

  return db.transaction(async (tx) => {
    await tx.execute(sql`
      SELECT pg_advisory_xact_lock(
        hashtext(${TOKEN_NAME_LOCK_NAMESPACE}),
        hashtext(${userId})
      )
    `);

    const existingTokens = await tx
      .select({
        name: apiTokens.name,
      })
      .from(apiTokens)
      .where(eq(apiTokens.userId, userId))
      .orderBy(desc(apiTokens.createdAt));

    const finalName = getUniqueTokenName(
      name,
      existingTokens.map((token) => token.name)
    );
    const token = generateApiToken();
    const tokenHashed = hashToken(token);
    const [createdToken] = await tx
      .insert(apiTokens)
      .values({
        userId,
        token: tokenHashed,
        name: finalName,
        expiresAt,
      })
      .returning({
        id: apiTokens.id,
        userId: apiTokens.userId,
        name: apiTokens.name,
        createdAt: apiTokens.createdAt,
        lastUsedAt: apiTokens.lastUsedAt,
        expiresAt: apiTokens.expiresAt,
      });

    return {
      ...createdToken,
      token,
    };
  });
}

export async function listPersonalTokens(userId: string): Promise<PersonalTokenListItem[]> {
  return db
    .select({
      id: apiTokens.id,
      userId: apiTokens.userId,
      name: apiTokens.name,
      createdAt: apiTokens.createdAt,
      lastUsedAt: apiTokens.lastUsedAt,
      expiresAt: apiTokens.expiresAt,
    })
    .from(apiTokens)
    .where(eq(apiTokens.userId, userId))
    .orderBy(desc(apiTokens.createdAt));
}

export async function revokePersonalToken(
  userId: string,
  tokenId: string
): Promise<boolean> {
  const result = await db
    .delete(apiTokens)
    .where(and(eq(apiTokens.id, tokenId), eq(apiTokens.userId, userId)))
    .returning({ id: apiTokens.id });

  return result.length > 0;
}

export async function authenticatePersonalToken(
  token: string,
  options: AuthenticatePersonalTokenOptions = {}
): Promise<PersonalTokenAuthResult> {
  if (!token.startsWith("tt_")) {
    return { status: "invalid" };
  }

  const tokenHashed = hashToken(token);

  const result = await db
    .select({
      tokenId: apiTokens.id,
      tokenValue: apiTokens.token,
      userId: apiTokens.userId,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      isAdmin: users.isAdmin,
      expiresAt: apiTokens.expiresAt,
    })
    .from(apiTokens)
    .innerJoin(users, eq(apiTokens.userId, users.id))
    .where(or(eq(apiTokens.token, tokenHashed), eq(apiTokens.token, token)))
    .limit(1);

  if (result.length === 0) {
    return { status: "invalid" };
  }

  const record = result[0];
  const isLegacyPlaintext = record.tokenValue === token;

  if (record.expiresAt && record.expiresAt <= new Date()) {
    return { status: "expired" };
  }

  const updates: Record<string, unknown> = {};
  if (options.touchLastUsedAt !== false) {
    updates.lastUsedAt = new Date();
  }
  const shouldUpgradeLegacyTokenHash =
    isLegacyPlaintext && options.upgradeLegacyTokenHash !== false;
  if (shouldUpgradeLegacyTokenHash) {
    updates.token = tokenHashed;
  }
  if (Object.keys(updates).length > 0) {
    await db
      .update(apiTokens)
      .set(updates)
      .where(eq(apiTokens.id, record.tokenId));
  }

  return {
    status: "valid",
    tokenId: record.tokenId,
    userId: record.userId,
    username: record.username,
    displayName: record.displayName,
    avatarUrl: record.avatarUrl,
    isAdmin: record.isAdmin,
    expiresAt: record.expiresAt,
    needsLegacyTokenHashUpgrade:
      isLegacyPlaintext && !shouldUpgradeLegacyTokenHash,
  };
}
