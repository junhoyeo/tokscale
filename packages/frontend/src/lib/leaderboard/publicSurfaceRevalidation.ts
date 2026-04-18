import { revalidatePath } from "next/cache";

const PUBLIC_LEADERBOARD_SURFACE_PATH = {
  HOME: "/",
  LEADERBOARD: "/leaderboard",
} as const;

export function revalidateLeaderboardPublicSurfacePaths(): void {
  revalidatePath(PUBLIC_LEADERBOARD_SURFACE_PATH.HOME);
  revalidatePath(PUBLIC_LEADERBOARD_SURFACE_PATH.LEADERBOARD);
}
