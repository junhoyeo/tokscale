#!/usr/bin/env bash
# Post release notes to Discord webhook.
# Usage: DISCORD_WEBHOOK_URL=... bash scripts/post-discord-release.sh <version>
# If notes exceed 2000 chars, they're split into multiple messages at line boundaries.
set -euo pipefail

VERSION="${1:?Usage: post-discord-release.sh <version>}"

if [ -z "${DISCORD_WEBHOOK_URL:-}" ]; then
  echo "⚠️  DISCORD_WEBHOOK_URL not set, skipping Discord notification"
  exit 0
fi

# Generate release notes, strip the centered HTML header and the markdown H1 title
RAW_NOTES=$(bun scripts/generate-release-notes.ts "$VERSION")
BODY=$(echo "$RAW_NOTES" | sed '1,/<\/div>/d' | sed '/^# /d' | sed '/^$/{ N; /^\n$/d; }')

# Prepend our own Discord-friendly heading
DISCORD_BODY="## \`tokscale@v${VERSION}\` is here!
${BODY}"

MAX_LEN=2000

send_message() {
  local content="$1"
  local payload
  payload=$(jq -n --arg c "$content" '{"content":$c}')
  curl -sf -H "Content-Type: application/json" -d "$payload" "$DISCORD_WEBHOOK_URL" >/dev/null
}

if [ "${#DISCORD_BODY}" -le "$MAX_LEN" ]; then
  send_message "$DISCORD_BODY"
  echo "✅ Posted release notes to Discord (${#DISCORD_BODY} chars)"
else
  CHUNK=""
  PART=1

  while IFS= read -r line || [ -n "$line" ]; do
    if [ -z "$CHUNK" ]; then
      CANDIDATE="$line"
    else
      CANDIDATE="$CHUNK"$'\n'"$line"
    fi

    if [ "${#CANDIDATE}" -gt "$MAX_LEN" ]; then
      if [ -n "$CHUNK" ]; then
        send_message "$CHUNK"
        echo "✅ Sent part $PART (${#CHUNK} chars)"
        sleep 1
        PART=$((PART + 1))
      fi
      while [ "${#line}" -gt "$MAX_LEN" ]; do
        send_message "${line:0:$MAX_LEN}"
        echo "✅ Sent part $PART (${MAX_LEN} chars)"
        sleep 1
        PART=$((PART + 1))
        line="${line:$MAX_LEN}"
      done
      CHUNK="$line"
    else
      CHUNK="$CANDIDATE"
    fi
  done <<< "$DISCORD_BODY"

  if [ -n "$CHUNK" ]; then
    send_message "$CHUNK"
    echo "✅ Sent part $PART (${#CHUNK} chars)"
  fi

  echo "📨 Posted release notes to Discord in $PART parts"
fi
