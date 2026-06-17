#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK_SCRIPT="${ROOT_DIR}/scripts/check-npm-release-state.sh"
PUBLISH_SCRIPT="${ROOT_DIR}/scripts/publish-npm-package.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

write_release_package_manifests() {
  local version="$1"
  mkdir -p \
    packages/cli \
    packages/cli-darwin-arm64 \
    packages/cli-darwin-x64 \
    packages/cli-linux-x64-gnu \
    packages/cli-linux-x64-musl \
    packages/cli-linux-arm64-gnu \
    packages/cli-linux-arm64-musl \
    packages/cli-win32-x64-msvc \
    packages/cli-win32-arm64-msvc \
    packages/tokscale

  cat > packages/cli/package.json <<EOF_MANIFEST
{
  "name": "@tokscale/cli",
  "version": "${version}",
  "optionalDependencies": {
    "@tokscale/cli-darwin-arm64": "${version}",
    "@tokscale/cli-darwin-x64": "${version}",
    "@tokscale/cli-linux-x64-gnu": "${version}",
    "@tokscale/cli-linux-x64-musl": "${version}",
    "@tokscale/cli-linux-arm64-gnu": "${version}",
    "@tokscale/cli-linux-arm64-musl": "${version}",
    "@tokscale/cli-win32-x64-msvc": "${version}",
    "@tokscale/cli-win32-arm64-msvc": "${version}"
  }
}
EOF_MANIFEST

  for pkg in \
    cli-darwin-arm64 \
    cli-darwin-x64 \
    cli-linux-x64-gnu \
    cli-linux-x64-musl \
    cli-linux-arm64-gnu \
    cli-linux-arm64-musl \
    cli-win32-x64-msvc \
    cli-win32-arm64-msvc; do
    cat > "packages/${pkg}/package.json" <<EOF_MANIFEST
{
  "name": "@tokscale/${pkg}",
  "version": "${version}"
}
EOF_MANIFEST
  done

  cat > packages/tokscale/package.json <<EOF_MANIFEST
{
  "name": "tokscale",
  "version": "${version}",
  "dependencies": {
    "@tokscale/cli": "${version}"
  }
}
EOF_MANIFEST
}

