#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

ARTIFACT_ROOT="${RELEASE_ARTIFACT_ROOT:-release-artifacts}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required for release artifact smoke tests"
}

require_cmd bun
require_cmd node
require_cmd npm

[[ -d "${ARTIFACT_ROOT}" ]] || fail "Missing release artifact directory: ${ARTIFACT_ROOT}"

BUN_BIN="${BUN_BIN:-$(command -v bun)}"
NODE_BIN="${NODE_BIN:-$(command -v node)}"
LDD_BIN="${LDD_BIN:-$(command -v ldd || true)}"

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/tokscale-release-artifact-smoke.XXXXXX")"
cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

MATRIX_FILE="${TMP_ROOT}/release-matrix.tsv"
node --input-type=module >"${MATRIX_FILE}" <<'NODE'
const rows = [
  ["cli-darwin-x64", "cli-binary-x86_64-apple-darwin", "tokscale"],
  ["cli-darwin-arm64", "cli-binary-aarch64-apple-darwin", "tokscale"],
  ["cli-linux-x64-gnu", "cli-binary-x86_64-unknown-linux-gnu", "tokscale"],
  ["cli-linux-x64-musl", "cli-binary-x86_64-unknown-linux-musl", "tokscale"],
  ["cli-linux-arm64-gnu", "cli-binary-aarch64-unknown-linux-gnu", "tokscale"],
  ["cli-linux-arm64-musl", "cli-binary-aarch64-unknown-linux-musl", "tokscale"],
  ["cli-win32-x64-msvc", "cli-binary-x86_64-pc-windows-msvc", "tokscale.exe"],
  ["cli-win32-arm64-msvc", "cli-binary-aarch64-pc-windows-msvc", "tokscale.exe"],
  ["cli-android-arm64", "cli-binary-aarch64-linux-android", "tokscale"],
];
for (const row of rows) {
  console.log(row.join("\t"));
}
NODE

HOST_PACKAGE="$(node --input-type=module <<'NODE'
import { execSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";

function detectLibcKind() {
  if (process.platform !== "linux") {
    return null;
  }

  const override = process.env.TOKSCALE_LIBC?.trim().toLowerCase();
  if (override === "musl") return "musl";
  if (override === "gnu" || override === "glibc") return "gnu";

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

  if (report?.header?.release?.sourceUrl?.toLowerCase().includes("musl")) {
    return "musl";
  }

  try {
    const output = execSync("ldd --version", {
      encoding: "utf-8",
      stdio: ["ignore", "pipe", "pipe"],
    }).toLowerCase();
    if (output.includes("musl")) return "musl";
    if (output.includes("glibc") || output.includes("gnu")) return "gnu";
  } catch (error) {
    const combined = `${error?.stdout ?? ""}\n${error?.stderr ?? ""}`.toLowerCase();
    if (combined.includes("musl")) return "musl";
    if (combined.includes("glibc") || combined.includes("gnu")) return "gnu";
  }

  const loaderPresent = (prefix) => {
    for (const dir of ["/lib", "/lib64"]) {
      try {
        if (readdirSync(dir).some((entry) => entry.startsWith(prefix))) {
          return true;
        }
      } catch {}
    }
    return false;
  };
  const hasGnuLoader = loaderPresent("ld-linux-");
  const hasMuslLoader = loaderPresent("ld-musl-");
  if (hasGnuLoader !== hasMuslLoader) return hasMuslLoader ? "musl" : "gnu";
  if (hasGnuLoader && hasMuslLoader) {
    return existsSync("/etc/alpine-release") ? "musl" : "gnu";
  }

  return "gnu";
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
} else if (process.platform === "win32") {
  if (arch === "arm64") console.log("cli-win32-arm64-msvc");
  else if (arch === "x64") console.log("cli-win32-x64-msvc");
  else process.exit(1);
} else if (process.platform === "android") {
  if (arch === "arm64") console.log("cli-android-arm64");
  else process.exit(1);
} else {
  process.exit(1);
}
NODE
)"

[[ -n "${HOST_PACKAGE}" ]] || fail "Unsupported host platform for launcher execution smoke"
HOST_BINARY_NAME="$(awk -F '\t' -v package="${HOST_PACKAGE}" '$1 == package { print $3 }' "${MATRIX_FILE}")"
[[ -n "${HOST_BINARY_NAME}" ]] || fail "Missing binary mapping for host package: ${HOST_PACKAGE}"

PACKAGE_STAGE_ROOT="${TMP_ROOT}/packages"
TARBALL_ROOT="${TMP_ROOT}/tarballs"
INSTALL_DIR="${TMP_ROOT}/install"
NPM_CACHE="${TMP_ROOT}/npm-cache"
BUN_ONLY_DIR="${TMP_ROOT}/bun-only-path"
NODE_ONLY_DIR="${TMP_ROOT}/node-only-path"
mkdir -p "${PACKAGE_STAGE_ROOT}" "${TARBALL_ROOT}" "${INSTALL_DIR}" "${NPM_CACHE}" "${BUN_ONLY_DIR}" "${NODE_ONLY_DIR}"

ln -s "${BUN_BIN}" "${BUN_ONLY_DIR}/bun"
ln -s "${NODE_BIN}" "${NODE_ONLY_DIR}/node"
if [[ -n "${LDD_BIN}" ]]; then
  ln -s "${LDD_BIN}" "${BUN_ONLY_DIR}/ldd"
  ln -s "${LDD_BIN}" "${NODE_ONLY_DIR}/ldd"
fi

