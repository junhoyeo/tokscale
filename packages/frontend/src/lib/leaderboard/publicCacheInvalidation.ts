import { revalidateTag } from "next/cache";
import {
  normalizeUsernameCacheKey,
  revalidateUsernamePaths,
} from "../db/usernameLookup";
import { revalidateUserGroupLeaderboards } from "../groups/cache";
import { revalidateLeaderboardPublicSurfacePaths } from "./publicSurfaceRevalidation";

export async function revalidateSubmissionPublicCaches(
  userId: string,
  username: string
): Promise<void> {
  const usernameCacheKey = normalizeUsernameCacheKey(username);

  revalidateTag("leaderboard", "max");
  revalidateTag(`user:${usernameCacheKey}`, "max");
  revalidateTag("user-rank", "max");
  revalidateTag(`user-rank:${usernameCacheKey}`, "max");
  revalidateLeaderboardPublicSurfacePaths();
  revalidateUsernamePaths(username);
  await revalidateUserGroupLeaderboards(userId);
}
