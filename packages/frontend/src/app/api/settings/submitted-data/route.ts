import { revalidatePath, revalidateTag } from "next/cache";
import { NextResponse } from "next/server";
import { eq } from "drizzle-orm";
import { getSession } from "@/lib/auth/session";
import { db, submissions } from "@/lib/db";

export async function DELETE() {
  try {
    const session = await getSession();
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const deletedRows = await db
      .delete(submissions)
      .where(eq(submissions.userId, session.id))
      .returning({ id: submissions.id });

    try {
      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${session.username}`, "max");
      revalidateTag("user-rank", "max");
      revalidateTag(`user-rank:${session.username}`, "max");
      revalidateTag(`embed-user:${session.username}`, "max");
      revalidateTag(`embed-user:${session.username}:tokens`, "max");
      revalidateTag(`embed-user:${session.username}:cost`, "max");

      revalidatePath("/leaderboard");
      revalidatePath("/profile");
      revalidatePath(`/u/${session.username}`);
      revalidatePath(`/api/users/${session.username}`);
      revalidatePath(`/api/embed/${session.username}/svg`);
    } catch (cacheError) {
      console.error("Cache invalidation failed after deletion:", cacheError);
    }

    return NextResponse.json({
      success: true,
      deleted: deletedRows.length > 0,
      deletedSubmissions: deletedRows.length,
    });
  } catch (error) {
    console.error("Submitted data delete error:", error);
    return NextResponse.json(
      { error: "Failed to delete submitted usage data" },
      { status: 500 }
    );
  }
}
