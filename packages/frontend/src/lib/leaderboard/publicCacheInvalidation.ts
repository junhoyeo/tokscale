import { revalidateTag } from "next/cache";
import {
  normalizeUsernameCacheKey,
  revalidateUsernamePaths,
} from "../db/usernameLookup";
import { revalidateUserGroupLeaderboards } from "../groups/cache";
import { revalidateLeaderboardPublicSurfacePaths } from "./publicSurfaceRevalidation";

export async function revalidateSubmissionPublicCaches(
  userId: string,
  username: string,
  affectedUsernames: readonly string[] = []
): Promise<void> {
  const affectedUsernamesByCacheKey = new Map<string, string>();
  for (const affectedUsername of [username, ...affectedUsernames]) {
    const cacheKey = normalizeUsernameCacheKey(affectedUsername);
    if (!affectedUsernamesByCacheKey.has(cacheKey)) {
      affectedUsernamesByCacheKey.set(cacheKey, affectedUsername);
    }
  }

  revalidateTag("leaderboard", "max");
  revalidateTag("user-rank", "max");

  for (const [cacheKey, affectedUsername] of affectedUsernamesByCacheKey) {
    revalidateTag(`user:${cacheKey}`, "max");
    revalidateTag(`user-rank:${cacheKey}`, "max");
    revalidateTag(`embed-user:${cacheKey}`, "max");
    revalidateTag(`embed-user:${cacheKey}:tokens`, "max");
    revalidateTag(`embed-user:${cacheKey}:cost`, "max");
    revalidateUsernamePaths(affectedUsername);
  }

  revalidateLeaderboardPublicSurfacePaths();
  await revalidateUserGroupLeaderboards(userId);
}
