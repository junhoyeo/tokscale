#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

FAKE_BIN="${TMP_DIR}/bin"
GH_LOG="${TMP_DIR}/gh.log"
CURL_PAYLOADS="${TMP_DIR}/curl-payloads.jsonl"
mkdir -p "${FAKE_BIN}"
: >"${GH_LOG}"
: >"${CURL_PAYLOADS}"

cat >"${FAKE_BIN}/git" <<'EOF_GIT'
#!/usr/bin/env bash
set -euo pipefail

case "$*" in
  "describe --tags --abbrev=0 HEAD^")
    printf '%s\n' 'v1.2.2'
    ;;
  "log -1 --format=%cI v1.2.2")
    printf '%s\n' '2026-07-01T00:00:00+00:00'
    ;;
  "log v1.2.2..HEAD --format=%H%x1f%s%x1f%an%x1f%ae --no-merges")
    printf 'abc123\x1ffix(cli): local subject\x1fAlice\x1falice@example.com\n'
    printf 'bump456\x1fchore: bump version to 1.2.3\x1fRelease Bot\x1fbot@example.com\n'
    ;;
  *)
    printf 'unexpected git invocation: %s\n' "$*" >&2
    exit 1
    ;;
esac
EOF_GIT

cat >"${FAKE_BIN}/gh" <<'EOF_GH'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "$*" >>"${FAKE_GH_LOG:?}"
case "$*" in
  "api repos/acme/tokscale/commits/abc123/pulls")
    printf '%s\n' '[{"number":77,"title":"fix(cli): keep release notifications deterministic","state":"closed","merged_at":"2026-07-02T00:00:00Z","user":{"login":"alice"}}]'
    ;;
  "pr list --repo acme/tokscale --state merged --author alice --json number,mergedAt --limit 200")
    printf '%s\n' '[{"number":77,"mergedAt":"2026-07-02T00:00:00Z"}]'
    ;;
  *)
    printf 'unexpected gh invocation: %s\n' "$*" >&2
    exit 1
    ;;
esac
EOF_GH

cat >"${FAKE_BIN}/curl" <<'EOF_CURL'
#!/usr/bin/env bash
set -euo pipefail

payload=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -sf)
      ;;
    -H)
      shift
      [ "${1:-}" = "Content-Type: application/json" ] || exit 1
      ;;
    -d)
      shift
      payload="${1:-}"
      ;;
    https://discord.invalid/webhook)
      url="$1"
      ;;
    *)
      printf 'unexpected curl argument: %s\n' "$1" >&2
      exit 1
      ;;
  esac
  shift
done

[ -n "${payload}" ] || { echo "curl payload is missing" >&2; exit 1; }
[ "${url}" = "https://discord.invalid/webhook" ] || { echo "curl URL is missing" >&2; exit 1; }
printf '%s\n' "${payload}" >>"${FAKE_CURL_PAYLOADS:?}"
EOF_CURL

chmod +x "${FAKE_BIN}/git" "${FAKE_BIN}/gh" "${FAKE_BIN}/curl"

export PATH="${FAKE_BIN}:${PATH}"
export GITHUB_REPOSITORY="acme/tokscale"
export FAKE_GH_LOG="${GH_LOG}"
export FAKE_CURL_PAYLOADS="${CURL_PAYLOADS}"

assert_contains() {
  local text="$1"
  local expected="$2"
  if ! grep -Fq -- "${expected}" <<<"${text}"; then
    printf 'expected output to contain: %s\n' "${expected}" >&2
    return 1
  fi
}

assert_excludes() {
  local text="$1"
  local unexpected="$2"
  if grep -Fq -- "${unexpected}" <<<"${text}"; then
    printf 'expected output to exclude: %s\n' "${unexpected}" >&2
    return 1
  fi
}

cd "${ROOT_DIR}"
notes="$(bun scripts/generate-release-notes.ts 1.2.3)"
assert_contains "${notes}" '<div align="center">'
assert_contains "${notes}" '# `tokscale@v1.2.3` is here!'
assert_contains "${notes}" '* fix(cli): keep release notifications deterministic by @alice in https://github.com/acme/tokscale/pull/77'
assert_contains "${notes}" '* @alice made their first contribution in https://github.com/acme/tokscale/pull/77'
assert_contains "${notes}" '**Full Changelog**: https://github.com/acme/tokscale/compare/v1.2.2...v1.2.3'
assert_excludes "${notes}" 'chore: bump version'

DISCORD_WEBHOOK_URL="https://discord.invalid/webhook" \
  bash scripts/post-discord-release.sh 1.2.3 >"${TMP_DIR}/discord-output.txt"

jq -s -e 'length == 1' "${CURL_PAYLOADS}" >/dev/null
content="$(jq -r -s '.[0].content' "${CURL_PAYLOADS}")"
assert_contains "${content}" '## `tokscale@v1.2.3` is here!'
assert_contains "${content}" '* fix(cli): keep release notifications deterministic by @alice in https://github.com/acme/tokscale/pull/77'
assert_contains "${content}" '* @alice made their first contribution in https://github.com/acme/tokscale/pull/77'
assert_contains "${content}" '**Full Changelog**: https://github.com/acme/tokscale/compare/v1.2.2...v1.2.3'
assert_excludes "${content}" '<div'
assert_excludes "${content}" '.github/assets/hero-v2.png'
if grep -Fxq '# `tokscale@v1.2.3` is here!' <<<"${content}"; then
  echo "Discord content retained the release-note H1" >&2
  exit 1
fi

grep -Fxq 'api repos/acme/tokscale/commits/abc123/pulls' "${GH_LOG}"
grep -Fxq 'pr list --repo acme/tokscale --state merged --author alice --json number,mergedAt --limit 200' "${GH_LOG}"

echo "release notes and Discord notification tests passed"
