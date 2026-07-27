#!/bin/sh
set -e

# Run pending Drizzle migrations before starting the server.
# Uses drizzle-kit migrate against the DATABASE_URL in the environment.
echo "Running database migrations..."
cd /app/packages/frontend
bunx drizzle-kit migrate

cd /app
exec "$@"
