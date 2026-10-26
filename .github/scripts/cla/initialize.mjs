import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

// Local only. Never creates/pushes a branch or overwrites a signature file.
export async function initializeStorage(path) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, JSON.stringify({ signedContributors: [] }, null, 2) + "\n", { flag: "wx" });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.argv.length !== 3) throw new Error("Usage: node initialize.mjs <new-json-path>");
    const path = resolve(process.argv[2]);
    await initializeStorage(path);
    console.log(`Created ${path}. No branch or remote state was changed.`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
