#!/usr/bin/env node
import { spawnSync, execSync } from "node:child_process";
import { existsSync, readdirSync, realpathSync } from "node:fs";
import { resolve, join, basename } from "node:path";
import { fileURLToPath } from "node:url";

const binaryName = process.platform === "win32" ? "tokscale.exe" : "tokscale";

const currentDir = fileURLToPath(new URL(".", import.meta.url));
const dirName = basename(currentDir);
// In npm install: currentDir = .../node_modules/@tokscale/cli/dist/
//   cliDir = .../node_modules/@tokscale/cli/
//   scopeDir = .../node_modules/@tokscale/
// In monorepo dev (dist): currentDir = .../packages/cli/dist/
//   cliDir = .../packages/cli/
//   scopeDir = .../packages/
// In monorepo dev (src): currentDir = .../packages/cli/src/
//   cliDir = .../packages/cli/
//   scopeDir = .../packages/
const isSubDir = dirName === "dist" || dirName === "src";
const cliDir = isSubDir ? resolve(currentDir, "..") : currentDir;
const scopeDir = resolve(cliDir, "..");
const workspaceRoot = resolve(scopeDir, "..");

type LibcKind = "gnu" | "musl";

function detectLibcKind(): LibcKind {
  const override = process.env.TOKSCALE_LIBC?.trim().toLowerCase();
  if (override === "musl") return "musl";
  if (override === "gnu" || override === "glibc") return "gnu";

  const report = process.report?.getReport?.() as
    | {
        header?: {
          glibcVersionRuntime?: string;
          release?: { sourceUrl?: string };
        };
        sharedObjects?: string[];
      }
    | undefined;

  if (report?.header?.glibcVersionRuntime) {
    return "gnu";
  }

  if (
    Array.isArray(report?.sharedObjects) &&
    report.sharedObjects.some((obj) => obj.toLowerCase().includes("musl"))
  ) {
    return "musl";
  }

  // Bun reports neither glibcVersionRuntime nor sharedObjects, but its
  // release.sourceUrl names the build flavor (e.g. bun-linux-x64-musl-baseline.zip).
  if (report?.header?.release?.sourceUrl?.toLowerCase().includes("musl")) {
    return "musl";
  }

  // musl distros (Alpine, Void-musl, ...) ship the loader as /lib/ld-musl-<arch>.so.1.
  try {
    if (readdirSync("/lib").some((entry) => entry.startsWith("ld-musl-"))) {
      return "musl";
    }
  } catch {
    // /lib unreadable or missing; fall through to ldd probing.
  }

  try {
    const output = execSync("ldd --version", {
      encoding: "utf-8",
      stdio: ["ignore", "pipe", "pipe"],
    }).toLowerCase();
    return output.includes("musl") ? "musl" : "gnu";
  } catch (error) {
    // musl's ldd rejects --version: it prints "musl libc" to stderr and
    // exits non-zero, so the answer is in the error, not the output.
    const { stdout, stderr } = (error ?? {}) as { stdout?: unknown; stderr?: unknown };
    const combined = `${stdout ?? ""}\n${stderr ?? ""}`.toLowerCase();
    return combined.includes("musl") ? "musl" : "gnu";
  }
}

function resolveTargetPackageName(): string | null {
  const arch = process.arch;

  if (process.platform === "darwin") {
    if (arch === "arm64") return "cli-darwin-arm64";
    if (arch === "x64") return "cli-darwin-x64";
    return null;
  }

  if (process.platform === "linux") {
    const libc = detectLibcKind();
    if (arch === "arm64") {
      return libc === "musl" ? "cli-linux-arm64-musl" : "cli-linux-arm64-gnu";
    }
    if (arch === "x64") {
      return libc === "musl" ? "cli-linux-x64-musl" : "cli-linux-x64-gnu";
    }
    return null;
  }

  if (process.platform === "win32") {
    if (arch === "arm64") return "cli-win32-arm64-msvc";
    if (arch === "x64") return "cli-win32-x64-msvc";
    return null;
  }

  return null;
}

function resolveRustTargetTriple(): string | null {
  const arch = process.arch;

  if (process.platform === "darwin") {
    if (arch === "arm64") return "aarch64-apple-darwin";
    if (arch === "x64") return "x86_64-apple-darwin";
    return null;
  }

  if (process.platform === "linux") {
    const libc = detectLibcKind();
    if (arch === "arm64") {
      return libc === "musl"
        ? "aarch64-unknown-linux-musl"
        : "aarch64-unknown-linux-gnu";
    }
    if (arch === "x64") {
      return libc === "musl"
        ? "x86_64-unknown-linux-musl"
        : "x86_64-unknown-linux-gnu";
    }
    return null;
  }

  if (process.platform === "win32") {
    if (arch === "arm64") return "aarch64-pc-windows-msvc";
    if (arch === "x64") return "x86_64-pc-windows-msvc";
    return null;
  }

  return null;
}

const targetPackage = resolveTargetPackageName();
const searchPaths: string[] = [];

if (targetPackage) {
  searchPaths.push(
    // npm/bun install: sibling scoped package (node_modules/@tokscale/cli-<platform>/bin/...)
    join(scopeDir, targetPackage, "bin", binaryName),
    // Nested node_modules: non-hoisted / pnpm (node_modules/@tokscale/cli/node_modules/@tokscale/cli-<platform>/bin/...)
    join(cliDir, "node_modules", "@tokscale", targetPackage, "bin", binaryName),
    // Hoisted edge case (node_modules/@tokscale/node_modules/@tokscale/cli-<platform>/bin/...)
    join(scopeDir, "node_modules", "@tokscale", targetPackage, "bin", binaryName),
    join(workspaceRoot, "node_modules", "@tokscale", targetPackage, "bin", binaryName),
    // Monorepo development
    join(workspaceRoot, "packages", targetPackage, "bin", binaryName),
  );
}

const rustTargetTriple = resolveRustTargetTriple();
if (rustTargetTriple) {
  searchPaths.push(join(workspaceRoot, "target", rustTargetTriple, "release", binaryName));
}

searchPaths.push(
  join(workspaceRoot, "target", "release", binaryName),
  join(cliDir, "bin", binaryName),
);

function tryRealpath(p: string): string {
  try {
    return realpathSync(p);
  } catch {
    return p;
  }
}

// Paths that would re-enter this wrapper if executed - using any of these as
// the "real" binary causes infinite recursion (a fork bomb). We compare by
// realpath so symlinks (e.g. npm/bun bin shims) are dereferenced.
const selfPaths = new Set<string>([
  tryRealpath(fileURLToPath(import.meta.url)),
  tryRealpath(join(cliDir, "bin.js")),
]);
if (process.argv[1]) {
  selfPaths.add(tryRealpath(process.argv[1]));
}

function isSelfReference(p: string): boolean {
  return selfPaths.has(tryRealpath(p));
}

let binary = searchPaths.find((p) => existsSync(p) && !isSelfReference(p));

if (!binary) {
  console.error("Error: tokscale binary not found");
  console.error("Build from source: cargo build --release -p tokscale-cli");
  if (targetPackage) {
    console.error(`Expected optional package: @tokscale/${targetPackage}`);
  }
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
process.exit(result.status ?? 1);
