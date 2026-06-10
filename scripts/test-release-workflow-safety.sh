#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="${ROOT_DIR}/scripts/check-release-workflow-safety.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

write_good_workflows() {
  local work="$1"
  mkdir -p "${work}/.github/workflows" "${work}/packages/cli-linux-x64-gnu"
  cat > "${work}/packages/cli-linux-x64-gnu/package.json" <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli-linux-x64-gnu",
  "version": "3.0.0"
}
EOF_MANIFEST
  cat > "${work}/.github/workflows/build-native.yml" <<'EOF_YAML'
name: Build Native (Test Only)

env:
  MACOSX_DEPLOYMENT_TARGET: "10.13"
  CARGO_TERM_COLOR: always
  CARGO_INCREMENTAL: 0

jobs:
  build:
    strategy:
      matrix:
        settings:
          - host: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            build: cargo zigbuild --release -p tokscale-cli --target x86_64-unknown-linux-gnu
            strip: strip target/x86_64-unknown-linux-gnu/release/tokscale
            bin_name: tokscale
EOF_YAML
  cat > "${work}/.github/workflows/publish-cli.yml" <<'EOF_YAML'
name: Publish

env:
  MACOSX_DEPLOYMENT_TARGET: "10.13"
  CARGO_TERM_COLOR: always
  CARGO_INCREMENTAL: 0

jobs:
  build-cli-binary:
    strategy:
      matrix:
        settings:
          - host: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            package_dir: cli-linux-x64-gnu
            artifact_name: cli-binary-x86_64-unknown-linux-gnu
            bin_name: tokscale
            build: cargo zigbuild --release -p tokscale-cli --target x86_64-unknown-linux-gnu
            strip: strip target/x86_64-unknown-linux-gnu/release/tokscale
  publish-platform-packages:
    strategy:
      matrix:
        settings:
          - package_name: '@tokscale/cli-linux-x64-gnu'
            package_dir: cli-linux-x64-gnu
            artifact_name: cli-binary-x86_64-unknown-linux-gnu
            binary_name: tokscale
EOF_YAML
}

test_accepts_matching_publish_and_native_workflows() {
  local work="${TMP_DIR}/good"
  write_good_workflows "${work}"

  (
    cd "${work}"
    python3 "${SCRIPT_UNDER_TEST}" >"${TMP_DIR}/good-output.txt" 2>&1
  )

  grep -q "Release workflow safety OK" "${TMP_DIR}/good-output.txt"
}

test_reads_workflows_as_utf8_when_locale_is_non_utf8() {
  local work="${TMP_DIR}/utf8-locale"
  write_good_workflows "${work}"
  printf '# UTF-8 sentinel: 🧪\n' >> "${work}/.github/workflows/publish-cli.yml"
  printf '# UTF-8 sentinel: 🧪\n' >> "${work}/.github/workflows/build-native.yml"

  (
    cd "${work}"
    LC_ALL=C PYTHONUTF8=0 python3 "${SCRIPT_UNDER_TEST}" >"${TMP_DIR}/utf8-locale-output.txt" 2>&1
  )

  grep -q "Release workflow safety OK" "${TMP_DIR}/utf8-locale-output.txt"
}

test_rejects_build_matrix_target_drift() {
  local work="${TMP_DIR}/target-drift"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text()
text = text.replace("target: x86_64-unknown-linux-gnu", "target: x86_64-unknown-linux-musl", 1)
path.write_text(text)
PY

  local output="${TMP_DIR}/target-drift-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject target drift" >&2
    return 1
  fi

  grep -q "build matrix targets differ" "${output}"
}

test_rejects_release_env_drift() {
  local work="${TMP_DIR}/env-drift"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace('MACOSX_DEPLOYMENT_TARGET: "10.13"', 'MACOSX_DEPLOYMENT_TARGET: "11.0"')
path.write_text(text)
PY

  local output="${TMP_DIR}/env-drift-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject env drift" >&2
    return 1
  fi

  grep -q "env MACOSX_DEPLOYMENT_TARGET differs" "${output}"
}

test_rejects_missing_required_release_env() {
  local work="${TMP_DIR}/missing-env"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" "${work}/.github/workflows/build-native.yml" <<'PY'
import pathlib
import sys

for path_arg in sys.argv[1:]:
    path = pathlib.Path(path_arg)
    text = "\n".join(
        line for line in path.read_text().splitlines() if "CARGO_INCREMENTAL:" not in line
    )
    path.write_text(text + "\n")
PY

  local output="${TMP_DIR}/missing-env-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject missing required env" >&2
    return 1
  fi

  grep -q "publish workflow missing required env CARGO_INCREMENTAL" "${output}"
  grep -q "build-native workflow missing required env CARGO_INCREMENTAL" "${output}"
}

test_rejects_platform_publish_matrix_drift() {
  local work="${TMP_DIR}/publish-drift"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace("artifact_name: cli-binary-x86_64-unknown-linux-gnu", "artifact_name: cli-binary-x86_64-unknown-linux-musl", 1)
path.write_text(text)
PY

  local output="${TMP_DIR}/publish-drift-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject platform publish drift" >&2
    return 1
  fi

  grep -q "publish platform artifact drift" "${output}"
}

test_accepts_matching_publish_and_native_workflows
test_reads_workflows_as_utf8_when_locale_is_non_utf8
test_rejects_build_matrix_target_drift
test_rejects_release_env_drift
test_rejects_missing_required_release_env
test_rejects_platform_publish_matrix_drift

echo "release workflow safety tests passed"
