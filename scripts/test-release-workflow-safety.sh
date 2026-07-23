#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="${ROOT_DIR}/scripts/check-release-workflow-safety.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

write_good_workflows() {
  local work="$1"
  mkdir -p \
    "${work}/.github/workflows" \
    "${work}/packages/cli-darwin-x64" \
    "${work}/packages/cli-linux-x64-gnu"
  cat > "${work}/packages/cli-darwin-x64/package.json" <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli-darwin-x64",
  "version": "3.0.0"
}
EOF_MANIFEST
  cat > "${work}/packages/cli-linux-x64-gnu/package.json" <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli-linux-x64-gnu",
  "version": "3.0.0"
}
EOF_MANIFEST
  cat > "${work}/.github/workflows/build-native.yml" <<'EOF_YAML'
name: Build Native (Test Only)

on:
  workflow_call:
    inputs:
      bumped-manifests:
        type: string
        default: ""

env:
  MACOSX_DEPLOYMENT_TARGET: "10.13"
  CARGO_TERM_COLOR: always
  CARGO_INCREMENTAL: 0

jobs:
  build:
    strategy:
      matrix:
        settings:
          - host: macos-latest
            target: x86_64-apple-darwin
            package_dir: cli-darwin-x64
            artifact_name: cli-binary-x86_64-apple-darwin
            build: cargo build --release -p tokscale-cli --target x86_64-apple-darwin
            strip: strip -x target/x86_64-apple-darwin/release/tokscale
            bin_name: tokscale
          - host: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            package_dir: cli-linux-x64-gnu
            artifact_name: cli-binary-x86_64-unknown-linux-gnu
            build: cargo zigbuild --release -p tokscale-cli --target x86_64-unknown-linux-gnu
            strip: strip target/x86_64-unknown-linux-gnu/release/tokscale
            bin_name: tokscale
    steps:
      - uses: actions/download-artifact@v6
        with:
          name: ${{ inputs.bumped-manifests }}
      - name: Smoke Android binary
        if: ${{ matrix.settings.target == 'aarch64-linux-android' }}
        run: cargo run --release -p tokscale-cli --target aarch64-linux-android -- --no-spinner --version
EOF_YAML
  cat > "${work}/.github/workflows/publish-cli.yml" <<'EOF_YAML'
name: Publish

jobs:
  build-cli-binary:
    needs: bump-versions
    uses: ./.github/workflows/build-native.yml
    with:
      bumped-manifests: bumped-manifests
  smoke-release-artifacts:
    needs: [bump-versions, build-cli-binary]
    steps:
      - uses: actions/download-artifact@v6
        with:
          pattern: cli-binary-*
          path: release-artifacts
      - run: bash scripts/test-release-package-artifacts.sh
  prepare-release-provenance:
    needs: [bump-versions, build-cli-binary, smoke-release-artifacts]
  publish-platform-packages:
    strategy:
      matrix:
        settings:
          - package_name: '@tokscale/cli-darwin-x64'
            package_dir: cli-darwin-x64
            artifact_name: cli-binary-x86_64-apple-darwin
            binary_name: tokscale
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
  python3 - "${work}/.github/workflows/build-native.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text()
text = text.replace("target: x86_64-unknown-linux-gnu", "target: unsupported-target", 1)
path.write_text(text)
PY

  local output="${TMP_DIR}/target-drift-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject target drift" >&2
    return 1
  fi

  grep -q "canonical build matrix has unknown targets" "${output}"
}

test_rejects_missing_canonical_target() {
  local work="${TMP_DIR}/missing-target"
  write_good_workflows "${work}"
  python3 - \
    "${work}/.github/workflows/build-native.yml" \
    "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import re
import sys

build_path = pathlib.Path(sys.argv[1])
build_text = build_path.read_text()
build_text, build_count = re.subn(
    r"\n          - host: macos-latest\n"
    r"            target: x86_64-apple-darwin\n"
    r"(?:            .*\n){5}",
    "\n",
    build_text,
    count=1,
)
if build_count != 1:
    raise SystemExit("failed to remove canonical Darwin build target")
build_path.write_text(build_text)

publish_path = pathlib.Path(sys.argv[2])
publish_text = publish_path.read_text()
publish_text, publish_count = re.subn(
    r"\n          - package_name: '@tokscale/cli-darwin-x64'\n"
    r"(?:            .*\n){3}",
    "\n",
    publish_text,
    count=1,
)
if publish_count != 1:
    raise SystemExit("failed to remove Darwin publish target")
publish_path.write_text(publish_text)
PY

  local output="${TMP_DIR}/missing-target-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject a missing canonical target" >&2
    return 1
  fi

  grep -q "canonical build matrix is missing targets: \\['x86_64-apple-darwin'\\]" "${output}"
}