echo "Building @tokscale/cli JavaScript entrypoint..."
bun run --cwd packages/cli build >/dev/null

LOCAL_TARBALLS_FILE="${TMP_ROOT}/platform-tarballs.tsv"
: >"${LOCAL_TARBALLS_FILE}"
while IFS=$'\t' read -r package_dir artifact_name binary_name; do
  artifact_dir="${ARTIFACT_ROOT}/${artifact_name}"
  source_binary="${artifact_dir}/${binary_name}"
  [[ -f "${source_binary}" ]] || fail "Missing ${binary_name} in ${artifact_dir}"

  stage_dir="${PACKAGE_STAGE_ROOT}/${package_dir}"
  cp -R "packages/${package_dir}" "${stage_dir}"
  mkdir -p "${stage_dir}/bin"
  cp "${source_binary}" "${stage_dir}/bin/${binary_name}"
  if [[ "${binary_name}" == "tokscale" ]]; then
    chmod +x "${stage_dir}/bin/${binary_name}"
  fi

  if [[ -f "${artifact_dir}/libFoundationModels.dylib" ]]; then
    cp "${artifact_dir}/libFoundationModels.dylib" "${stage_dir}/bin/libFoundationModels.dylib"
  elif [[ "${package_dir}" == "cli-darwin-arm64" ]]; then
    fail "Missing libFoundationModels.dylib in ${artifact_dir}"
  fi

  package_name="$(node --input-type=module - "${stage_dir}/package.json" <<'NODE'
import fs from "node:fs";
const manifest = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
console.log(manifest.name);
NODE
)"
  tarball_name="$(cd "${stage_dir}" && NPM_CONFIG_CACHE="${NPM_CACHE}" npm pack --silent)"
  printf '%s\t%s\n' "${package_name}" "file:${stage_dir}/${tarball_name}" >>"${LOCAL_TARBALLS_FILE}"
  cp "${stage_dir}/${tarball_name}" "${TARBALL_ROOT}/"
  echo "Packed ${package_name} from ${artifact_name}"
done <"${MATRIX_FILE}"

CLI_STAGE="${PACKAGE_STAGE_ROOT}/cli"
cp -R packages/cli "${CLI_STAGE}"
node --input-type=module - "${CLI_STAGE}/package.json" "${LOCAL_TARBALLS_FILE}" <<'NODE'
import fs from "node:fs";

const [manifestPath, mappingPath] = process.argv.slice(2);
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
const lines = fs.readFileSync(mappingPath, "utf8");
const replacements = new Map(lines.trim().split("\n").filter(Boolean).map((line) => line.split("\t")));
const optionalDependencies = {};
for (const packageName of Object.keys(manifest.optionalDependencies ?? {}).sort()) {
  const replacement = replacements.get(packageName);
  if (!replacement) {
    throw new Error(`Missing local tarball for optional dependency ${packageName}`);
  }
  optionalDependencies[packageName] = replacement;
}
manifest.optionalDependencies = optionalDependencies;
fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
NODE
CLI_TGZ="$(cd "${CLI_STAGE}" && NPM_CONFIG_CACHE="${NPM_CACHE}" npm pack --silent)"

WRAPPER_STAGE="${PACKAGE_STAGE_ROOT}/tokscale"
cp -R packages/tokscale "${WRAPPER_STAGE}"
node --input-type=module - "${WRAPPER_STAGE}/package.json" "file:${CLI_STAGE}/${CLI_TGZ}" <<'NODE'
import fs from "node:fs";
const [manifestPath, cliSpec] = process.argv.slice(2);
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
manifest.dependencies = {
  ...manifest.dependencies,
  "@tokscale/cli": cliSpec,
};
fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
NODE
WRAPPER_TGZ="$(cd "${WRAPPER_STAGE}" && NPM_CONFIG_CACHE="${NPM_CACHE}" npm pack --silent)"

echo "Installing release wrapper tarball with Bun..."
(
  cd "${INSTALL_DIR}"
  env PATH="${BUN_ONLY_DIR}" bun add "${WRAPPER_STAGE}/${WRAPPER_TGZ}" >/dev/null
)

WRAPPER_BIN="${INSTALL_DIR}/node_modules/tokscale/bin.js"
CLI_BIN="${INSTALL_DIR}/node_modules/@tokscale/cli/bin.js"
HOST_BINARY="${INSTALL_DIR}/node_modules/@tokscale/${HOST_PACKAGE}/bin/${HOST_BINARY_NAME}"
[[ -f "${WRAPPER_BIN}" ]] || fail "Installed wrapper bin.js missing"
[[ -f "${CLI_BIN}" ]] || fail "Installed @tokscale/cli bin.js missing"
[[ -f "${HOST_BINARY}" ]] || fail "Installed host platform binary missing: ${HOST_BINARY}"

echo "Checking installed wrapper with Node-only PATH..."
VERSION_NODE="$(env PATH="${NODE_ONLY_DIR}" "${WRAPPER_BIN}" --version)"
[[ "${VERSION_NODE}" == tokscale* ]] || fail "Unexpected Node-only wrapper output: ${VERSION_NODE}"

echo "Checking installed launcher with Bun runtime..."
VERSION_BUN="$(env PATH="${BUN_ONLY_DIR}" bun "${INSTALL_DIR}/node_modules/.bin/tokscale" --version)"
[[ "${VERSION_BUN}" == tokscale* ]] || fail "Unexpected Bun launcher output: ${VERSION_BUN}"

echo "Release artifact package smoke tests passed."
