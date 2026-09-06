#!/usr/bin/env python3
"""Require every path-filtered Rust workflow to re-run when the toolchain pin changes.

`rust-toolchain.toml` selects the compiler for every bare `cargo` call in the
checkout: rustup ranks it above the `rustup default` that
`dtolnay/rust-toolchain` sets, so bumping the pin changes what CI builds with.
A workflow whose `paths:` filter omits the file would skip exactly the run
that should exercise the new compiler.
"""
import pathlib
import re
import sys


ROOT = pathlib.Path.cwd()
WORKFLOWS_DIR = ROOT / ".github/workflows"
TOOLCHAIN_FILE = "rust-toolchain.toml"
WORKFLOW_GLOBS = ("*.yml", "*.yaml")
RUST_MARKERS = re.compile(r"dtolnay/rust-toolchain|\bcargo\b")
# `on: push` or `on: [push, pull_request]`: event names alone, so no `paths:`
# can hide in them. Any other inline value (a flow mapping, an anchor) can.
INLINE_TRIGGER_WITHOUT_FILTERS = re.compile(r"[A-Za-z_]+|\[[^\[\]{}:]*\]")


def fail(message: str) -> None:
    print(f"ERROR: {message}", file=sys.stderr)
    raise SystemExit(1)


def strip_yaml_comment(value: str) -> str:
    quote: str | None = None
    for index, char in enumerate(value):
        if quote:
            if char == quote:
                quote = None
            continue
        if char in {'"', "'"}:
            quote = char
            continue
        if char == "#" and (index == 0 or value[index - 1].isspace()):
            return value[:index].rstrip()
    return value.rstrip()


def indent_of(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def unquote(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value


def trigger_block(label: str, lines: list[str]) -> tuple[int, int]:
    """Return the [start, end) line range of the top-level `on:` mapping.

    An inline `on:` value is either a bare event list, which cannot carry a
    filter and yields an empty range, or unsupported, which fails loudly
    rather than exempting the workflow.
    """
    start = None
    for index, line in enumerate(lines):
        match = re.match(r"""(?:on|"on"|'on'):\s*(.*)$""", line)
        if not match:
            continue
        inline = match.group(1).strip()
        if not inline:
            start = index + 1
            break
        if INLINE_TRIGGER_WITHOUT_FILTERS.fullmatch(inline):
            return 0, 0
        fail(f"{label}:{index + 1}: unsupported inline `on:` value: {inline}")
    if start is None:
        return 0, 0
    for index in range(start, len(lines)):
        if lines[index].strip() and indent_of(lines[index]) == 0:
            return start, index
    return start, len(lines)


def list_entries(lines: list[str], start: int, indent: int) -> list[str]:
    entries: list[str] = []
    for line in lines[start:]:
        if not line.strip():
            continue
        if indent_of(line) <= indent:
            break
        item = re.match(r"\s*-\s*(.+?)\s*$", line)
        if item:
            entries.append(unquote(item.group(1)))
    return entries


def path_filters(label: str, lines: list[str]) -> list[tuple[int, str, list[str]]]:
    """Return (line_number, key, entries) for each paths/paths-ignore filter under `on:`."""
    start, end = trigger_block(label, lines)
    anchors: dict[str, list[str]] = {}
    filters: list[tuple[int, str, list[str]]] = []
    for index in range(start, end):
        match = re.match(r"(\s*)(paths|paths-ignore):\s*(.*)$", lines[index])
        if not match:
            continue
        line_number = index + 1
        indent = len(match.group(1))
        key = match.group(2)
        rest = match.group(3).strip()

        if rest.startswith("*"):
            alias = rest[1:].strip()
            if alias not in anchors:
                fail(f"{label}:{line_number}: `{key}: *{alias}` has no earlier anchor")
            filters.append((line_number, key, anchors[alias]))
            continue

        anchor = None
        if rest.startswith("&"):
            anchor, _, rest = rest[1:].partition(" ")
            rest = rest.strip()
        if rest.startswith("[") and rest.endswith("]"):
            entries = [unquote(item) for item in rest[1:-1].split(",") if item.strip()]
        elif rest:
            fail(f"{label}:{line_number}: unsupported `{key}:` value: {rest}")
        else:
            entries = list_entries(lines, index + 1, indent)
        if anchor:
            anchors[anchor] = entries
        filters.append((line_number, key, entries))
    return filters


def workflow_problems(path: pathlib.Path) -> list[str] | None:
    """Return the filter problems for a Rust workflow, or None if it is not one."""
    label = path.relative_to(ROOT).as_posix()
    lines = [
        strip_yaml_comment(line)
        for line in path.read_text(encoding="utf-8").splitlines()
    ]
    if not any(RUST_MARKERS.search(line) for line in lines):
        return None

    problems: list[str] = []
    for line_number, key, entries in path_filters(label, lines):
        if key == "paths" and TOOLCHAIN_FILE not in entries:
            problems.append(f"{label}:{line_number}: `paths:` filter omits {TOOLCHAIN_FILE}")
        if key == "paths-ignore" and TOOLCHAIN_FILE in entries:
            problems.append(f"{label}:{line_number}: `paths-ignore:` filter lists {TOOLCHAIN_FILE}")
    return problems


def main() -> None:
    if not (ROOT / TOOLCHAIN_FILE).exists():
        fail(f"Missing {TOOLCHAIN_FILE}; the Rust workflows pin their compiler through it")
    if not WORKFLOWS_DIR.is_dir():
        fail(f"Missing workflows directory: {WORKFLOWS_DIR}")

    checked: list[str] = []
    problems: list[str] = []
    workflows = sorted(
        path for pattern in WORKFLOW_GLOBS for path in WORKFLOWS_DIR.glob(pattern)
    )
    for path in workflows:
        found = workflow_problems(path)
        if found is None:
            continue
        checked.append(path.name)
        problems.extend(found)

    if not checked:
        fail(f"No Rust workflows found under {WORKFLOWS_DIR}")
    if problems:
        for problem in problems:
            print(f"ERROR: {problem}", file=sys.stderr)
        fail(f"a {TOOLCHAIN_FILE}-only bump would skip the workflows above")
    print(
        f"Workflow toolchain paths OK: {len(checked)} Rust workflows checked "
        f"({', '.join(checked)})"
    )


if __name__ == "__main__":
    main()
