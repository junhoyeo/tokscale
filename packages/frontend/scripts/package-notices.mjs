import { mkdir, readFile, realpath, stat, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep, win32 } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

// Keep repository-relative paths so links in the copied documents still work.
const noticeFiles = [
  "LICENSE",
  "CLA.md",
  "TRADEMARKS.md",
  "packages/frontend/LICENSE",
  "packages/frontend/LICENSING.md",
  "packages/frontend/src/content/LICENSE.md",
  "packages/frontend/src/content/third-party/models-dev/LICENSE",
  "packages/frontend/src/content/third-party/models-dev/README.md",
];

const index = `# Tokscale frontend notices

Start with the [frontend licensing scope](./packages/frontend/LICENSING.md).

- [Full GNU AGPL v3 license](./packages/frontend/LICENSE)
- [Content-data license and third-party terms](./packages/frontend/src/content/LICENSE.md)
- [models.dev MIT notice](./packages/frontend/src/content/third-party/models-dev/LICENSE) and [retrieval provenance](./packages/frontend/src/content/third-party/models-dev/README.md)
- [Inherited MIT copyright and permission notice](./LICENSE)
- [Trademark policy](./TRADEMARKS.md)
- [Contributor License Agreement](./CLA.md)

These files retain their repository-relative layout. This notice bundle is not a source-code distribution or a complete inventory of dependency and asset notices; keep those notices with their respective files too.
`;

export async function packageNotices(root = repositoryRoot) {
  const frontend = join(root, "packages/frontend");
  const standalone = join(frontend, ".next/standalone");
  const manifestPath = join(frontend, ".next/required-server-files.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  const appDir = manifest?.appDir;
  const relativeAppDir = manifest?.relativeAppDir;
  const tracingRoot = manifest?.config?.outputFileTracingRoot;
  if (
    manifest?.version !== 1 || manifest?.config?.output !== "standalone" ||
    typeof appDir !== "string" || !isAbsolute(appDir) ||
    typeof tracingRoot !== "string" || !isAbsolute(tracingRoot) ||
    typeof relativeAppDir !== "string" || isAbsolute(relativeAppDir) ||
    win32.parse(relativeAppDir).root !== "" || relativeAppDir.split(/[\\/]/).includes("..")
  ) {
    throw new Error(`Invalid standalone build manifest at ${manifestPath}`);
  }
  const actualFrontend = await realpath(frontend);
  if (
    await realpath(appDir) !== actualFrontend ||
    await realpath(resolve(tracingRoot, relativeAppDir)) !== actualFrontend
  ) {
    throw new Error(`Standalone build manifest does not describe ${frontend}`);
  }

  // Next can infer a workspace above the repository. Use its recorded layout,
  // not a directory scan or a fallback to some unrelated server.js. Docker
  // separately checks the fixed path required by its CMD.
  const server = join(standalone, relativeAppDir, "server.js");
  const actualServer = await realpath(server);
  const withinStandalone = relative(await realpath(standalone), actualServer);
  if (
    isAbsolute(withinStandalone) || withinStandalone === ".." ||
    withinStandalone.startsWith(`..${sep}`) || !(await stat(actualServer)).isFile()
  ) {
    throw new Error(`Expected a standalone Next.js server file inside ${standalone}`);
  }

  // Read all inputs before writing anything, and fail the build if any is absent.
  const documents = await Promise.all(
    noticeFiles.map(async (file) => [file, await readFile(join(root, file))]),
  );
  const destination = join(standalone, "notices");
  for (const [file, contents] of documents) {
    const target = join(destination, file);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, contents);
  }
  await writeFile(join(destination, "README.md"), index);
  return destination;
}

// Node resolves the module URL through symlinks (including macOS /var -> /private/var).
const entryPoint = process.argv[1] ? await realpath(process.argv[1]).catch(() => null) : null;
if (entryPoint === fileURLToPath(import.meta.url)) {
  await packageNotices();
  console.log("Packaged frontend notices in .next/standalone/notices");
}
