#!/usr/bin/env bash
set -euo pipefail

compose_file="${1:-docker-compose.yml}"

# The default TUI profile must never name optional client-data source paths:
# a rootful Docker daemon creates a missing bind source as root even for `:ro`.
if rg -nF '/.claude:' "$compose_file"; then
  echo "default TUI profile must not bind ~/.claude" >&2
  exit 1
fi
if rg -nF '/.config/Code/User/globalStorage:' "$compose_file"; then
  echo "default TUI profile must not bind VS Code globalStorage" >&2
  exit 1
fi
if rg -nF '/.config/Cursor/User/globalStorage:' "$compose_file"; then
  echo "default TUI profile must not bind Cursor globalStorage" >&2
  exit 1
fi
if rg -nF '/.local/share/zed:' "$compose_file"; then
  echo "default TUI profile must not bind Zed data" >&2
  exit 1
fi

rg -qF '${HOME}/.config/tokscale:/home/tokscale/.config/tokscale' "$compose_file"
rg -qF '${HOME}/.cache/tokscale:/home/tokscale/.cache/tokscale' "$compose_file"
