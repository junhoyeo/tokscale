#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

NEW_VERSION="${NEW_VERSION:-}"
RELEASE_REF_NAME="${RELEASE_REF_NAME:-}"
RELEASE_REF_TYPE="${RELEASE_REF_TYPE:-branch}"
EXPECTED_RELEASE_BASE_SHA="${EXPECTED_RELEASE_BASE_SHA:-}"
GITHUB_OUTPUT="${GITHUB_OUTPUT:-}"
RELEASE_RECOVERY="${RELEASE_RECOVERY:-false}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

release_manifest_paths() {
  python3 - <<'PY'
import json
import pathlib

root = pathlib.Path(".")
paths = [
    pathlib.Path("Cargo.toml"),
    pathlib.Path("Cargo.lock"),
    pathlib.Path("packages/cli/package.json"),
]
cli = json.loads((root / paths[-1]).read_text())
for package_name in sorted(cli.get("optionalDependencies", {})):
    if not package_name.startswith("@tokscale/cli-"):
        raise SystemExit(f"Unexpected optional dependency package name: {package_name}")
    paths.append(pathlib.Path("packages") / package_name.removeprefix("@tokscale/") / "package.json")
paths.append(pathlib.Path("packages/tokscale/package.json"))
for path in paths:
    print(path.as_posix())
PY
}

[[ -n "${NEW_VERSION}" ]] || fail "NEW_VERSION is required"
[[ -n "${RELEASE_REF_NAME}" ]] || fail "RELEASE_REF_NAME is required"
[[ -n "${EXPECTED_RELEASE_BASE_SHA}" ]] || fail "EXPECTED_RELEASE_BASE_SHA is required"
[[ "${RELEASE_REF_TYPE}" == "branch" ]] || fail "Release publishing must run from a branch ref"

git check-ref-format --branch "${RELEASE_REF_NAME}" >/dev/null ||
  fail "Invalid release branch name: ${RELEASE_REF_NAME}"
git rev-parse --verify "${EXPECTED_RELEASE_BASE_SHA}^{commit}" >/dev/null ||
  fail "Expected release base is not a commit: ${EXPECTED_RELEASE_BASE_SHA}"

local_sha="$(git rev-parse HEAD)"
if [[ "${local_sha}" != "${EXPECTED_RELEASE_BASE_SHA}" ]]; then
  fail "Checked-out release base ${local_sha} does not match expected ${EXPECTED_RELEASE_BASE_SHA}"
fi

remote_tracking_ref="refs/remotes/origin/${RELEASE_REF_NAME}"
git fetch --no-tags origin "+refs/heads/${RELEASE_REF_NAME}:${remote_tracking_ref}"
remote_sha="$(git rev-parse "${remote_tracking_ref}^{commit}")"
if [[ "${remote_sha}" != "${EXPECTED_RELEASE_BASE_SHA}" ]]; then
  fail "Release base is stale: origin/${RELEASE_REF_NAME} is ${remote_sha}, expected ${EXPECTED_RELEASE_BASE_SHA}. Re-run the publish workflow from the updated branch before publishing npm packages."
fi

if git ls-remote --exit-code --tags origin "refs/tags/v${NEW_VERSION}" >/dev/null 2>&1; then
  if [[ "${RELEASE_RECOVERY}" != "true" ]]; then
    fail "Tag v${NEW_VERSION} already exists. Choose a new version before publishing npm packages."
  fi

  tag_sha="$(git ls-remote --tags origin "refs/tags/v${NEW_VERSION}" | awk '{print $1}' | head -n1)"
  if [[ "${tag_sha}" != "${local_sha}" ]]; then
    fail "Tag v${NEW_VERSION} points at ${tag_sha}, expected ${local_sha}"
  fi
fi

bash scripts/check-version-coherence.sh --expect-version "${NEW_VERSION}"

mapfile -t MANIFEST_PATHS < <(release_manifest_paths)
git add -- "${MANIFEST_PATHS[@]}"
if git diff --cached --quiet; then
  if [[ "${RELEASE_RECOVERY}" == "true" ]]; then
    release_commit="${local_sha}"
    echo "Reusing release commit ${release_commit} for ${NEW_VERSION}"
    if [[ -n "${GITHUB_OUTPUT}" ]]; then
      echo "release_commit=${release_commit}" >> "${GITHUB_OUTPUT}"
    fi
    exit 0
  fi

  fail "No release manifest changes staged for ${NEW_VERSION}. Use RELEASE_RECOVERY=true only when retrying an already committed release."
fi

if [[ "${RELEASE_RECOVERY}" == "true" ]]; then
  fail "Recovery requested for ${NEW_VERSION}, but release manifest changes are staged. Retry recovery from the committed release ref or run a normal release for a new version."
fi

git commit -m "chore: bump version to ${NEW_VERSION}"
release_commit="$(git rev-parse HEAD)"
git push origin "HEAD:refs/heads/${RELEASE_REF_NAME}"

echo "Release commit ${release_commit} pushed to ${RELEASE_REF_NAME}"
if [[ -n "${GITHUB_OUTPUT}" ]]; then
  echo "release_commit=${release_commit}" >> "${GITHUB_OUTPUT}"
fi
