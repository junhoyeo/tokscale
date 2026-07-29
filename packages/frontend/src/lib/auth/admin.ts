import type { SessionUser } from "./session";

/**
 * Site-wide moderator identity.
 *
 * Keyed on GitHub's numeric account id rather than the username: usernames can
 * be renamed, and a released username can be claimed by someone else, so
 * authorizing against one would be a standing account-takeover risk.
 *
 * 32605822 is @junhoyeo. The env var exists so additional moderators can be
 * added without a code change; it replaces the default rather than extending
 * it, so setting it to an empty value leaves nobody with access.
 */
const DEFAULT_ADMIN_GITHUB_IDS = [32605822];

const ENV_VAR = "TOKSCALE_ADMIN_GITHUB_IDS";

/**
 * Parses the allowlist, failing closed.
 *
 * A malformed entry means the operator intended to grant access to someone and
 * we cannot tell who, so the whole list is rejected rather than silently
 * honouring the entries that happened to parse. Returning the built-in default
 * on malformed input would be worse still: a typo would quietly restore
 * single-admin access that the operator believed they had changed.
 */
export function resolveAdminGithubIds(
  rawValue: string | undefined = process.env[ENV_VAR]
): number[] {
  if (rawValue === undefined) {
    return DEFAULT_ADMIN_GITHUB_IDS;
  }

  const entries = rawValue
    .split(",")
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0);

  if (entries.length === 0) {
    return [];
  }

  const ids: number[] = [];

  for (const entry of entries) {
    // Number() would accept "0x1f", " 12 " and "1e3"; GitHub ids are plain
    // positive integers and anything else is a mistake worth refusing.
    if (!/^\d+$/.test(entry)) {
      console.error(`[admin] ${ENV_VAR} contains a non-numeric entry; denying all admin access`);
      return [];
    }

    const id = Number(entry);

    if (!Number.isSafeInteger(id) || id <= 0) {
      console.error(`[admin] ${ENV_VAR} contains an out-of-range entry; denying all admin access`);
      return [];
    }

    ids.push(id);
  }

  return ids;
}

/**
 * Whether this session may take moderation actions.
 *
 * Personal API tokens carry no githubId, so they can never satisfy this — admin
 * actions are web-session only. Pass `allowAuthorizationHeader: false` to
 * getSessionFromRequest on admin routes so token auth is rejected outright
 * rather than reaching here and failing with a confusing 404.
 */
export function isAdmin(session: SessionUser | null | undefined): boolean {
  if (!session || typeof session.githubId !== "number") {
    return false;
  }

  return resolveAdminGithubIds().includes(session.githubId);
}