test_rejects_publish_build_call_drift() {
  local work="${TMP_DIR}/call-drift"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace(
    "uses: ./.github/workflows/build-native.yml",
    "uses: ./.github/workflows/other.yml",
)
path.write_text(text)
PY

  local output="${TMP_DIR}/call-drift-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject reusable workflow drift" >&2
    return 1
  fi

  grep -q "publish build must call the canonical build-native workflow" "${output}"
}

test_rejects_missing_bumped_manifest_handoff() {
  local work="${TMP_DIR}/missing-bumped-manifests"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace(
    "bumped-manifests: bumped-manifests",
    "bumped-manifests: stale-manifests",
)
path.write_text(text)
PY

  local output="${TMP_DIR}/missing-bumped-manifests-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject a stale manifest handoff" >&2
    return 1
  fi

  grep -q "publish build must pass the bumped-manifests artifact" "${output}"
}

test_rejects_missing_android_smoke() {
  local work="${TMP_DIR}/missing-android-smoke"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/build-native.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace(
    """      - name: Smoke Android binary
        if: ${{ matrix.settings.target == 'aarch64-linux-android' }}
        run: cargo run --release -p tokscale-cli --target aarch64-linux-android -- --no-spinner --version
""",
    "",
)
path.write_text(text)
PY

  local output="${TMP_DIR}/missing-android-smoke-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to require the Android execution smoke" >&2
    return 1
  fi

  grep -q "build-native workflow must execute the Android binary smoke" "${output}"
}

test_rejects_missing_required_release_env() {
  local work="${TMP_DIR}/missing-env"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/build-native.yml" <<'PY'
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

test_rejects_missing_release_artifact_smoke_job() {
  local work="${TMP_DIR}/missing-smoke"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text()
text = re.sub(r"\n  smoke-release-artifacts:\n(?:    .*\n)*?  prepare-release-provenance:", "\n  prepare-release-provenance:", text)
path.write_text(text)
PY

  local output="${TMP_DIR}/missing-smoke-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject missing release artifact smoke job" >&2
    return 1
  fi

  grep -q "publish workflow missing smoke-release-artifacts job" "${output}"
}

test_rejects_commented_release_artifact_smoke_requirements() {
  local work="${TMP_DIR}/commented-smoke-requirements"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text()
text = text.replace("          pattern: cli-binary-*", "          # pattern: cli-binary-*")
text = text.replace("      - run: bash scripts/test-release-package-artifacts.sh", "      # - run: bash scripts/test-release-package-artifacts.sh")
path.write_text(text)
PY

  local output="${TMP_DIR}/commented-smoke-requirements-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject commented release artifact smoke requirements" >&2
    return 1
  fi

  grep -Fq "smoke-release-artifacts job must download cli-binary-* artifacts" "${output}"
  grep -q "smoke-release-artifacts job must run scripts/test-release-package-artifacts.sh" "${output}"
}

test_accepts_multiline_release_artifact_smoke_dependency() {
  local work="${TMP_DIR}/multiline-smoke-need"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace(
    "needs: [bump-versions, build-cli-binary, smoke-release-artifacts]",
    "needs:\n      - bump-versions\n      - build-cli-binary\n      - smoke-release-artifacts",
)
path.write_text(text)
PY

  (
    cd "${work}"
    python3 "${SCRIPT_UNDER_TEST}" >"${TMP_DIR}/multiline-smoke-need-output.txt" 2>&1
  )

  grep -q "Release workflow safety OK" "${TMP_DIR}/multiline-smoke-need-output.txt"
}

test_rejects_provenance_without_release_artifact_smoke_dependency() {
  local work="${TMP_DIR}/missing-smoke-need"
  write_good_workflows "${work}"
  python3 - "${work}/.github/workflows/publish-cli.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace(
    "needs: [bump-versions, build-cli-binary, smoke-release-artifacts]",
    "needs: [bump-versions, build-cli-binary]",
)
path.write_text(text)
PY

  local output="${TMP_DIR}/missing-smoke-need-output.txt"
  if (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1); then
    echo "Expected workflow safety check to reject provenance without release artifact smoke dependency" >&2
    return 1
  fi

  grep -q "prepare-release-provenance must depend on smoke-release-artifacts" "${output}"
}

test_accepts_matching_publish_and_native_workflows
test_reads_workflows_as_utf8_when_locale_is_non_utf8
test_rejects_build_matrix_target_drift
test_rejects_missing_canonical_target
test_rejects_publish_build_call_drift
test_rejects_missing_bumped_manifest_handoff
test_rejects_missing_android_smoke
test_rejects_missing_required_release_env
test_rejects_platform_publish_matrix_drift
test_rejects_missing_release_artifact_smoke_job
test_rejects_commented_release_artifact_smoke_requirements
test_accepts_multiline_release_artifact_smoke_dependency
test_rejects_provenance_without_release_artifact_smoke_dependency

echo "release workflow safety tests passed"
