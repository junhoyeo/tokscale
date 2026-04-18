import { revalidatePath } from "next/cache";

const PUBLIC_LEADERBOARD_SURFACE_PATH = {
  HOME: "/",
  API_LEADERBOARD: "/api/leaderboard",
  LEADERBOARD: "/leaderboard",
} as const;

export function revalidateLeaderboardPublicSurfacePaths(): void {
  revalidatePath(PUBLIC_LEADERBOARD_SURFACE_PATH.HOME);
  revalidatePath(PUBLIC_LEADERBOARD_SURFACE_PATH.API_LEADERBOARD);
  revalidatePath(PUBLIC_LEADERBOARD_SURFACE_PATH.LEADERBOARD);
}
