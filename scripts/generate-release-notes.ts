#!/usr/bin/env bun
/**
 * Usage: bun scripts/generate-release-notes.ts <version>
 * Env: GITHUB_REPOSITORY (default: junhoyeo/tokscale)
 */
export {};

import { execFileSync } from "node:child_process";

const REPO = process.env.GITHUB_REPOSITORY || "junhoyeo/tokscale";

interface Commit {
  hash: string;
  message: string;
  authorName: string;
  authorEmail: string;
}

interface PRInfo {
  number: number;
  title: string;
  authorLogin: string;
}

interface ChangeEntry {
  hash: string;
  message: string;
  author: string;
  prNumber?: number;
}

interface ContributorInfo {
  username: string;
  firstPrNumber: number;
}

function run(command: string, args: string[], allowFailure = false): string {
  try {
    return execFileSync(command, args, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  } catch (error) {
    if (allowFailure) return "";
    if (error instanceof Error) {
      throw new Error(`${command} ${args.join(" ")} failed: ${error.message}`);
    }
    throw error;
  }
}

function runJson<T>(command: string, args: string[], allowFailure = false): T | null {
  const output = run(command, args, allowFailure);
  if (!output) return null;
  try {
    return JSON.parse(output) as T;
  } catch {
    return null;
  }
}

function getPreviousTag(): string | null {
  const tag = run("git", ["describe", "--tags", "--abbrev=0", "HEAD^"], true);
  return tag || null;
}

function getTagDate(tag: string): string {
  return run("git", ["log", "-1", "--format=%cI", tag]);
}

function getCommitsBetween(fromTag: string, toRef: string): Commit[] {
  const output = run("git", [
    "log",
    `${fromTag}..${toRef}`,
    "--format=%H%x1f%s%x1f%an%x1f%ae",
    "--no-merges",
  ]);
  if (!output) return [];
  return output
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => {
      const [hash = "", message = "", authorName = "", authorEmail = ""] = line.split("\x1f");
      return { hash, message, authorName, authorEmail };
    })
    .filter((entry) => entry.hash && !entry.message.startsWith("chore: bump version"));
}

function resolveGitHubUsername(email: string, fallbackName: string): string {
  if (email.includes("@users.noreply.github.com")) {
    const match = email.match(/(?:\d+\+)?([^@]+)@users\.noreply\.github\.com/);
    if (match?.[1]) return `@${match[1]}`;
  }

  const search = runJson<{ items?: Array<{ login?: string }> }>(
    "gh",
    ["api", `/search/users?q=${encodeURIComponent(email)}+in:email`],
    true
  );
  const login = search?.items?.[0]?.login;
  return login ? `@${login}` : fallbackName;
}

function findAssociatedPR(commitHash: string): PRInfo | null {
  const result = runJson<Array<{ number: number; title: string; author?: { login?: string } }>>(
    "gh",
    [
      "pr",
      "list",
      "--repo",
      REPO,
      "--state",
      "merged",
      "--search",
      commitHash,
      "--json",
      "number,title,author",
      "--limit",
      "1",
    ],
    true
  );
  const pr = result?.[0];
  if (!pr?.number || !pr.author?.login) return null;
  return { number: pr.number, title: pr.title, authorLogin: pr.author.login };
}

function isFirstContributionAfter(login: string, thresholdDate: string): ContributorInfo | null {
  const result = runJson<Array<{ number: number; mergedAt: string }>>(
    "gh",
    [
      "pr",
      "list",
      "--repo",
      REPO,
      "--state",
      "merged",
      "--author",
      login,
      "--json",
      "number,mergedAt",
      "--limit",
      "200",
    ],
    true
  );
  if (!result?.length) return null;
  const oldest = [...result].sort(
    (a, b) => new Date(a.mergedAt).getTime() - new Date(b.mergedAt).getTime()
  )[0];
  return new Date(oldest.mergedAt) > new Date(thresholdDate)
    ? { username: `@${login}`, firstPrNumber: oldest.number }
    : null;
}

function generateReleaseNotes(version: string): string {
  const prevTag = getPreviousTag();
  if (!prevTag) {
    throw new Error("No previous tag found. Aborting release-note generation.");
  }

  const prevTagDate = getTagDate(prevTag);
  const commits = getCommitsBetween(prevTag, "HEAD");
  const entries: ChangeEntry[] = [];
  const candidateLogins = new Set<string>();

  const seenPRs = new Set<number>();

  for (const commit of commits) {
    const prInfo = findAssociatedPR(commit.hash);

    if (prInfo?.number && seenPRs.has(prInfo.number)) {
      // Skip duplicate commits from the same PR
      continue;
    }

    if (prInfo?.number) {
      seenPRs.add(prInfo.number);
    }

    const author = prInfo
      ? `@${prInfo.authorLogin}`
      : resolveGitHubUsername(commit.authorEmail, commit.authorName);

    entries.push({
      hash: commit.hash,
      message: prInfo?.title || commit.message,
      author,
      prNumber: prInfo?.number,
    });

    if (prInfo?.authorLogin) {
      candidateLogins.add(prInfo.authorLogin);
    }
  }

  const newContributors = Array.from(candidateLogins)
    .map((login) => isFirstContributionAfter(login, prevTagDate))
    .filter((item): item is ContributorInfo => Boolean(item));

  const lines: string[] = [
    '<div align="center">',
    "",
    `[![Tokscale](https://github.com/${REPO}/raw/main/.github/assets/hero.png)](https://github.com/${REPO})`,
    "",
    `# \`tokscale@v${version}\` is here!`,
    "</div>",
    "",
    "## What's Changed",
  ];

  if (entries.length === 0) {
    lines.push("* No notable changes");
  } else {
    for (const entry of entries.reverse()) {
      const prLink = entry.prNumber
        ? ` in https://github.com/${REPO}/pull/${entry.prNumber}`
        : "";
      const commitLink = entry.prNumber
        ? ""
        : ` (${entry.hash})`;
      lines.push(`* ${entry.message} by ${entry.author}${prLink}${commitLink}`);
    }
  }

  if (newContributors.length > 0) {
    lines.push("", "## New Contributors");
    for (const contributor of newContributors) {
      lines.push(
        `* ${contributor.username} made their first contribution in https://github.com/${REPO}/pull/${contributor.firstPrNumber}`
      );
    }
  }

  lines.push(
    "",
    `**Full Changelog**: https://github.com/${REPO}/compare/${prevTag}...v${version}`
  );

  return lines.join("\n");
}

function main(): void {
  const version = process.argv[2];
  if (!version) {
    console.error("Usage: bun scripts/generate-release-notes.ts <version>");
    process.exit(1);
  }
  const notes = generateReleaseNotes(version);
  console.log(notes);
}

main();
