#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v bun >/dev/null 2>&1; then
  echo "bun is required for launcher smoke tests" >&2
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "node is required for launcher smoke tests" >&2
  exit 1
fi

BUN_BIN="${BUN_BIN:-$(command -v bun)}"
NODE_BIN="${NODE_BIN:-$(command -v node)}"
LDD_BIN="${LDD_BIN:-$(command -v ldd || true)}"

PLATFORM_PACKAGE="$(node --input-type=module <<'NODE'
import { execSync } from "node:child_process";

function detectLibcKind() {
  if (process.platform !== "linux") {
    return null;
  }

  const report = process.report?.getReport?.();
  if (report?.header?.glibcVersionRuntime) {
    return "gnu";
  }

  if (
    Array.isArray(report?.sharedObjects) &&
    report.sharedObjects.some((obj) => obj.toLowerCase().includes("musl"))
  ) {
    return "musl";
  }

  try {
    const output = execSync("ldd --version", {
      encoding: "utf-8",
      stdio: ["ignore", "pipe", "pipe"],
    }).toLowerCase();
    return output.includes("musl") ? "musl" : "gnu";
  } catch {
    throw new Error("Unable to determine Linux libc kind for launcher smoke tests");
  }
}

const arch = process.arch;

if (process.platform === "darwin") {
  if (arch === "arm64") console.log("cli-darwin-arm64");
  else if (arch === "x64") console.log("cli-darwin-x64");
  else process.exit(1);
} else if (process.platform === "linux") {
  const libc = detectLibcKind();
  if (arch === "arm64") console.log(libc === "musl" ? "cli-linux-arm64-musl" : "cli-linux-arm64-gnu");
  else if (arch === "x64") console.log(libc === "musl" ? "cli-linux-x64-musl" : "cli-linux-x64-gnu");
  else process.exit(1);
} else {
  process.exit(1);
}
NODE
)"

if [[ -z "${PLATFORM_PACKAGE}" ]]; then
  echo "Unsupported platform for launcher smoke tests: $(uname -s) / $(uname -m)" >&2
  exit 1
fi

echo "Building CLI wrapper and native binary..."
bun run --cwd packages/cli build >/dev/null
cargo build --release -p tokscale-cli >/dev/null

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/tokscale-launcher-smoke.XXXXXX")"
cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

CLI_STAGE="${TMP_ROOT}/cli"
WRAPPER_STAGE="${TMP_ROOT}/tokscale"
PLATFORM_STAGE="${TMP_ROOT}/${PLATFORM_PACKAGE}"
INSTALL_DIR="${TMP_ROOT}/install"
NPM_CACHE="${TMP_ROOT}/npm-cache"
EMPTY_PATH_DIR="${TMP_ROOT}/empty-path"
BUN_ONLY_DIR="${TMP_ROOT}/bun-only-path"
NODE_ONLY_DIR="${TMP_ROOT}/node-only-path"

cp -R packages/cli "${CLI_STAGE}"
cp -R packages/tokscale "${WRAPPER_STAGE}"
cp -R "packages/${PLATFORM_PACKAGE}" "${PLATFORM_STAGE}"
mkdir -p \
  "${PLATFORM_STAGE}/bin" \
  "${INSTALL_DIR}" \
  "${NPM_CACHE}" \
  "${EMPTY_PATH_DIR}" \
  "${BUN_ONLY_DIR}" \
  "${NODE_ONLY_DIR}"
cp target/release/tokscale "${PLATFORM_STAGE}/bin/tokscale"

chmod +x "${CLI_STAGE}/bin.js" "${WRAPPER_STAGE}/bin.js" "${PLATFORM_STAGE}/bin/tokscale"

ln -s "${BUN_BIN}" "${BUN_ONLY_DIR}/bun"
ln -s "${NODE_BIN}" "${NODE_ONLY_DIR}/node"
if [[ -n "${LDD_BIN}" ]]; then
  ln -s "${LDD_BIN}" "${BUN_ONLY_DIR}/ldd"
  ln -s "${LDD_BIN}" "${NODE_ONLY_DIR}/ldd"
fi

BUN_ONLY_PATH="${BUN_ONLY_DIR}"
NODE_ONLY_PATH="${NODE_ONLY_DIR}"

CLI_TGZ="$(cd "${CLI_STAGE}" && NPM_CONFIG_CACHE="${NPM_CACHE}" npm pack --silent)"
WRAPPER_TGZ="$(cd "${WRAPPER_STAGE}" && NPM_CONFIG_CACHE="${NPM_CACHE}" npm pack --silent)"
PLATFORM_TGZ="$(cd "${PLATFORM_STAGE}" && NPM_CONFIG_CACHE="${NPM_CACHE}" npm pack --silent)"

echo "Installing local tarballs with Bun..."
(
  cd "${INSTALL_DIR}"
  env PATH="${BUN_ONLY_PATH}" bun add \
    "${CLI_STAGE}/${CLI_TGZ}" \
    "${WRAPPER_STAGE}/${WRAPPER_TGZ}" \
    "${PLATFORM_STAGE}/${PLATFORM_TGZ}" >/dev/null
)

INSTALLED_BIN="${INSTALL_DIR}/node_modules/.bin/tokscale"
if [[ ! -e "${INSTALLED_BIN}" ]]; then
  echo "Installed tokscale launcher not found at ${INSTALLED_BIN}" >&2
  exit 1
fi

echo "Checking source-tree wrapper with Node-only PATH..."
env PATH="${NODE_ONLY_PATH}" "${ROOT_DIR}/packages/tokscale/bin.js" --version >/dev/null

echo "Checking installed launcher via Bun runtime..."
INSTALLED_VERSION_BUN="$(env PATH="${BUN_ONLY_PATH}" bun "${INSTALLED_BIN}" --version)"
[[ "${INSTALLED_VERSION_BUN}" == tokscale* ]] || {
  echo "Unexpected Bun launcher output: ${INSTALLED_VERSION_BUN}" >&2
  exit 1
}

echo "Checking installed launcher with Node-only PATH..."
INSTALLED_VERSION_NODE="$(env PATH="${NODE_ONLY_PATH}" "${INSTALLED_BIN}" --version)"
[[ "${INSTALLED_VERSION_NODE}" == tokscale* ]] || {
  echo "Unexpected Node-only launcher output: ${INSTALLED_VERSION_NODE}" >&2
  exit 1
}

echo "Checking error path with no Node/Bun in PATH..."
set +e
ERROR_OUTPUT="$(env PATH="${EMPTY_PATH_DIR}" "${INSTALLED_BIN}" --version 2>&1)"
ERROR_CODE=$?
set -e
if [[ ${ERROR_CODE} -eq 0 ]]; then
  echo "Expected launcher to fail when neither Node nor Bun is available" >&2
  exit 1
fi
[[ "${ERROR_OUTPUT}" == *"node"* ]] || {
  echo "Unexpected launcher error output: ${ERROR_OUTPUT}" >&2
  exit 1
}

echo "Launcher smoke tests passed."
