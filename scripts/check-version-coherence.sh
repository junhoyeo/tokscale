#!/usr/bin/env bash
set -euo pipefail

EXPECTED_VERSION="${1:-}"
if [[ "${EXPECTED_VERSION}" == "--expect-version" ]]; then
  if [[ -z "${2:-}" ]]; then
    echo "--expect-version requires a value" >&2
    exit 2
  fi
  EXPECTED_VERSION="${2}"
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

python3 - <<'PY' "${EXPECTED_VERSION}"
import json
import pathlib
import sys

expected_version = sys.argv[1] or None
root = pathlib.Path(".")

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None

if tomllib is None:
    raise SystemExit("Python tomllib is required (Python 3.11+)")

with (root / "Cargo.toml").open("rb") as cargo_file:
    cargo_data = tomllib.load(cargo_file)

workspace_section = cargo_data.get("workspace", {}).get("package", {})
workspace_version = workspace_section.get("version")
if not workspace_version:
    raise SystemExit("Could not find [workspace.package] version in Cargo.toml")
if expected_version and workspace_version != expected_version:
    raise SystemExit(
        f"Cargo workspace version mismatch: expected {expected_version}, found {workspace_version}"
    )

def load_json(path: str) -> dict:
    return json.loads((root / path).read_text())

cli_package = load_json("packages/cli/package.json")
wrapper_package = load_json("packages/tokscale/package.json")

platform_packages = sorted((root / "packages").glob("cli-*/package.json"))
if not platform_packages:
    raise SystemExit("No platform package manifests found under packages/cli-*")

errors: list[str] = []

def expect_equal(label: str, actual: str, expected: str) -> None:
    if actual != expected:
        errors.append(f"{label}: expected {expected}, found {actual}")

expect_equal("packages/cli/package.json version", cli_package["version"], workspace_version)
expect_equal("packages/tokscale/package.json version", wrapper_package["version"], workspace_version)
expect_equal(
    "packages/tokscale dependency on @tokscale/cli",
    wrapper_package["dependencies"]["@tokscale/cli"],
    workspace_version,
)

platform_names = set()
for path in platform_packages:
    manifest = json.loads(path.read_text())
    name = manifest.get("name")
    if not name:
        errors.append(f"{path} missing package name")
        continue
    platform_names.add(name)
    expect_equal(f"{path} version", manifest["version"], workspace_version)

expected_optional = {
    "@tokscale/cli-darwin-arm64",
    "@tokscale/cli-darwin-x64",
    "@tokscale/cli-linux-x64-gnu",
    "@tokscale/cli-linux-x64-musl",
    "@tokscale/cli-linux-arm64-gnu",
    "@tokscale/cli-linux-arm64-musl",
    "@tokscale/cli-win32-x64-msvc",
    "@tokscale/cli-win32-arm64-msvc",
}
actual_optional = set(cli_package["optionalDependencies"].keys())
if actual_optional != expected_optional:
    errors.append(
        "packages/cli optionalDependencies keys mismatch: "
        f"expected {sorted(expected_optional)}, found {sorted(actual_optional)}"
    )

for name, version in cli_package["optionalDependencies"].items():
    expect_equal(f"packages/cli optional dependency {name}", version, workspace_version)

missing_manifests = actual_optional - platform_names
extra_manifests = platform_names - actual_optional
if missing_manifests:
    errors.append(
        "Missing platform manifests for optional dependencies: "
        f"{sorted(missing_manifests)}"
    )
if extra_manifests:
    errors.append(
        "Platform manifests not listed in optionalDependencies: "
        f"{sorted(extra_manifests)}"
    )

if expected_version and cli_package["version"] != expected_version:
    errors.append(
        f"packages/cli/package.json version mismatch: expected {expected_version}, found {cli_package['version']}"
    )

if errors:
    raise SystemExit("Version coherence check failed:\n- " + "\n- ".join(errors))

print(f"Version coherence OK: {workspace_version}")
PY
