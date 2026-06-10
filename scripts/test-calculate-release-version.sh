#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="${ROOT_DIR}/scripts/calculate-release-version.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

write_cli_manifest() {
  local work="$1"
  local version="$2"
  mkdir -p "${work}/packages/cli"
  cat > "${work}/packages/cli/package.json" <<EOF_MANIFEST
{
  "name": "@tokscale/cli",
  "version": "${version}"
}
EOF_MANIFEST
}

run_calculate() {
  local work="$1"
  local output_file="$2"
  (
    cd "${work}"
    GITHUB_OUTPUT="${output_file}" bash scripts/calculate-release-version.sh
  )
}

test_rejects_invalid_override_version() {
  local work="${TMP_DIR}/invalid-override"
  mkdir -p "${work}/scripts"
  cp "${SCRIPT_UNDER_TEST}" "${work}/scripts/calculate-release-version.sh"
  write_cli_manifest "${work}" "3.0.0"

  local output="${TMP_DIR}/invalid-override-output.txt"
  local gh_output="${TMP_DIR}/invalid-override-github-output.txt"
  if RELEASE_VERSION='3.0.1$(echo injected)' RELEASE_BUMP='patch (x.x.X)' run_calculate "${work}" "${gh_output}" >"${output}" 2>&1; then
    echo "Expected invalid version override to fail" >&2
    return 1
  fi

  grep -q "Invalid release version override" "${output}"
  test ! -s "${gh_output}"
}

test_recovery_requires_override_version() {
  local work="${TMP_DIR}/recovery-without-version"
  mkdir -p "${work}/scripts"
  cp "${SCRIPT_UNDER_TEST}" "${work}/scripts/calculate-release-version.sh"
  write_cli_manifest "${work}" "3.0.0"

  local output="${TMP_DIR}/recovery-without-version-output.txt"
  local gh_output="${TMP_DIR}/recovery-without-version-github-output.txt"
  if RELEASE_RECOVERY=true RELEASE_BUMP='patch (x.x.X)' run_calculate "${work}" "${gh_output}" >"${output}" 2>&1; then
    echo "Expected recovery without override version to fail" >&2
    return 1
  fi

  grep -q "Recovery publishes must provide the existing release version" "${output}"
  test ! -s "${gh_output}"
}

test_calculates_patch_bump_and_sets_outputs() {
  local work="${TMP_DIR}/patch-bump"
  mkdir -p "${work}/scripts"
  cp "${SCRIPT_UNDER_TEST}" "${work}/scripts/calculate-release-version.sh"
  write_cli_manifest "${work}" "3.0.0-beta.1"

  local gh_output="${TMP_DIR}/patch-bump-github-output.txt"
  RELEASE_BUMP='patch (x.x.X)' run_calculate "${work}" "${gh_output}" >"${TMP_DIR}/patch-bump-output.txt" 2>&1

  grep -q '^base_version=3.0.0-beta.1$' "${gh_output}"
  grep -q '^version=3.0.1$' "${gh_output}"
}

test_rejects_unknown_bump_choice() {
  local work="${TMP_DIR}/unknown-bump"
  mkdir -p "${work}/scripts"
  cp "${SCRIPT_UNDER_TEST}" "${work}/scripts/calculate-release-version.sh"
  write_cli_manifest "${work}" "3.0.0"

  local output="${TMP_DIR}/unknown-bump-output.txt"
  local gh_output="${TMP_DIR}/unknown-bump-github-output.txt"
  if RELEASE_BUMP='patch; echo injected' run_calculate "${work}" "${gh_output}" >"${output}" 2>&1; then
    echo "Expected unknown bump choice to fail" >&2
    return 1
  fi

  grep -q "Unsupported release bump value" "${output}"
  test ! -s "${gh_output}"
}

test_publish_workflow_does_not_interpolate_inputs_inside_shell_blocks() {
  python3 - "${ROOT_DIR}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
lines = path.read_text().splitlines()
errors = []
in_run_block = False
run_indent = -1

for number, line in enumerate(lines, start=1):
    stripped = line.lstrip(" ")
    indent = len(line) - len(stripped)
    if stripped.startswith("run: |"):
        in_run_block = True
        run_indent = indent
        continue
    if in_run_block and stripped and indent <= run_indent:
        in_run_block = False
    if in_run_block and "${{ inputs." in line:
        errors.append(f"{path}:{number}: {line.strip()}")

if errors:
    raise SystemExit("Workflow inputs are interpolated inside shell run blocks:\n" + "\n".join(errors))
PY
}

test_rejects_invalid_override_version
test_recovery_requires_override_version
test_calculates_patch_bump_and_sets_outputs
test_rejects_unknown_bump_choice
test_publish_workflow_does_not_interpolate_inputs_inside_shell_blocks

echo "calculate-release-version tests passed"