write_fake_npm() {
  local path="$1"
  cat > "${path}" <<'EOF_NPM'
#!/usr/bin/env bash
set -euo pipefail

echo "$*" >> "${FAKE_NPM_LOG}"

if [[ "${1:-}" == "whoami" ]]; then
  echo "tokscale-ci"
  exit 0
fi

if [[ "${1:-}" == "publish" ]]; then
  echo "published" >> "${FAKE_NPM_PUBLISH_LOG}"
  exit 0
fi

if [[ "${1:-}" == "view" ]]; then
  spec="${2:-}"
  if [[ -n "${FAKE_NPM_TRANSIENT_SPEC:-}" && "${spec}" == "${FAKE_NPM_TRANSIENT_SPEC}" ]]; then
    echo "npm ERR! code E500" >&2
    echo "npm ERR! registry temporarily unavailable" >&2
    exit 1
  fi
  case "${spec}" in
    *@3.1.0|*@3.1.0-beta.1|*@3.1.0+build-1|*@3.1.0+build-2)
      echo "npm ERR! code E404" >&2
      exit 1
      ;;
    *@3.0.0)
      case "${spec}" in
        @tokscale/cli-darwin-x64@3.0.0|@tokscale/cli@3.0.0)
          echo '"3.0.0"'
          exit 0
          ;;
      esac
      echo "npm ERR! code E404" >&2
      exit 1
      ;;
    *@3.0.1)
      echo "npm ERR! code E404" >&2
      exit 1
      ;;
    @tokscale/*|tokscale)
      echo '"2.1.3"'
      exit 0
      ;;
  esac
fi

echo "unexpected fake npm invocation: $*" >&2
exit 2
EOF_NPM
  chmod +x "${path}"
}

test_refuses_repo_version_ahead_of_npm_without_recovery() {
  local work="${TMP_DIR}/ahead"
  mkdir -p "${work}/scripts"
  cp "${CHECK_SCRIPT}" "${work}/scripts/check-npm-release-state.sh"
  (
    cd "${work}"
    write_release_package_manifests "3.0.1"
    local fake_npm="${TMP_DIR}/fake-npm-ahead"
    write_fake_npm "${fake_npm}"

    local output="${TMP_DIR}/ahead-output.txt"
    if FAKE_NPM_LOG="${TMP_DIR}/ahead-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/ahead-publish.log" \
      NPM_CMD="${fake_npm}" \
      NPM_CHECK_AUTH=0 \
      NEW_VERSION="3.0.1" \
      RELEASE_BASE_VERSION="3.0.0" \
      bash scripts/check-npm-release-state.sh >"${output}" 2>&1; then
      echo "Expected release-state check to reject unrecovered repo-ahead state" >&2
      return 1
    fi

    grep -q "Repository version 3.0.0 is ahead of npm latest 2.1.3 for @tokscale/cli" "${output}"
  )
}

test_recovery_allows_existing_target_versions_for_partial_retry() {
  local work="${TMP_DIR}/recovery-state"
  mkdir -p "${work}/scripts"
  cp "${CHECK_SCRIPT}" "${work}/scripts/check-npm-release-state.sh"
  (
    cd "${work}"
    write_release_package_manifests "3.0.0"
    local fake_npm="${TMP_DIR}/fake-npm-recovery"
    write_fake_npm "${fake_npm}"

    local output="${TMP_DIR}/recovery-state-output.txt"
    FAKE_NPM_LOG="${TMP_DIR}/recovery-state-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/recovery-state-publish.log" \
      NPM_CMD="${fake_npm}" \
      NPM_CHECK_AUTH=0 \
      NEW_VERSION="3.0.0" \
      RELEASE_BASE_VERSION="3.0.0" \
      RELEASE_RECOVERY=true \
      bash scripts/check-npm-release-state.sh >"${output}" 2>&1

    grep -q "@tokscale/cli-darwin-x64@3.0.0 already exists; recovery publish will skip it" "${output}"
    grep -q "npm release-state OK for 3.0.0" "${output}"
  )
}

test_recovery_requires_base_version() {
  local work="${TMP_DIR}/recovery-missing-base"
  mkdir -p "${work}/scripts"
  cp "${CHECK_SCRIPT}" "${work}/scripts/check-npm-release-state.sh"
  (
    cd "${work}"
    write_release_package_manifests "3.0.0"
    local fake_npm="${TMP_DIR}/fake-npm-missing-base"
    write_fake_npm "${fake_npm}"

    local output="${TMP_DIR}/recovery-missing-base-output.txt"
    if FAKE_NPM_LOG="${TMP_DIR}/recovery-missing-base-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/recovery-missing-base-publish.log" \
      NPM_CMD="${fake_npm}" \
      NPM_CHECK_AUTH=0 \
      NEW_VERSION="3.0.0" \
      RELEASE_RECOVERY=true \
      bash scripts/check-npm-release-state.sh >"${output}" 2>&1; then
      echo "Expected recovery without RELEASE_BASE_VERSION to fail" >&2
      return 1
    fi

    grep -q "Recovery target 3.0.0 requires RELEASE_BASE_VERSION" "${output}"
  )
}

test_precheck_fails_on_non_404_npm_lookup_errors() {
  local work="${TMP_DIR}/lookup-error"
  mkdir -p "${work}/scripts"
  cp "${CHECK_SCRIPT}" "${work}/scripts/check-npm-release-state.sh"
  (
    cd "${work}"
    write_release_package_manifests "3.0.1"
    local fake_npm="${TMP_DIR}/fake-npm-lookup-error"
    write_fake_npm "${fake_npm}"

    local output="${TMP_DIR}/lookup-error-output.txt"
    if FAKE_NPM_LOG="${TMP_DIR}/lookup-error-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/lookup-error-publish.log" \
      FAKE_NPM_TRANSIENT_SPEC="@tokscale/cli@3.0.1" \
      NPM_CMD="${fake_npm}" \
      NPM_CHECK_AUTH=0 \
      NEW_VERSION="3.0.1" \
      RELEASE_BASE_VERSION="2.1.3" \
      bash scripts/check-npm-release-state.sh >"${output}" 2>&1; then
      echo "Expected release-state check to fail on non-404 npm lookup errors" >&2
      return 1
    fi

    grep -q "npm view @tokscale/cli@3.0.1 failed" "${output}"
    grep -q "@tokscale/cli@3.0.1: npm lookup failed" "${output}"
  )
}

test_publish_skips_existing_target_version_during_recovery() {
  local work="${TMP_DIR}/publish-skip"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.0.0"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish"
    local publish_log="${TMP_DIR}/publish-skip.log"
    write_fake_npm "${fake_npm}"

    FAKE_NPM_LOG="${TMP_DIR}/publish-skip-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${publish_log}" \
      NPM_CMD="${fake_npm}" \
      RELEASE_RECOVERY=true \
      bash scripts/publish-npm-package.sh packages/cli >"${TMP_DIR}/publish-skip-output.txt" 2>&1

    test ! -e "${publish_log}"
    grep -q "Skipping @tokscale/cli@3.0.0 because it already exists on npm" "${TMP_DIR}/publish-skip-output.txt"
  )
}

test_refuses_to_publish_existing_target_without_recovery() {
  local work="${TMP_DIR}/publish-refuse"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.0.0"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish-refuse"
    write_fake_npm "${fake_npm}"

    local output="${TMP_DIR}/publish-refuse-output.txt"
    if FAKE_NPM_LOG="${TMP_DIR}/publish-refuse-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/publish-refuse.log" \
      NPM_CMD="${fake_npm}" \
      bash scripts/publish-npm-package.sh packages/cli >"${output}" 2>&1; then
      echo "Expected publish helper to refuse existing target without recovery" >&2
      return 1
    fi

    grep -q "@tokscale/cli@3.0.0 already exists on npm; set RELEASE_RECOVERY=true to skip already-published packages" "${output}"
  )
}

test_publish_fails_on_non_404_npm_lookup_errors() {
  local work="${TMP_DIR}/publish-lookup-error"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.0.1"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish-lookup-error"
    write_fake_npm "${fake_npm}"

    local output="${TMP_DIR}/publish-lookup-error-output.txt"
    if FAKE_NPM_LOG="${TMP_DIR}/publish-lookup-error-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/publish-lookup-error.log" \
      FAKE_NPM_TRANSIENT_SPEC="@tokscale/cli@3.0.1" \
      NPM_CMD="${fake_npm}" \
      bash scripts/publish-npm-package.sh packages/cli >"${output}" 2>&1; then
      echo "Expected publish helper to fail on non-404 npm lookup errors" >&2
      return 1
    fi

    grep -q "npm view @tokscale/cli@3.0.1 failed" "${output}"
    grep -q "Unable to verify @tokscale/cli@3.0.1 on npm" "${output}"
  )
}

test_prerelease_publish_uses_prerelease_dist_tag() {
  local work="${TMP_DIR}/publish-prerelease-tag"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.1.0-beta.1"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish-prerelease-tag"
    write_fake_npm "${fake_npm}"

    FAKE_NPM_LOG="${TMP_DIR}/publish-prerelease-tag-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/publish-prerelease-tag-publish.log" \
      NPM_CMD="${fake_npm}" \
      bash scripts/publish-npm-package.sh packages/cli >"${TMP_DIR}/publish-prerelease-tag-output.txt" 2>&1

    grep -q '^publish --access public --tag beta$' "${TMP_DIR}/publish-prerelease-tag-npm.log"
  )
}

test_prerelease_publish_rejects_explicit_latest_dist_tag() {
  local work="${TMP_DIR}/publish-prerelease-latest-tag"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.1.0-beta.1"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish-prerelease-latest-tag"
    write_fake_npm "${fake_npm}"

    local output="${TMP_DIR}/publish-prerelease-latest-tag-output.txt"
    if FAKE_NPM_LOG="${TMP_DIR}/publish-prerelease-latest-tag-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/publish-prerelease-latest-tag-publish.log" \
      NPM_CMD="${fake_npm}" \
      NPM_DIST_TAG="latest" \
      bash scripts/publish-npm-package.sh packages/cli >"${output}" 2>&1; then
      echo "Expected publish helper to reject prerelease latest dist-tag" >&2
      return 1
    fi

    grep -q "Refusing to publish prerelease @tokscale/cli@3.1.0-beta.1 with npm dist-tag latest" "${output}"
    if [[ -s "${TMP_DIR}/publish-prerelease-latest-tag-npm.log" ]]; then
      ! grep -q '^publish ' "${TMP_DIR}/publish-prerelease-latest-tag-npm.log"
    fi
  )
}

test_stable_publish_uses_latest_dist_tag() {
  local work="${TMP_DIR}/publish-stable-tag"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.1.0"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish-stable-tag"
    write_fake_npm "${fake_npm}"

    FAKE_NPM_LOG="${TMP_DIR}/publish-stable-tag-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/publish-stable-tag-publish.log" \
      NPM_CMD="${fake_npm}" \
      bash scripts/publish-npm-package.sh packages/cli >"${TMP_DIR}/publish-stable-tag-output.txt" 2>&1

    grep -q '^publish --access public --tag latest$' "${TMP_DIR}/publish-stable-tag-npm.log"
  )
}

test_stable_build_metadata_publish_uses_latest_dist_tag() {
  local work="${TMP_DIR}/publish-stable-build-metadata-tag"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.1.0+build-1"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish-stable-build-metadata-tag"
    write_fake_npm "${fake_npm}"

    FAKE_NPM_LOG="${TMP_DIR}/publish-stable-build-metadata-tag-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/publish-stable-build-metadata-tag-publish.log" \
      NPM_CMD="${fake_npm}" \
      bash scripts/publish-npm-package.sh packages/cli >"${TMP_DIR}/publish-stable-build-metadata-tag-output.txt" 2>&1

    grep -q '^publish --access public --tag latest$' "${TMP_DIR}/publish-stable-build-metadata-tag-npm.log"
  )
}

test_stable_build_metadata_allows_explicit_latest_dist_tag() {
  local work="${TMP_DIR}/publish-stable-build-metadata-explicit-latest"
  mkdir -p "${work}/scripts" "${work}/packages/cli"
  cp "${PUBLISH_SCRIPT}" "${work}/scripts/publish-npm-package.sh"
  (
    cd "${work}"
    cat > packages/cli/package.json <<'EOF_MANIFEST'
{
  "name": "@tokscale/cli",
  "version": "3.1.0+build-2"
}
EOF_MANIFEST
    local fake_npm="${TMP_DIR}/fake-npm-publish-stable-build-metadata-explicit-latest"
    write_fake_npm "${fake_npm}"

    FAKE_NPM_LOG="${TMP_DIR}/publish-stable-build-metadata-explicit-latest-npm.log" \
      FAKE_NPM_PUBLISH_LOG="${TMP_DIR}/publish-stable-build-metadata-explicit-latest-publish.log" \
      NPM_CMD="${fake_npm}" \
      NPM_DIST_TAG="latest" \
      bash scripts/publish-npm-package.sh packages/cli >"${TMP_DIR}/publish-stable-build-metadata-explicit-latest-output.txt" 2>&1

    grep -q '^publish --access public --tag latest$' "${TMP_DIR}/publish-stable-build-metadata-explicit-latest-npm.log"
  )
}

test_refuses_repo_version_ahead_of_npm_without_recovery
test_recovery_allows_existing_target_versions_for_partial_retry
test_recovery_requires_base_version
test_precheck_fails_on_non_404_npm_lookup_errors
test_publish_skips_existing_target_version_during_recovery
test_refuses_to_publish_existing_target_without_recovery
test_publish_fails_on_non_404_npm_lookup_errors
test_prerelease_publish_uses_prerelease_dist_tag
test_prerelease_publish_rejects_explicit_latest_dist_tag
test_stable_publish_uses_latest_dist_tag
test_stable_build_metadata_publish_uses_latest_dist_tag
test_stable_build_metadata_allows_explicit_latest_dist_tag

echo "npm release-state tests passed"
