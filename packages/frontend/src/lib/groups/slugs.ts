import { eq } from "drizzle-orm";
import { db, groups } from "@/lib/db";
import { slugifyGroupName } from "./utils";

interface SlugDb {
  select: typeof db.select;
}

export async function generateUniqueGroupSlug(
  name: string,
  queryDb: SlugDb = db
): Promise<string> {
  for (let attempt = 0; attempt < 50; attempt++) {
    const slug = slugifyGroupName(name, attempt === 0 ? {} : { suffix: attempt });
    const existing = await queryDb
      .select({ id: groups.id })
      .from(groups)
      .where(eq(groups.slug, slug))
      .limit(1);

    if (existing.length === 0) {
      return slug;
    }
  }

  return slugifyGroupName(name, { suffix: Date.now().toString(36) });
}
