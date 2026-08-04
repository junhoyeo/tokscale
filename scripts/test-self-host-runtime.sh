#!/usr/bin/env bash
set -euo pipefail

base_url="${1:-http://localhost:3333}"
origin="${2:?pass the expected APP_URL origin}"

for path in / /privacy /robots.txt /sitemap.xml; do
  body="$(curl --fail --silent --show-error "${base_url}${path}")"
  if [[ "$body" != *"$origin"* ]]; then
    echo "${path} did not emit runtime origin ${origin}" >&2
    exit 1
  fi
  if [[ "$body" == *"https://tokscale.ai"* ]]; then
    echo "${path} retained hosted origin" >&2
    exit 1
  fi
done

redirect="$(curl --silent --show-error --dump-header - --output /dev/null "${base_url}/api/auth/github/callback" | tr -d '\r')"
if [[ "$redirect" != *"location: ${origin}/?error=missing_params"* ]]; then
  echo "OAuth callback did not redirect to runtime origin" >&2
  exit 1
fi
