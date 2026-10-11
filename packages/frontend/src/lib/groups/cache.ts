import { revalidatePath, revalidateTag } from "next/cache";
import { eq } from "drizzle-orm";
import { db, groupMembers } from "@/lib/db";

export async function revalidateGroupCaches(groupId: string, slug?: string): Promise<void> {
  revalidateTag(`group:${groupId}`, { expire: 0 });
  revalidateTag(`group-leaderboard:${groupId}`, { expire: 0 });
  revalidatePath("/groups");

  if (slug) {
    revalidatePath(`/groups/${slug}`);
  }
}

export async function revalidateUserGroupLeaderboards(userId: string): Promise<void> {
  const memberships = await db
    .select({ groupId: groupMembers.groupId })
    .from(groupMembers)
    .where(eq(groupMembers.userId, userId));

  for (const membership of memberships) {
    revalidateTag(`group:${membership.groupId}`, "max");
    revalidateTag(`group-leaderboard:${membership.groupId}`, "max");
  }
}
