#!/bin/sh
set -e

# Run pending Drizzle migrations before starting the server.
# Uses drizzle-kit migrate against the DATABASE_URL in the environment.
echo "Running database migrations..."
cd /app/packages/frontend
bun run scripts/migrate-docker.ts

cd /app
exec "$@"
