import { createHash } from "node:crypto";
import { unstable_cache } from "next/cache";
import { EXPLORE_CACHE_TAG } from "./cache";

// Next's FETCH cache ceiling is 2 MiB, including its JSON envelope. Base64
// keeps escaping/UTF-8 expansion predictable: a leaf is at most 349,528 ASCII
// characters plus the small Next envelope, not a multi-megabyte rollup.
export const CACHE_PART_BYTES = 256 * 1024;
export const EXPLORE_REVALIDATE_SECONDS = 600;
const options = { revalidate: EXPLORE_REVALIDATE_SECONDS, tags: [EXPLORE_CACHE_TAG] };
// A part never changes under its digest. Only manifests have a freshness TTL;
// otherwise a stale leaf would try to revalidate without a publishing value.
const partOptions = { revalidate: false as const, tags: [EXPLORE_CACHE_TAG] };
interface Manifest { digest: string; count: number }
class MissingPart extends Error {}

/**
 * Cache a JSON projection in bounded, immutable, content-addressed parts.
 *
 * Resolve shared cached dependencies BEFORE calling this function. The only
 * nested unstable_cache calls here publish constant leaf values; they do not
 * read other caches or run queries. Next deliberately bypasses nested lookups,
 * which is desirable when publishing a generation after privacy invalidation.
 * Leaves are computed/enqueued before their manifest. Next defers cache writes
 * until request completion, so another request may see the manifest first and
 * repair it. The digest and request-local parts prevent mixed generations even
 * in that window; this is not an atomic multi-entry cache transaction.
 *
 * A backend may evict a leaf before its manifest. A separately cached repair
 * manifest rebuilds the projection once, rather than combining fresh rows with
 * stale coverage or repeatedly serving an uncached large response.
 */
export async function partitionedCache<T>(load: () => Promise<T>, key: string[]): Promise<T> {
  const available = new Map<string, string[]>();
  const part = (manifest: Manifest, index: number, value?: string) => unstable_cache(async () => {
    if (value === undefined) throw new MissingPart();
    return value;
  }, [...key, "part", manifest.digest, String(index)], partOptions)();

  const publish = async (): Promise<Manifest> => {
    const data = Buffer.from(JSON.stringify(await load()), "utf8");
    const manifest = {
      digest: createHash("sha256").update(data).digest("hex"),
      count: Math.ceil(data.length / CACHE_PART_BYTES),
    };
    const parts: string[] = [];
    for (let offset = 0; offset < data.length; offset += CACHE_PART_BYTES) {
      parts.push(data.subarray(offset, offset + CACHE_PART_BYTES).toString("base64"));
    }
    available.set(manifest.digest, parts);
    await Promise.all(parts.map((value, index) => part(manifest, index, value)));
    return manifest;
  };
  const read = async (manifest: Manifest): Promise<T> => {
    const parts = available.get(manifest.digest)
      ?? await Promise.all(Array.from({ length: manifest.count }, (_, index) => part(manifest, index)));
    const data = Buffer.concat(parts.map((value) => Buffer.from(value, "base64")));
    if (createHash("sha256").update(data).digest("hex") !== manifest.digest) throw new MissingPart();
    return JSON.parse(data.toString("utf8")) as T;
  };

  let manifest = await unstable_cache(publish, [...key, "manifest"], options)();
  let repairPath = "";
  for (;;) {
    try {
      return await read(manifest);
    } catch (error) {
      if (!(error instanceof MissingPart)) throw error;
      // Every recovery publishes a durable pointer, including recovery of a
      // previously repaired generation. Hash the path to keep keys bounded.
      // Never return bare publish(): it strands the new parts behind an old
      // manifest, forcing another full SQL load on every subsequent request.
      repairPath = createHash("sha256").update(`${repairPath}:${manifest.digest}`).digest("hex");
      manifest = await unstable_cache(publish, [...key, "repair", repairPath], options)();
    }
  }
}
