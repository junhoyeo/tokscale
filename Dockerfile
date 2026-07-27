# syntax=docker/dockerfile:1

# ── 1. base ────────────────────────────────────────────────────────────────────
FROM oven/bun:1-alpine AS base
WORKDIR /repo

# ── 2. deps — install all workspace dependencies ───────────────────────────────
FROM base AS deps
COPY package.json bun.lock ./
# All workspace package.json files must be present for bun to resolve the
# workspace before checking the lockfile — even for packages not built here.
COPY packages/benchmarks/package.json         packages/benchmarks/package.json
COPY packages/cli/package.json                packages/cli/package.json
COPY packages/cli-android-arm64/package.json  packages/cli-android-arm64/package.json
COPY packages/cli-darwin-arm64/package.json   packages/cli-darwin-arm64/package.json
COPY packages/cli-darwin-x64/package.json     packages/cli-darwin-x64/package.json
COPY packages/cli-linux-arm64-gnu/package.json packages/cli-linux-arm64-gnu/package.json
COPY packages/cli-linux-arm64-musl/package.json packages/cli-linux-arm64-musl/package.json
COPY packages/cli-linux-x64-gnu/package.json  packages/cli-linux-x64-gnu/package.json
COPY packages/cli-linux-x64-musl/package.json packages/cli-linux-x64-musl/package.json
COPY packages/cli-win32-arm64-msvc/package.json packages/cli-win32-arm64-msvc/package.json
COPY packages/cli-win32-x64-msvc/package.json packages/cli-win32-x64-msvc/package.json
COPY packages/frontend/package.json           packages/frontend/package.json
COPY packages/tokscale/package.json           packages/tokscale/package.json
RUN bun install --frozen-lockfile

# ── 3. builder — compile the Next.js app ──────────────────────────────────────
FROM base AS builder
WORKDIR /repo

COPY --from=deps /repo/node_modules ./node_modules
COPY --from=deps /repo/packages/frontend/node_modules ./packages/frontend/node_modules
COPY packages/frontend ./packages/frontend
COPY package.json bun.lock ./

WORKDIR /repo/packages/frontend
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    NODE_OPTIONS="--max-old-space-size=1536"

# DATABASE_URL is required at build time because Next.js statically prerenders
# pages that query the DB. Pass it via --build-arg or docker-compose build.args.
ARG DATABASE_URL
ENV DATABASE_URL=${DATABASE_URL}

RUN bun run build

# ── 4. runner — minimal production image ──────────────────────────────────────
FROM oven/bun:1-alpine AS runner
WORKDIR /app

ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0

# next standalone bundles server.js + required node_modules under /app
COPY --from=builder /repo/packages/frontend/.next/standalone ./
COPY --from=builder /repo/packages/frontend/.next/static      ./packages/frontend/.next/static
COPY --from=builder /repo/packages/frontend/public            ./packages/frontend/public

# drizzle-kit + migration files for the entrypoint
COPY --from=builder /repo/packages/frontend/drizzle.config.ts  ./packages/frontend/drizzle.config.ts
COPY --from=builder /repo/packages/frontend/src/lib/db         ./packages/frontend/src/lib/db
COPY --from=builder /repo/packages/frontend/scripts/migrate-docker.ts ./packages/frontend/scripts/migrate-docker.ts
COPY --from=builder /repo/packages/frontend/scripts/migrate-retry.ts  ./packages/frontend/scripts/migrate-retry.ts
COPY --from=deps    /repo/packages/frontend/node_modules      ./packages/frontend/node_modules
COPY --from=deps    /repo/node_modules                        ./node_modules
COPY package.json ./

COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 3000

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["node", "packages/frontend/server.js"]
