import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, posix, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { packageNotices } from "../../scripts/package-notices.mjs";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
// Independent artifact contract: changing only the script's manifest must not
// silently drop a license or a linked document from the distribution.
const expectedFiles = [
  "LICENSE",
  "CLA.md",
  "TRADEMARKS.md",
  "packages/frontend/LICENSE",
  "packages/frontend/LICENSING.md",
  "packages/frontend/src/content/LICENSE.md",
  "packages/frontend/src/content/third-party/models-dev/LICENSE",
  "packages/frontend/src/content/third-party/models-dev/README.md",
];
const modelsNotice = "packages/frontend/src/content/third-party/models-dev";
const modelsLicenseSha256 = "dc2dc41c9fea2fd3c41c21c6f484844cab367800b88ff24781f980ad3a9d160a";
const standaloneRelative = "packages/frontend/.next/standalone";
let fixtureRoot: string;
let standalone: string;

async function writeBuildManifest(overrides: { relativeAppDir?: string; appDir?: string; tracingRoot?: string } = {}) {
  await writeFile(join(fixtureRoot, "packages/frontend/.next/required-server-files.json"), JSON.stringify({
    version: 1,
    config: { output: "standalone", outputFileTracingRoot: overrides.tracingRoot ?? fixtureRoot },
    appDir: overrides.appDir ?? join(fixtureRoot, "packages/frontend"),
    relativeAppDir: overrides.relativeAppDir ?? "packages/frontend",
  }));
}

async function filesIn(directory: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      files.push(...(await filesIn(join(directory, entry.name))).map((file) => `${entry.name}/${file}`));
    } else {
      files.push(entry.name);
    }
  }
  return files.sort();
}

