import { and, desc, eq, sql } from "drizzle-orm";
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
  expiresAt: Date | null;
}

export type PersonalTokenAuthResult =
  | { status: "invalid" }
  | { status: "expired" }
  | ({ status: "valid" } & AuthenticatedPersonalToken);

export interface AuthenticatePersonalTokenOptions {
  touchLastUsedAt?: boolean;
}

const TOKEN_NAME_LOCK_NAMESPACE = "personal_token_names";

type PersonalTokenDb = Pick<typeof db, "execute" | "insert" | "select">;

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

async function insertPersonalToken(
  client: PersonalTokenDb,
  {
    userId,
    name,
    expiresAt,
  }: {
    userId: string;
    name: string;
    expiresAt: Date | null;
  }
): Promise<IssuedPersonalToken> {
  const token = generateApiToken();
  const tokenHashed = hashToken(token);
  const [createdToken] = await client
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

export async function issuePersonalTokenInTransaction(
  tx: PersonalTokenDb,
  {
    userId,
    name,
    expiresAt = null,
    ensureUniqueName = false,
  }: IssuePersonalTokenInput
): Promise<IssuedPersonalToken> {
  if (!ensureUniqueName) {
    return insertPersonalToken(tx, { userId, name, expiresAt });
  }

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

  return insertPersonalToken(tx, { userId, name: finalName, expiresAt });
}

export async function issuePersonalToken({
  userId,
  name,
  expiresAt = null,
  ensureUniqueName = false,
}: IssuePersonalTokenInput): Promise<IssuedPersonalToken> {
  if (!ensureUniqueName) {
    return issuePersonalTokenInTransaction(db, {
      userId,
      name,
      expiresAt,
      ensureUniqueName,
    });
  }

  return db.transaction(async (tx) => {
    return issuePersonalTokenInTransaction(tx, {
      userId,
      name,
      expiresAt,
      ensureUniqueName,
    });
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

  // All rows are SHA-256 hashed at rest. Migration
  // 0006_rehash_plaintext_personal_tokens.sql rehashed any pre-#512
  // plaintext rows in place; the prior transitional OR-clause that also
  // matched the raw token value has been removed.
  const result = await db
    .select({
      tokenId: apiTokens.id,
      userId: apiTokens.userId,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      expiresAt: apiTokens.expiresAt,
    })
    .from(apiTokens)
    .innerJoin(users, eq(apiTokens.userId, users.id))
    .where(eq(apiTokens.token, tokenHashed))
    .limit(1);

  if (result.length === 0) {
    return { status: "invalid" };
  }

  const record = result[0];

  if (record.expiresAt && record.expiresAt <= new Date()) {
    return { status: "expired" };
  }

  if (options.touchLastUsedAt !== false) {
    await db
      .update(apiTokens)
      .set({ lastUsedAt: new Date() })
      .where(eq(apiTokens.id, record.tokenId));
  }

  return {
    status: "valid",
    tokenId: record.tokenId,
    userId: record.userId,
    username: record.username,
    displayName: record.displayName,
    avatarUrl: record.avatarUrl,
    expiresAt: record.expiresAt,
  };
}
