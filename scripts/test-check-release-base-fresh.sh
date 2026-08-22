#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="${ROOT_DIR}/scripts/check-release-base-fresh.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

git_config() {
  git config user.name "Test Runner"
  git config user.email "test@example.com"
}

create_origin_with_initial_commit() {
  local origin="$1"
  local seed="$2"

  git init --bare "${origin}" >/dev/null
  git init "${seed}" >/dev/null
  (
    cd "${seed}"
    git_config
    git checkout -b main >/dev/null 2>&1
    echo "seed" > README.md
    git add README.md
    git commit -m "seed" >/dev/null
    git remote add origin "${origin}"
    git push origin main >/dev/null
  )
  git --git-dir="${origin}" symbolic-ref HEAD refs/heads/main
}

advance_origin_main() {
  local origin="$1"
  local advancer="$2"

  git clone "${origin}" "${advancer}" >/dev/null 2>&1
  (
    cd "${advancer}"
    git_config
    git checkout main >/dev/null 2>&1
    echo "advance" >> README.md
    git add README.md
    git commit -m "advance main" >/dev/null
    git push origin main >/dev/null
  )
}

run_check() {
  local repo="$1"
  local expected_sha="$2"
  local ref_type="${3:-branch}"
  local ref_name="${4:-main}"

  (
    cd "${repo}"
    RELEASE_REF_NAME="${ref_name}" \
      RELEASE_REF_TYPE="${ref_type}" \
      EXPECTED_RELEASE_BASE_SHA="${expected_sha}" \
      bash "${SCRIPT_UNDER_TEST}"
  )
}

test_current_base_passes() {
  local origin="${TMP_DIR}/current-origin.git"
  local seed="${TMP_DIR}/current-seed"
  local work="${TMP_DIR}/current-work"
  create_origin_with_initial_commit "${origin}" "${seed}"

  git clone "${origin}" "${work}" >/dev/null 2>&1
  local dispatch_sha
  dispatch_sha="$(git -C "${work}" rev-parse HEAD)"

  local output="${TMP_DIR}/current-output.txt"
  run_check "${work}" "${dispatch_sha}" >"${output}" 2>&1

  grep -q "Release base is current" "${output}"
}

test_stale_base_fails() {
  local origin="${TMP_DIR}/stale-origin.git"
  local seed="${TMP_DIR}/stale-seed"
  local work="${TMP_DIR}/stale-work"
  local advancer="${TMP_DIR}/stale-advancer"
  create_origin_with_initial_commit "${origin}" "${seed}"

  git clone "${origin}" "${work}" >/dev/null 2>&1
  local dispatch_sha
  dispatch_sha="$(git -C "${work}" rev-parse HEAD)"

  advance_origin_main "${origin}" "${advancer}"

  local output="${TMP_DIR}/stale-output.txt"
  if run_check "${work}" "${dispatch_sha}" >"${output}" 2>&1; then
    echo "Expected stale release base to fail" >&2
    return 1
  fi

  grep -q "Release base is stale" "${output}"
}

test_shallow_clone_still_detects_stale_base() {
  # The real workflow runs on actions/checkout's depth-1 clone, so the guard must
  # not depend on having history or remote-tracking refs locally.
  local origin="${TMP_DIR}/shallow-origin.git"
  local seed="${TMP_DIR}/shallow-seed"
  local work="${TMP_DIR}/shallow-work"
  local advancer="${TMP_DIR}/shallow-advancer"
  create_origin_with_initial_commit "${origin}" "${seed}"

  git clone --depth 1 "${origin}" "${work}" >/dev/null 2>&1
  local dispatch_sha
  dispatch_sha="$(git -C "${work}" rev-parse HEAD)"

  advance_origin_main "${origin}" "${advancer}"

  local output="${TMP_DIR}/shallow-output.txt"
  if run_check "${work}" "${dispatch_sha}" >"${output}" 2>&1; then
    echo "Expected stale release base to fail on a shallow clone" >&2
    return 1
  fi

  grep -q "Release base is stale" "${output}"
}

test_tag_ref_is_rejected() {
  local origin="${TMP_DIR}/tag-origin.git"
  local seed="${TMP_DIR}/tag-seed"
  local work="${TMP_DIR}/tag-work"
  create_origin_with_initial_commit "${origin}" "${seed}"

  git clone "${origin}" "${work}" >/dev/null 2>&1
  local dispatch_sha
  dispatch_sha="$(git -C "${work}" rev-parse HEAD)"

  local output="${TMP_DIR}/tag-output.txt"
  if run_check "${work}" "${dispatch_sha}" "tag" >"${output}" 2>&1; then
    echo "Expected tag ref type to fail" >&2
    return 1
  fi

  grep -q "must run from a branch ref" "${output}"
}

test_missing_expected_sha_is_rejected() {
  local origin="${TMP_DIR}/missing-origin.git"
  local seed="${TMP_DIR}/missing-seed"
  local work="${TMP_DIR}/missing-work"
  create_origin_with_initial_commit "${origin}" "${seed}"

  git clone "${origin}" "${work}" >/dev/null 2>&1

  local output="${TMP_DIR}/missing-output.txt"
  if run_check "${work}" "" >"${output}" 2>&1; then
    echo "Expected missing EXPECTED_RELEASE_BASE_SHA to fail" >&2
    return 1
  fi

  grep -q "EXPECTED_RELEASE_BASE_SHA is required" "${output}"
}

test_unknown_branch_is_rejected() {
  local origin="${TMP_DIR}/unknown-origin.git"
  local seed="${TMP_DIR}/unknown-seed"
  local work="${TMP_DIR}/unknown-work"
  create_origin_with_initial_commit "${origin}" "${seed}"

  git clone "${origin}" "${work}" >/dev/null 2>&1
  local dispatch_sha
  dispatch_sha="$(git -C "${work}" rev-parse HEAD)"

  local output="${TMP_DIR}/unknown-output.txt"
  if run_check "${work}" "${dispatch_sha}" "branch" "no-such-branch" >"${output}" 2>&1; then
    echo "Expected unknown branch to fail" >&2
    return 1
  fi

  grep -q "Could not resolve origin/no-such-branch" "${output}"
}

test_current_base_passes
test_stale_base_fails
test_shallow_clone_still_detects_stale_base
test_tag_ref_is_rejected
test_missing_expected_sha_is_rejected
test_unknown_branch_is_rejected

echo "check-release-base-fresh tests passed"
