import { sql } from "drizzle-orm";
import { revalidatePath } from "next/cache";
import { users } from "./schema";
import { usernameLowerExpression } from "./usernameIndex";

export const USERNAME_LOOKUP_LIMIT = 2;

export class AmbiguousUsernameError extends Error {
  constructor(username: string) {
    super(`Multiple users match username ${username} case-insensitively`);
    this.name = "AmbiguousUsernameError";
  }
}

export function usernameEqualsIgnoreCase(username: string) {
  return sql`${usernameLowerExpression(users.username)} = ${normalizeUsernameCacheKey(username)}`;
}

export function normalizeUsernameCacheKey(username: string): string {
  return username.toLowerCase();
}

export function getSingleUsernameMatch<T>(
  rows: readonly T[],
  username: string,
): T | null {
  if (rows.length > 1) {
    throw new AmbiguousUsernameError(username);
  }

  return rows[0] ?? null;
}

/**
 * Revalidate every public path that may have ISR-cached a response keyed
 * by the username. Always call this with BOTH the canonical username and
 * its lowercased form, because case-insensitive lookups can populate ISR
 * entries under multiple URL casings.
 */
export function revalidateUsernamePaths(username: string): void {
  const canonical = username;
  const lower = normalizeUsernameCacheKey(username);
  const variants = canonical === lower ? [canonical] : [canonical, lower];

  for (const variant of variants) {
    revalidatePath(`/u/${variant}`);
    revalidatePath(`/api/users/${variant}`);
    revalidatePath(`/api/embed/${variant}/svg`);
  }
}
