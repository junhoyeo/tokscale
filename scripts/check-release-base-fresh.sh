#!/usr/bin/env bash
# Fail-fast guard: verify the dispatched release base still matches the branch tip.
#
# This is an EARLY copy of the staleness check in scripts/prepare-release-provenance.sh.
# It exists only so a stale dispatch fails in seconds instead of after the full
# cross-platform binary build (~20 min). It is NOT the authoritative check: the branch
# can still move while those builds run, so the check inside
# scripts/prepare-release-provenance.sh is the one that actually protects the release.
# Do not delete that one in favour of this.
set -euo pipefail

RELEASE_REF_NAME="${RELEASE_REF_NAME:-}"
RELEASE_REF_TYPE="${RELEASE_REF_TYPE:-branch}"
EXPECTED_RELEASE_BASE_SHA="${EXPECTED_RELEASE_BASE_SHA:-}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

[[ -n "${RELEASE_REF_NAME}" ]] || fail "RELEASE_REF_NAME is required"
[[ -n "${EXPECTED_RELEASE_BASE_SHA}" ]] || fail "EXPECTED_RELEASE_BASE_SHA is required"
[[ "${RELEASE_REF_TYPE}" == "branch" ]] || fail "Release publishing must run from a branch ref"

git check-ref-format --branch "${RELEASE_REF_NAME}" >/dev/null ||
  fail "Invalid release branch name: ${RELEASE_REF_NAME}"

# ls-remote instead of fetch: this runs on the shallow actions/checkout clone, and we
# only need the tip SHA, not the objects behind it.
remote_line="$(git ls-remote --exit-code origin "refs/heads/${RELEASE_REF_NAME}")" ||
  fail "Could not resolve origin/${RELEASE_REF_NAME}"
remote_sha="$(printf '%s\n' "${remote_line}" | awk 'NR==1{print $1}')"
[[ -n "${remote_sha}" ]] || fail "Could not parse tip SHA for origin/${RELEASE_REF_NAME}"

if [[ "${remote_sha}" != "${EXPECTED_RELEASE_BASE_SHA}" ]]; then
  fail "Release base is stale: origin/${RELEASE_REF_NAME} is ${remote_sha}, expected ${EXPECTED_RELEASE_BASE_SHA}. Re-run the publish workflow from the updated branch before publishing npm packages."
fi

echo "Release base is current: origin/${RELEASE_REF_NAME} is ${remote_sha}"