// Interpret the current Dockerfile's COPY graph, including WORKDIR and multi-
// source COPY. Reject unsupported syntax instead of treating it as verified.
async function dockerCopyGraph() {
  const text = await readFile(join(repositoryRoot, "Dockerfile"), "utf8");
  const copies: Array<{ stage: string; workdir: string; from?: string; sources: string[]; destination: string }> = [];
  let stage = "";
  let workdir = "/";
  for (const line of text.replace(/\\\r?\n\s*/g, " ").split("\n")) {
    const [instruction, ...args] = line.trim().split(/\s+/);
    if (instruction === "FROM") {
      stage = args.at(-1)!;
      workdir = "/";
    } else if (instruction === "WORKDIR") {
      workdir = posix.resolve(workdir, args[0]);
    } else if (instruction === "COPY") {
      expect(line).not.toMatch(/["[\]]/);
      const flags = args.filter((arg) => arg.startsWith("--"));
      expect(flags.every((flag) => /^--(from|chown)=/.test(flag))).toBe(true);
      const paths = args.filter((arg) => !arg.startsWith("--"));
      copies.push({ stage, workdir, from: flags.find((flag) => flag.startsWith("--from="))?.slice(7), sources: paths.slice(0, -1), destination: paths.at(-1)! });
    }
  }
  return { text, copies };
}

// Dockerignore matches a path or one of its parent directories. Its current
// patterns use *, **, and ?; fail if new syntax needs a more capable matcher.
function dockerIgnored(file: string, ignoreFile: string): boolean {
  const ancestors = file.split("/").map((_, index, parts) => parts.slice(0, index + 1).join("/"));
  let ignored = false;
  for (const raw of ignoreFile.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const negate = line.startsWith("!");
    const pattern = (negate ? line.slice(1) : line).replace(/^\/+|\/+$/g, "");
    expect(pattern).not.toMatch(/[\[\]\\]/);
    let regex = "";
    for (let i = 0; i < pattern.length; i++) {
      if (pattern.startsWith("**/", i)) { regex += "(?:.*/)?"; i += 2; }
      else if (pattern.startsWith("**", i)) { regex += ".*"; i++; }
      else if (pattern[i] === "*") regex += "[^/]*";
      else if (pattern[i] === "?") regex += "[^/]";
      else regex += pattern[i].replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    }
    if (ancestors.some((path) => new RegExp(`^${regex}$`).test(path))) ignored = !negate;
  }
  return ignored;
}

beforeEach(async () => {
  fixtureRoot = await mkdtemp(join(tmpdir(), "tokscale-notices-"));
  for (const file of expectedFiles) {
    const target = join(fixtureRoot, file);
    await mkdir(dirname(target), { recursive: true });
    await cp(join(repositoryRoot, file), target);
  }
  standalone = join(fixtureRoot, standaloneRelative);
  await mkdir(join(standalone, "packages/frontend"), { recursive: true });
  await writeFile(join(standalone, "packages/frontend/server.js"), "// Standalone server fixture\n");
  await writeBuildManifest();
});

afterEach(async () => {
  if (fixtureRoot) await rm(fixtureRoot, { recursive: true, force: true });
});

describe("frontend notice artifacts", () => {
  it("bundles complete, byte-identical notices at stable repository-relative paths", async () => {
    const notices = await packageNotices(fixtureRoot);
    expect(notices).toBe(join(standalone, "notices"));
    expect(await filesIn(notices)).toEqual([...expectedFiles, "README.md"].sort());
    for (const file of expectedFiles) {
      expect(await readFile(join(notices, file)), file).toEqual(await readFile(join(repositoryRoot, file)));
    }
    expect((await readFile(join(notices, "packages/frontend/LICENSE"), "utf8"))).toContain("GNU AFFERO GENERAL PUBLIC LICENSE");
    expect((await stat(join(notices, "packages/frontend/LICENSE"))).size).toBeGreaterThan(30_000);
    expect(await readFile(join(notices, "LICENSE"), "utf8")).toContain("Copyright (c) 2025 Junho Yeo");
  });

  it("resolves every relative Markdown link inside the distributed bundle", async () => {
    const notices = await packageNotices(fixtureRoot);
    let checked = 0;
    for (const file of (await filesIn(notices)).filter((name) => name.endsWith(".md"))) {
      const text = await readFile(join(notices, file), "utf8");
      for (const match of text.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
        const link = match[1];
        if (/^[a-z][a-z\d+.-]*:/i.test(link) || link.startsWith("#")) continue;
        const target = resolve(dirname(join(notices, file)), decodeURIComponent(link.split("#")[0]));
        expect(target === notices || target.startsWith(`${notices}${sep}`), `${file}: ${link}`).toBe(true);
        expect(await stat(target), `${file}: ${link}`).toBeDefined();
        checked++;
      }
    }
    expect(checked).toBeGreaterThan(10);
  });

  it("preserves the exact reviewed models.dev notice and identifies its retrieval, not an invented revision", async () => {
    const notices = await packageNotices(fixtureRoot);
    const license = await readFile(join(notices, modelsNotice, "LICENSE"));
    expect(createHash("sha256").update(license).digest("hex")).toBe(modelsLicenseSha256);
    expect(license.toString()).toContain("Copyright (c) 2025 models.dev");
    const provenance = await readFile(join(notices, modelsNotice, "README.md"), "utf8");
    expect(provenance).toContain(modelsLicenseSha256);
    expect(provenance).toContain("https://raw.githubusercontent.com/anomalyco/models.dev/dev/LICENSE");
    expect(provenance).toContain("Retrieval date: 2026-10-22");
    expect(provenance).toContain("commit or release version was not recorded");
    expect(provenance).toContain("upstream repository revision behind the API response was not recorded");
    expect(provenance).toContain("packages/frontend/src/content/benchmarks/models-dev.json");
  });

  it("fails instead of producing a notice-only artifact without a built server", async () => {
    await rm(standalone, { recursive: true });
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/server\.js/);
    await expect(stat(standalone)).rejects.toThrow(/ENOENT/);
  });

  it("follows a nested standalone layout when Next infers a workspace above the repository", async () => {
    const tracingRoot = dirname(fixtureRoot);
    const relativeAppDir = relative(tracingRoot, join(fixtureRoot, "packages/frontend"));
    await writeBuildManifest({ tracingRoot, relativeAppDir });
    await rm(join(standalone, "packages"), { recursive: true });
    await mkdir(join(standalone, relativeAppDir), { recursive: true });
    const server = join(standalone, relativeAppDir, "server.js");
    await writeFile(server, "// Nested standalone server fixture\n");
    const notices = await packageNotices(fixtureRoot);
    expect(notices).toBe(join(standalone, "notices"));
    expect(await filesIn(notices)).toEqual([...expectedFiles, "README.md"].sort());
    expect(await readFile(server, "utf8")).toBe("// Nested standalone server fixture\n");
  });

  it("requires the build manifest even if a file exists at Docker's conventional server path", async () => {
    await rm(join(fixtureRoot, "packages/frontend/.next/required-server-files.json"));
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/required-server-files\.json/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it("rejects malformed build metadata rather than falling back to a conventional server path", async () => {
    await writeFile(join(fixtureRoot, "packages/frontend/.next/required-server-files.json"), "{}");
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/Invalid standalone build manifest/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it.each(["../outside", "packages/../frontend", "/outside", "C:\\outside", "C:outside", "\\outside"])("rejects an unsafe manifest relativeAppDir: %s", async (relativeAppDir) => {
    await writeBuildManifest({ relativeAppDir });
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/Invalid standalone build manifest/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it("rejects a manifest that describes a different source app", async () => {
    await writeBuildManifest({ appDir: fixtureRoot });
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/does not describe/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it("rejects a tracing-root mapping that disagrees with the app directory", async () => {
    await writeBuildManifest({ relativeAppDir: "" });
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/does not describe/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it("requires the server at the path named by the manifest, not an unrelated conventional server", async () => {
    const tracingRoot = dirname(fixtureRoot);
    await writeBuildManifest({ tracingRoot, relativeAppDir: relative(tracingRoot, join(fixtureRoot, "packages/frontend")) });
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/server\.js/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it("rejects a server symlink that escapes the standalone artifact", async () => {
    const outsideServer = join(fixtureRoot, "outside-server.js");
    await writeFile(outsideServer, "// Not part of the standalone artifact\n");
    const server = join(standalone, "packages/frontend/server.js");
    await rm(server);
    await symlink(outsideServer, server);
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/server file inside/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it("fails before writing the bundle when a required source notice is missing", async () => {
    await rm(join(fixtureRoot, modelsNotice, "LICENSE"));
    await expect(packageNotices(fixtureRoot)).rejects.toThrow(/ENOENT/);
    await expect(stat(join(standalone, "notices"))).rejects.toThrow(/ENOENT/);
  });

  it.each(["node", "bun"])("runs the build-hook CLI under %s without relying on the caller's working directory", async (runtime) => {
    const script = join(fixtureRoot, "packages/frontend/scripts/package-notices.mjs");
    await mkdir(dirname(script), { recursive: true });
    await cp(join(repositoryRoot, "packages/frontend/scripts/package-notices.mjs"), script);
    const args = runtime === "bun" ? ["--no-env-file", script] : [script];
    const output = execFileSync(runtime, args, { cwd: tmpdir(), encoding: "utf8" });
    expect(output).toContain(".next/standalone/notices");
    expect(await filesIn(join(standalone, "notices"))).toEqual([...expectedFiles, "README.md"].sort());
  });

  it("can refresh the notices without changing the standalone server", async () => {
    const server = await readFile(join(standalone, "packages/frontend/server.js"));
    const notices = await packageNotices(fixtureRoot);
    await writeFile(join(notices, "LICENSE"), "stale notice");
    await packageNotices(fixtureRoot);
    expect(await readFile(join(notices, "LICENSE"))).toEqual(await readFile(join(repositoryRoot, "LICENSE")));
    expect(await readFile(join(standalone, "packages/frontend/server.js"))).toEqual(server);
  });

  it("keeps the notice inputs in the Docker context and copies them into the builder", async () => {
    const { copies } = await dockerCopyGraph();
    const ignoreFile = await readFile(join(repositoryRoot, ".dockerignore"), "utf8");
    expect(dockerIgnored("README.md", ignoreFile)).toBe(true);
    expect(dockerIgnored("node_modules/example/LICENSE", ignoreFile)).toBe(true);
    for (const file of expectedFiles) {
      expect(dockerIgnored(file, ignoreFile), file).toBe(false);
      const destinations = copies.filter((copy) => copy.stage === "builder" && !copy.from).flatMap((copy) => copy.sources.flatMap((source) => {
        if (file === source) {
          const basename = copy.destination.endsWith("/") || copy.sources.length > 1 ? posix.basename(source) : "";
          return [posix.resolve(copy.workdir, copy.destination, basename)];
        }
        return file.startsWith(`${source}/`) ? [posix.resolve(copy.workdir, copy.destination, posix.relative(source, file))] : [];
      }));
      expect(destinations, file).toContain(`/repo/${file}`);
    }
  });

  it("carries the same notice tree through the actual Docker runner COPY destination", async () => {
    const { text, copies } = await dockerCopyGraph();
    const manifest = JSON.parse(await readFile(join(repositoryRoot, "packages/frontend/package.json"), "utf8"));
    expect(manifest.scripts.build).toBe("next build && node scripts/package-notices.mjs");
    expect(await readFile(join(repositoryRoot, "packages/frontend/next.config.ts"), "utf8")).toMatch(/output:\s*"standalone"/);
    expect(text).toMatch(/WORKDIR \/repo\/packages\/frontend\b[\s\S]*?RUN bun run build\b/);
    expect(text).toContain('CMD ["node", "packages/frontend/server.js"]');
    expect(text.replace(/\\\r?\n\s*/g, " ").replace(/[ \t]+/g, " ")).toContain("RUN bun run build && test -f .next/standalone/packages/frontend/server.js");
    const copy = copies.find((copy) => copy.stage === "runner" && copy.from === "builder" && copy.sources.includes(`/repo/${standaloneRelative}`));
    expect(copy).toBeDefined();
    const imageDestination = posix.resolve(copy!.workdir, copy!.destination);
    expect(imageDestination).toBe("/app");
    await packageNotices(fixtureRoot);
    const imageRoot = join(fixtureRoot, "image");
    const target = join(imageRoot, posix.relative("/", imageDestination));
    await cp(join(fixtureRoot, relative("/repo", copy!.sources[0])), target, { recursive: true });
    for (const file of [...expectedFiles, "README.md"]) {
      expect(await readFile(join(target, "notices", file)), file).toEqual(await readFile(join(standalone, "notices", file)));
    }
    expect(await stat(join(target, "packages/frontend/server.js"))).toBeDefined();
  });
});
