#!/usr/bin/env python3
import json
import pathlib
import re
import sys


ROOT = pathlib.Path.cwd()
PUBLISH_WORKFLOW = ROOT / ".github/workflows/publish-cli.yml"
BUILD_NATIVE_WORKFLOW = ROOT / ".github/workflows/build-native.yml"
REQUIRED_ENV_KEYS = ("MACOSX_DEPLOYMENT_TARGET", "CARGO_TERM_COLOR", "CARGO_INCREMENTAL")
COMMON_BUILD_FIELDS = ("host", "target", "build", "strip", "bin_name")
TARGET_PACKAGES = {
    "x86_64-apple-darwin": "cli-darwin-x64",
    "aarch64-apple-darwin": "cli-darwin-arm64",
    "x86_64-unknown-linux-gnu": "cli-linux-x64-gnu",
    "x86_64-unknown-linux-musl": "cli-linux-x64-musl",
    "aarch64-unknown-linux-gnu": "cli-linux-arm64-gnu",
    "aarch64-unknown-linux-musl": "cli-linux-arm64-musl",
    "x86_64-pc-windows-msvc": "cli-win32-x64-msvc",
    "aarch64-pc-windows-msvc": "cli-win32-arm64-msvc",
}


def fail(message: str) -> None:
    print(f"ERROR: {message}", file=sys.stderr)
    raise SystemExit(1)


def read_lines(path: pathlib.Path) -> list[str]:
    if not path.exists():
        fail(f"Missing workflow: {path}")
    return path.read_text(encoding="utf-8").splitlines()


def strip_yaml_scalar(value: str) -> str:
    value = value.strip()
    if value in {'""', "''"}:
        return ""
    if (value.startswith('"') and value.endswith('"')) or (
        value.startswith("'") and value.endswith("'")
    ):
        return value[1:-1]
    return value


def top_level_env(lines: list[str]) -> dict[str, str]:
    env: dict[str, str] = {}
    for index, line in enumerate(lines):
        if line == "env:":
            for env_line in lines[index + 1 :]:
                if not env_line.startswith("  "):
                    break
                match = re.match(r"\s{2}([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$", env_line)
                if match:
                    env[match.group(1)] = strip_yaml_scalar(match.group(2))
            return env
    return env


def job_block(lines: list[str], job_name: str) -> list[str]:
    start = None
    for index, line in enumerate(lines):
        if re.match(rf"\s{{2}}{re.escape(job_name)}:\s*$", line):
            start = index + 1
            break
    if start is None:
        fail(f"Missing workflow job: {job_name}")

    end = len(lines)
    for index in range(start, len(lines)):
        if re.match(r"\s{2}[A-Za-z0-9_-]+:\s*$", lines[index]):
            end = index
            break
    return lines[start:end]


def matrix_settings(lines: list[str], job_name: str) -> list[dict[str, str]]:
    block = job_block(lines, job_name)
    settings_start = None
    settings_indent = 0
    for index, line in enumerate(block):
        match = re.match(r"(\s*)settings:\s*$", line)
        if match:
            settings_start = index + 1
            settings_indent = len(match.group(1))
            break
    if settings_start is None:
        fail(f"Missing matrix.settings for job: {job_name}")

    entries: list[dict[str, str]] = []
    current: dict[str, str] | None = None
    current_indent = 0
    for line in block[settings_start:]:
        if not line.strip():
            continue
        indent = len(line) - len(line.lstrip(" "))
        if indent <= settings_indent:
            break

        entry_match = re.match(r"(\s*)-\s+([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$", line)
        if entry_match:
            current = {entry_match.group(2): strip_yaml_scalar(entry_match.group(3))}
            current_indent = len(entry_match.group(1))
            entries.append(current)
            continue

        field_match = re.match(r"\s*([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$", line)
        if current is not None and field_match and indent > current_indent:
            current[field_match.group(1)] = strip_yaml_scalar(field_match.group(2))

    if not entries:
        fail(f"No matrix settings found for job: {job_name}")
    return entries


def by_target(entries: list[dict[str, str]], label: str) -> dict[str, dict[str, str]]:
    result: dict[str, dict[str, str]] = {}
    for entry in entries:
        target = entry.get("target")
        if not target:
            fail(f"{label} matrix entry is missing target: {entry}")
        if target in result:
            fail(f"{label} matrix target is duplicated: {target}")
        result[target] = entry
    return result


