#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="${ROOT_DIR}/scripts/check-workflow-toolchain-paths.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

write_good_tree() {
  local work="$1"
  mkdir -p "${work}/.github/workflows"
  cat > "${work}/rust-toolchain.toml" <<'EOF_TOML'
[toolchain]
channel = "1.98.0"
EOF_TOML
  cat > "${work}/.github/workflows/rust_anchored.yml" <<'EOF_YAML'
name: Rust Anchored

on:
  workflow_dispatch:
  push:
    branches: [main]
    paths: &rust_paths
      - 'crates/**'
      - 'Cargo.lock'
      - 'rust-toolchain.toml'
  pull_request:
    branches: [main]
    paths: *rust_paths

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@1.98.0
      - run: cargo test --workspace
EOF_YAML
  cat > "${work}/.github/workflows/rust_unfiltered.yml" <<'EOF_YAML'
name: Rust Unfiltered

on:
  workflow_dispatch:

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@1.98.0
      - run: cargo build --release
EOF_YAML
  cat > "${work}/.github/workflows/frontend.yml" <<'EOF_YAML'
name: Frontend

on:
  pull_request:
    paths:
      # cargo is only mentioned in this comment, so the filter is exempt
      - "packages/frontend/**"

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - run: bun test
EOF_YAML
}

run_check() {
  local work="$1"
  local output="$2"
  (cd "${work}" && python3 "${SCRIPT_UNDER_TEST}" >"${output}" 2>&1)
}

test_accepts_rust_workflows_that_watch_the_pin() {
  local work="${TMP_DIR}/good"
  write_good_tree "${work}"

  local output="${TMP_DIR}/good-output.txt"
  run_check "${work}" "${output}"

  grep -q "Workflow toolchain paths OK: 2 Rust workflows checked (rust_anchored.yml, rust_unfiltered.yml)" "${output}"
}

test_rejects_rust_workflow_whose_filter_omits_the_pin() {
  local work="${TMP_DIR}/omitted"
  write_good_tree "${work}"
  sed -i.bak "/rust-toolchain.toml/d" "${work}/.github/workflows/rust_anchored.yml"
  rm "${work}/.github/workflows/rust_anchored.yml.bak"

  local output="${TMP_DIR}/omitted-output.txt"
  if run_check "${work}" "${output}"; then
    echo "Expected toolchain path check to reject a filter that omits rust-toolchain.toml" >&2
    return 1
  fi

  grep -q "rust_anchored.yml:7: \`paths:\` filter omits rust-toolchain.toml" "${output}"
  # Deleting the entry shifts the alias line up by one.
  grep -q "rust_anchored.yml:12: \`paths:\` filter omits rust-toolchain.toml" "${output}"
}

test_rejects_pin_missing_from_one_trigger_only() {
  local work="${TMP_DIR}/one-trigger"
  write_good_tree "${work}"
  python3 - "${work}/.github/workflows/rust_anchored.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace(
    "    paths: *rust_paths\n",
    "    paths:\n      - 'crates/**'\n",
    1,
)
path.write_text(text)
PY

  local output="${TMP_DIR}/one-trigger-output.txt"
  if run_check "${work}" "${output}"; then
    echo "Expected toolchain path check to reject a pull_request filter that omits rust-toolchain.toml" >&2
    return 1
  fi

  grep -q "rust_anchored.yml:13: \`paths:\` filter omits rust-toolchain.toml" "${output}"
  if grep -q "rust_anchored.yml:7:" "${output}"; then
    echo "Expected the push filter, which lists rust-toolchain.toml, to pass" >&2
    return 1
  fi
}

test_rejects_pin_listed_in_paths_ignore() {
  local work="${TMP_DIR}/paths-ignore"
  write_good_tree "${work}"
  python3 - "${work}/.github/workflows/rust_unfiltered.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace(
    "  workflow_dispatch:\n",
    "  pull_request:\n    paths-ignore: [\"docs/**\", \"rust-toolchain.toml\"]\n",
    1,
)
path.write_text(text)
PY

  local output="${TMP_DIR}/paths-ignore-output.txt"
  if run_check "${work}" "${output}"; then
    echo "Expected toolchain path check to reject a paths-ignore that lists rust-toolchain.toml" >&2
    return 1
  fi

  grep -q "rust_unfiltered.yml:5: \`paths-ignore:\` filter lists rust-toolchain.toml" "${output}"
}

test_rejects_missing_toolchain_pin() {
  local work="${TMP_DIR}/no-pin"
  write_good_tree "${work}"
  rm "${work}/rust-toolchain.toml"

  local output="${TMP_DIR}/no-pin-output.txt"
  if run_check "${work}" "${output}"; then
    echo "Expected toolchain path check to reject a tree without rust-toolchain.toml" >&2
    return 1
  fi

  grep -q "Missing rust-toolchain.toml" "${output}"
}

test_rejects_tree_without_rust_workflows() {
  local work="${TMP_DIR}/no-rust"
  write_good_tree "${work}"
  rm "${work}/.github/workflows/rust_anchored.yml" "${work}/.github/workflows/rust_unfiltered.yml"

  local output="${TMP_DIR}/no-rust-output.txt"
  if run_check "${work}" "${output}"; then
    echo "Expected toolchain path check to reject a tree with no Rust workflows" >&2
    return 1
  fi

  grep -q "No Rust workflows found" "${output}"
}

test_accepts_rust_workflows_that_watch_the_pin
test_rejects_rust_workflow_whose_filter_omits_the_pin
test_rejects_pin_missing_from_one_trigger_only
test_rejects_pin_listed_in_paths_ignore
test_rejects_missing_toolchain_pin
test_rejects_tree_without_rust_workflows

echo "workflow toolchain path tests passed"
