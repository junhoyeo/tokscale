# Frontend Guidelines

Constraints of `packages/frontend` that are easy to break without noticing. Each one caused slow pages on tokscale.ai before it was fixed. Read the named file before relying on a line here.

## Where the code runs

- Vercel functions run in the region set by `regions` in `vercel.json`. It must be the region of the production Postgres. A function elsewhere pays a cross-region round trip for every query, which turns a page that issues a few dozen queries into one that takes seconds. If the database moves, change both together.
- `src/lib/db/index.ts` sizes the connection pool per function instance. One instance serves many requests at once, so a pool of one makes every query wait behind the slowest one in flight. The total is instances times `max`, and the server's `max_connections` is the ceiling.

## Prerendered and per-request pages

- `isPrerenderedDeployment()` in `src/lib/prerender.ts` is true on Vercel. There the root layout does not call `connection()`, so a page that reads no cookies, headers or search params is built at deploy time and served from the CDN. A self-hosted build prerenders nothing, because `APP_URL` is only known when the container starts.
- A prerendered page is never refreshed unless it exports `revalidate`. Any such page that reads the database must export it.
- After adding a page or a data source, read the route table of a Vercel-like build: `VERCEL=1 DATABASE_URL=<local> bunx next build`. Rows marked `○` or `●` are prerendered and need the right `Revalidate` value; rows marked `ƒ` render per request. A page that shows anything about the signed-in user must be `ƒ` or load that part in the browser.
- A prerendered page is published as a whole and kept until the next rebuild. Let a failed load throw there, as `section()` in `src/app/(main)/page.tsx` does, so the last complete page stays up. Catching the error publishes an empty section instead.
- Hiding or deleting public usage must reach prerendered pages at once. A page carries the cache tags of the data it reads, so `expireExploreCache()` already covers pages built from the explore rollups. A page that reads usage through an untagged path would stay stale: tag it or call `revalidatePath`.

## Links, metadata and images

- `generateMetadata` runs for link prefetches as well as page views. Keep it to cached reads such as `getModelSummary`; never run a usage scan there.
- Links to `/u/<username>` and `/models/<key>` pass `prefetch={false}`. Those pages render per request and have no loading state, so a prefetch fetches no content and only invokes the function, once per visible row of every list.
- Build avatar URLs with `avatarSrc` or `sizedAvatarUrl` from `src/lib/avatar.ts`. A stored GitHub avatar URL has no size, and GitHub then returns the full image for a slot of a few dozen pixels.