def package_manifest_name(package_dir: str) -> str:
    manifest_path = ROOT / "packages" / package_dir / "package.json"
    if not manifest_path.exists():
        fail(f"Missing platform package manifest: {manifest_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    name = manifest.get("name")
    if not isinstance(name, str) or not name:
        fail(f"{manifest_path} missing package name")
    return name


def block_contains(block: list[str], needle: str) -> bool:
    return any(needle in line for line in uncommented_lines(block))


def uncommented_lines(lines: list[str]) -> list[str]:
    return [line for line in lines if not line.lstrip().startswith("#")]


def parse_needs(block: list[str]) -> set[str]:
    lines = uncommented_lines(block)
    for index, line in enumerate(lines):
        match = re.match(r"(\s*)needs:\s*(.*)$", line)
        if not match:
            continue

        needs_indent = len(match.group(1))
        value = match.group(2).strip()
        if value:
            value = strip_yaml_scalar(value)
            if value.startswith("[") and value.endswith("]"):
                return {
                    strip_yaml_scalar(item)
                    for item in value[1:-1].split(",")
                    if strip_yaml_scalar(item)
                }
            return {value}

        needs: set[str] = set()
        for child in lines[index + 1 :]:
            if not child.strip():
                continue
            child_indent = len(child) - len(child.lstrip(" "))
            if child_indent <= needs_indent:
                break
            item_match = re.match(r"\s*-\s*(.+?)\s*$", child)
            if item_match:
                needs.add(strip_yaml_scalar(item_match.group(1)))
        return needs

    return set()


def main() -> None:
    publish_lines = read_lines(PUBLISH_WORKFLOW)
    native_lines = read_lines(BUILD_NATIVE_WORKFLOW)
    errors: list[str] = []

    publish_env = top_level_env(publish_lines)
    native_env = top_level_env(native_lines)
    for key in REQUIRED_ENV_KEYS:
        publish_has_key = key in publish_env
        native_has_key = key in native_env
        if not publish_has_key:
            errors.append(f"publish workflow missing required env {key}")
        if not native_has_key:
            errors.append(f"build-native workflow missing required env {key}")
        if (
            publish_has_key
            and native_has_key
            and publish_env.get(key) != native_env.get(key)
        ):
            errors.append(
                f"env {key} differs: publish={publish_env.get(key)!r}, build-native={native_env.get(key)!r}"
            )

    publish_build = by_target(matrix_settings(publish_lines, "build-cli-binary"), "publish build")
    native_build = by_target(matrix_settings(native_lines, "build"), "build-native")

    if list(publish_build) != list(native_build):
        errors.append(
            f"build matrix targets differ: publish={list(publish_build)}, build-native={list(native_build)}"
        )

    for target, publish_entry in publish_build.items():
        native_entry = native_build.get(target)
        if native_entry is None:
            continue
        for field in COMMON_BUILD_FIELDS:
            if publish_entry.get(field, "") != native_entry.get(field, ""):
                errors.append(
                    f"build matrix {target} field {field} differs: publish={publish_entry.get(field)!r}, build-native={native_entry.get(field)!r}"
                )

        expected_package_dir = TARGET_PACKAGES.get(target)
        if publish_entry.get("package_dir") != expected_package_dir:
            errors.append(
                f"build matrix {target} package_dir drift: expected {expected_package_dir}, found {publish_entry.get('package_dir')}"
            )
        expected_artifact = f"cli-binary-{target}"
        if publish_entry.get("artifact_name") != expected_artifact:
            errors.append(
                f"build matrix {target} artifact drift: expected {expected_artifact}, found {publish_entry.get('artifact_name')}"
            )

    publish_platform = matrix_settings(publish_lines, "publish-platform-packages")
    platform_by_dir: dict[str, dict[str, str]] = {}
    for entry in publish_platform:
        package_dir = entry.get("package_dir")
        if not package_dir:
            errors.append(f"publish platform entry missing package_dir: {entry}")
            continue
        if package_dir in platform_by_dir:
            errors.append(f"publish platform package_dir is duplicated: {package_dir}")
            continue
        platform_by_dir[package_dir] = entry

    expected_package_dirs = {
        entry["package_dir"] for entry in publish_build.values() if entry.get("package_dir")
    }
    if set(platform_by_dir) != expected_package_dirs:
        errors.append(
            f"publish platform package_dirs differ from build matrix: publish={sorted(platform_by_dir)}, build={sorted(expected_package_dirs)}"
        )

    build_by_package_dir = {
        entry["package_dir"]: entry for entry in publish_build.values() if entry.get("package_dir")
    }
    for package_dir, platform_entry in platform_by_dir.items():
        build_entry = build_by_package_dir.get(package_dir)
        if build_entry is None:
            continue
        expected_package_name = package_manifest_name(package_dir)
        if platform_entry.get("package_name") != expected_package_name:
            errors.append(
                f"publish platform package name drift for {package_dir}: expected {expected_package_name}, found {platform_entry.get('package_name')}"
            )
        if platform_entry.get("artifact_name") != build_entry.get("artifact_name"):
            errors.append(
                f"publish platform artifact drift for {package_dir}: expected {build_entry.get('artifact_name')}, found {platform_entry.get('artifact_name')}"
            )
        if platform_entry.get("binary_name") != build_entry.get("bin_name"):
            errors.append(
                f"publish platform binary drift for {package_dir}: expected {build_entry.get('bin_name')}, found {platform_entry.get('binary_name')}"
            )

    try:
        smoke_block = job_block(publish_lines, "smoke-release-artifacts")
    except SystemExit:
        smoke_block = []
        errors.append("publish workflow missing smoke-release-artifacts job")

    if smoke_block:
        if not block_contains(smoke_block, "pattern: cli-binary-*"):
            errors.append("smoke-release-artifacts job must download cli-binary-* artifacts")
        if not block_contains(smoke_block, "scripts/test-release-package-artifacts.sh"):
            errors.append("smoke-release-artifacts job must run scripts/test-release-package-artifacts.sh")

    prepare_block = job_block(publish_lines, "prepare-release-provenance")
    if "smoke-release-artifacts" not in parse_needs(prepare_block):
        errors.append("prepare-release-provenance must depend on smoke-release-artifacts")

    if errors:
        raise SystemExit("Release workflow safety check failed:\n- " + "\n- ".join(errors))

    print("Release workflow safety OK")


if __name__ == "__main__":
    main()
