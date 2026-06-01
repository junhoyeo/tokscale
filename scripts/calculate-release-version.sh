#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

RELEASE_BUMP="${RELEASE_BUMP:-patch (x.x.X)}"
RELEASE_VERSION="${RELEASE_VERSION:-}"
RELEASE_RECOVERY="${RELEASE_RECOVERY:-false}"
RELEASE_VERSION_FILE="${RELEASE_VERSION_FILE:-}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

validate_semver() {
  local label="$1"
  local value="$2"
  python3 - "${label}" "${value}" <<'PY'
import re
import sys

label, value = sys.argv[1:]
identifier = r"(?:0|[1-9A-Za-z-][0-9A-Za-z-]*)"
pattern = re.compile(
    r"^(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    rf"(?:-({identifier}(?:\.{identifier})*))?"
    rf"(?:\+({identifier}(?:\.{identifier})*))?$"
)
if not pattern.match(value):
    raise SystemExit(f"{label}: {value}")
PY
}

write_output() {
  local key="$1"
  local value="$2"
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    echo "${key}=${value}" >> "${GITHUB_OUTPUT}"
  fi
}

CURRENT_VERSION="$(jq -r '.version' packages/cli/package.json)"
[[ -n "${CURRENT_VERSION}" && "${CURRENT_VERSION}" != "null" ]] ||
  fail "Could not read packages/cli/package.json version"
validate_semver "Invalid current version" "${CURRENT_VERSION}" ||
  fail "Invalid current version: ${CURRENT_VERSION}"

if [[ "${RELEASE_RECOVERY}" == "true" && -z "${RELEASE_VERSION}" ]]; then
  fail "Recovery publishes must provide the existing release version."
fi

if [[ -n "${RELEASE_VERSION}" ]]; then
  validate_semver "Invalid release version override" "${RELEASE_VERSION}" ||
    fail "Invalid release version override: ${RELEASE_VERSION}"
  NEW_VERSION="${RELEASE_VERSION}"
else
  BASE="${CURRENT_VERSION%%-*}"
  BASE="${BASE%%+*}"
  IFS='.' read -r MAJOR MINOR PATCH <<< "${BASE}"

  case "${RELEASE_BUMP}" in
    "major (X.0.0)")
      NEW_VERSION="$((MAJOR + 1)).0.0"
      ;;
    "minor (x.X.0)")
      NEW_VERSION="${MAJOR}.$((MINOR + 1)).0"
      ;;
    "patch (x.x.X)")
      NEW_VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))"
      ;;
    *)
      fail "Unsupported release bump value: ${RELEASE_BUMP}"
      ;;
  esac
fi

write_output "base_version" "${CURRENT_VERSION}"
write_output "version" "${NEW_VERSION}"
if [[ -n "${RELEASE_VERSION_FILE}" ]]; then
  printf '%s\n' "${NEW_VERSION}" > "${RELEASE_VERSION_FILE}"
fi
echo "release version: ${NEW_VERSION}"
