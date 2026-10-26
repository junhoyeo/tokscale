/** Shared privacy tag. Ordinary submissions can remain stale for ten minutes. */
import { revalidateTag } from "next/cache";

export const EXPLORE_CACHE_TAG = "explore";

/** Hides and deletions must not return a stale-while-revalidate response. */
export function expireExploreCache(): void {
  revalidateTag(EXPLORE_CACHE_TAG, { expire: 0 });
}
