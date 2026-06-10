#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

NEW_VERSION="${NEW_VERSION:-}"
RELEASE_BASE_VERSION="${RELEASE_BASE_VERSION:-}"
RELEASE_RECOVERY="${RELEASE_RECOVERY:-false}"
NPM_CMD="${NPM_CMD:-npm}"
NPM_CHECK_AUTH="${NPM_CHECK_AUTH:-1}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

[[ -n "${NEW_VERSION}" ]] || fail "NEW_VERSION is required"

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

npm_view_required_version() {
  local __result_var="$1"
  local spec="$2"
  local package_label="$3"
  local status
  npm_view_version_status "${__result_var}" "${spec}"
  status=$?
  if [[ ${status} -eq 0 ]]; then
    return 0
  fi
  if [[ ${status} -eq 1 ]]; then
    errors+=("${package_label}: package is not visible on npm")
  else
    errors+=("${package_label}: npm lookup failed")
  fi
  return "${status}"
}

npm_view_optional_version() {
  local __result_var="$1"
  local spec="$2"
  local package_label="$3"
  local status
  npm_view_version_status "${__result_var}" "${spec}"
  status=$?
  if [[ ${status} -eq 0 ]]; then
    return 0
  fi
  if [[ ${status} -eq 1 ]]; then
    return 1
  fi
  errors+=("${package_label}: npm lookup failed")
  return "${status}"
}

semver_gt() {
  python3 - "$1" "$2" <<'PY'
import re
import sys

def parts(version: str) -> tuple[int, int, int, str]:
    match = re.match(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$", version)
    if not match:
        raise SystemExit(f"Unsupported semver value: {version}")
    major, minor, patch, prerelease = match.groups()
    return int(major), int(minor), int(patch), prerelease or ""

left = parts(sys.argv[1])
right = parts(sys.argv[2])
if left[:3] != right[:3]:
    raise SystemExit(0 if left[:3] > right[:3] else 1)
if left[3] == right[3]:
    raise SystemExit(1)
if not left[3] and right[3]:
    raise SystemExit(0)
if left[3] and not right[3]:
    raise SystemExit(1)
raise SystemExit(0 if left[3] > right[3] else 1)
PY
}

release_packages() {
  python3 - <<'PY'
import json
import pathlib

root = pathlib.Path(".")
paths = [root / "packages/cli/package.json"]
cli = json.loads(paths[0].read_text())
for package_name in cli.get("optionalDependencies", {}):
    if not package_name.startswith("@tokscale/cli-"):
        raise SystemExit(f"Unexpected optional dependency package name: {package_name}")
    paths.append(root / "packages" / package_name.removeprefix("@tokscale/") / "package.json")
paths.append(root / "packages/tokscale/package.json")

seen = set()
for path in paths:
    manifest = json.loads(path.read_text())
    name = manifest.get("name")
    version = manifest.get("version")
    if not name or not version:
        raise SystemExit(f"{path} must contain name and version")
    if name in seen:
        continue
    seen.add(name)
    print(f"{path}\t{name}\t{version}")
PY
}

if [[ "${NPM_CHECK_AUTH}" != "0" ]]; then
  "${NPM_CMD}" whoami >/dev/null
fi

primary_packages=("@tokscale/cli" "tokscale")
errors=()
checked=0
existing_targets=0

while IFS=$'\t' read -r path package_name manifest_version; do
  [[ -n "${package_name}" ]] || continue
  checked=$((checked + 1))

  if [[ "${manifest_version}" != "${NEW_VERSION}" ]]; then
    errors+=("${path}: expected version ${NEW_VERSION}, found ${manifest_version}")
  fi

  if ! npm_view_required_version current_version "${package_name}" "${package_name}"; then
    continue
  fi
  echo "${package_name}: npm latest ${current_version}"

  if npm_view_optional_version target_version "${package_name}@${NEW_VERSION}" "${package_name}@${NEW_VERSION}"; then
    existing_targets=$((existing_targets + 1))
    if [[ "${RELEASE_RECOVERY}" == "true" ]]; then
      echo "${package_name}@${target_version} already exists; recovery publish will skip it"
    else
      errors+=("${package_name}@${NEW_VERSION} already exists on npm; choose a new version or set RELEASE_RECOVERY=true for a retry")
    fi
  fi

  for primary in "${primary_packages[@]}"; do
    if [[ "${package_name}" == "${primary}" && -n "${RELEASE_BASE_VERSION}" && "${RELEASE_RECOVERY}" != "true" ]]; then
      if semver_gt "${RELEASE_BASE_VERSION}" "${current_version}"; then
        errors+=("Repository version ${RELEASE_BASE_VERSION} is ahead of npm latest ${current_version} for ${package_name}; set RELEASE_RECOVERY=true and target ${RELEASE_BASE_VERSION} when retrying the failed publish")
      fi
    fi
  done
done < <(release_packages)

if [[ ${checked} -eq 0 ]]; then
  errors+=("No release packages found")
fi

if [[ "${RELEASE_RECOVERY}" == "true" ]]; then
  if [[ -z "${RELEASE_BASE_VERSION}" ]]; then
    errors+=("Recovery target ${NEW_VERSION} requires RELEASE_BASE_VERSION")
  elif [[ "${NEW_VERSION}" != "${RELEASE_BASE_VERSION}" ]]; then
    errors+=("Recovery target ${NEW_VERSION} must match release base version ${RELEASE_BASE_VERSION}")
  fi
fi

if [[ ${#errors[@]} -gt 0 ]]; then
  printf 'npm release-state check failed:\n' >&2
  printf -- '- %s\n' "${errors[@]}" >&2
  exit 1
fi

if [[ "${RELEASE_RECOVERY}" == "true" ]]; then
  echo "Recovery target ${NEW_VERSION}: ${existing_targets} package version(s) already exist on npm"
fi
echo "npm release-state OK for ${NEW_VERSION}"
