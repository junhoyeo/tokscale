#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

PACKAGE_DIR="${1:-}"
RELEASE_RECOVERY="${RELEASE_RECOVERY:-false}"
NPM_CMD="${NPM_CMD:-npm}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

[[ -n "${PACKAGE_DIR}" ]] || fail "Usage: $0 <package-dir>"
[[ -f "${PACKAGE_DIR}/package.json" ]] || fail "Missing package manifest: ${PACKAGE_DIR}/package.json"

read_manifest_field() {
  local field="$1"
  python3 - "${PACKAGE_DIR}/package.json" "${field}" <<'PY'
import json
import sys

path, field = sys.argv[1:]
value = json.load(open(path, encoding="utf-8")).get(field)
if not isinstance(value, str) or not value:
    raise SystemExit(f"{path} missing string field {field}")
print(value)
PY
}

normalize_npm_version() {
  python3 -c 'import json, sys
raw = sys.stdin.read().strip()
if not raw:
    raise SystemExit(1)
try:
    value = json.loads(raw)
except json.JSONDecodeError:
    value = raw.strip("\"")
if isinstance(value, list):
    value = value[-1] if value else ""
if not isinstance(value, str) or not value:
    raise SystemExit(1)
print(value)
'
}

npm_view_version() {
  local spec="$1"
  local output
  local status
  local stderr_file
  stderr_file="$(mktemp)"
  set +e
  output="$("${NPM_CMD}" view "${spec}" version --json 2>"${stderr_file}")"
  status=$?
  set -e
  if [[ ${status} -ne 0 ]]; then
    if grep -Eiq 'E404|404 Not Found|not found' "${stderr_file}"; then
      rm -f "${stderr_file}"
      return 1
    fi
    echo "npm view ${spec} failed:" >&2
    cat "${stderr_file}" >&2
    rm -f "${stderr_file}"
    return 2
  fi
  rm -f "${stderr_file}"
  if ! printf '%s' "${output}" | normalize_npm_version; then
    echo "npm view ${spec} returned an invalid version payload" >&2
    return 2
  fi
}

npm_view_version_status() {
  local __result_var="$1"
  local spec="$2"
  local output
  local status
  set +e
  output="$(npm_view_version "${spec}")"
  status=$?
  set -e
  if [[ ${status} -eq 0 ]]; then
    printf -v "${__result_var}" '%s' "${output}"
  fi
  return "${status}"
}

package_name="$(read_manifest_field name)"
package_version="$(read_manifest_field version)"
package_spec="${package_name}@${package_version}"

semver_prerelease() {
  python3 - "$1" <<'PY'
import re
import sys

version = sys.argv[1]
match = re.fullmatch(r"\d+\.\d+\.\d+(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?", version)
if not match:
    raise SystemExit(f"Unsupported semver value: {version}")
print(match.group(1) or "")
PY
}

derive_dist_tag() {
  python3 - "$1" <<'PY'
import re
import sys

prerelease = sys.argv[1]
if not prerelease:
    print("latest")
    raise SystemExit(0)

identifier = prerelease.split(".", 1)[0].lower()
if not re.fullmatch(r"[a-z][a-z0-9_-]*", identifier):
    identifier = "next"
print(identifier)
PY
}

package_prerelease="$(semver_prerelease "${package_version}")"
npm_dist_tag="${NPM_DIST_TAG:-$(derive_dist_tag "${package_prerelease}")}"
if [[ -n "${package_prerelease}" && "${npm_dist_tag}" == "latest" ]]; then
  fail "Refusing to publish prerelease ${package_spec} with npm dist-tag latest"
fi

package_exists=false
if npm_view_version_status published_version "${package_spec}"; then
  package_exists=true
else
  status=$?
  if [[ ${status} -ne 1 ]]; then
    fail "Unable to verify ${package_spec} on npm"
  fi
fi

if [[ "${package_exists}" == "true" ]]; then
  if [[ "${RELEASE_RECOVERY}" == "true" ]]; then
    echo "Skipping ${package_name}@${published_version} because it already exists on npm"
    exit 0
  fi
  fail "${package_spec} already exists on npm; set RELEASE_RECOVERY=true to skip already-published packages"
fi

echo "Publishing ${package_spec} with npm dist-tag ${npm_dist_tag}"
(
  cd "${PACKAGE_DIR}"
  "${NPM_CMD}" publish --access public --tag "${npm_dist_tag}"
)
