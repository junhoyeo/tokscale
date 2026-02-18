/**
 * Tokscale CLI Submit Command
 * Submits local token usage data to the social platform
 */

import pc from "picocolors";
import * as readline from "node:readline/promises";
import { stdin as input, stdout as output } from "node:process";
import { exec } from "node:child_process";
import { promisify } from "node:util";
import { loadCredentials, getApiBaseUrl, loadStarCache, saveStarCache } from "./credentials.js";
import { parseLocalSourcesAsync, finalizeReportAndGraphAsync, type ParsedMessages } from "./native.js";
import { syncCursorCache, isCursorLoggedIn, hasCursorUsageCache } from "./cursor.js";
import type { TokenContributionData } from "./graph-types.js";
import { formatCurrency } from "./table.js";
import { parseDateStringToLocal, getStartOfDayTimestamp, getEndOfDayTimestamp, validateTimestampMs } from "./date-utils.js";

const execAsync = promisify(exec);

function getTimestampFilters(since?: string, until?: string): { sinceTs?: number; untilTs?: number } {
  let sinceTs: number | undefined;
  let untilTs: number | undefined;
  
  if (since) {
    const sinceDate = parseDateStringToLocal(since);
    if (sinceDate) {
      sinceTs = getStartOfDayTimestamp(sinceDate);
      sinceTs = validateTimestampMs(sinceTs, '--since');
    }
  }
  
  if (until) {
    const untilDate = parseDateStringToLocal(until);
    if (untilDate) {
      untilTs = getEndOfDayTimestamp(untilDate);
      untilTs = validateTimestampMs(untilTs, '--until');
    }
  }
  
  return { sinceTs, untilTs };
}

interface SubmitOptions {
  opencode?: boolean;
  claude?: boolean;
  codex?: boolean;
  gemini?: boolean;
  cursor?: boolean;
  amp?: boolean;
  droid?: boolean;
  openclaw?: boolean;
  pi?: boolean;
  since?: string;
  until?: string;
  year?: string;
  dryRun?: boolean;
}

interface SubmitResponse {
  success: boolean;
  submissionId?: string;
  username?: string;
  metrics?: {
    totalTokens: number;
    totalCost: number;
    dateRange: {
      start: string;
      end: string;
    };
    activeDays: number;
    sources: string[];
  };
  warnings?: string[];
  error?: string;
  details?: string[];
}

type SourceType = "opencode" | "claude" | "codex" | "gemini" | "cursor" | "amp" | "droid" | "openclaw" | "pi";

async function checkGhCliExists(): Promise<boolean> {
  try {
    await execAsync("gh --version");
    return true;
  } catch {
    return false;
  }
}

async function checkGitHubStarStatus(): Promise<boolean> {
  try {
    await execAsync("gh api /user/starred/junhoyeo/tokscale");
    return true;
  } catch (error: any) {
    if (error.code === 1 || error.stderr?.includes("404")) {
      return false;
    }
    throw error;
  }
}

async function attemptToStarRepo(): Promise<boolean> {
  try {
    await execAsync("gh api --silent --method PUT /user/starred/junhoyeo/tokscale >/dev/null 2>&1 || true");
    return true;
  } catch {
    return false;
  }
}

async function promptUserToStar(): Promise<'star' | 'decline'> {
  const rl = readline.createInterface({ input, output });
  
  return new Promise((resolve) => {
    const cleanup = () => {
      rl.close();
    };
    
    const handleSigint = () => {
      cleanup();
      resolve('decline');
    };
    
    process.once('SIGINT', handleSigint);
    
    rl.question(pc.white("  ⭐ Would you like to star tokscale? (Y/n): "))
      .then((answer) => {
        process.off('SIGINT', handleSigint);
        cleanup();
        const normalized = answer.trim().toLowerCase();
        resolve(normalized === 'n' ? 'decline' : 'star');
      })
      .catch(() => {
        process.off('SIGINT', handleSigint);
        cleanup();
        resolve('decline');
      });
  });
}

async function handleStarPrompt(username: string): Promise<void> {
  const starCache = loadStarCache(username);
  if (starCache?.hasStarred) {
    return;
  }

  const ghExists = await checkGhCliExists();

  if (ghExists) {
    try {
      const hasStarred = await checkGitHubStarStatus();
      if (hasStarred) {
        saveStarCache({
          username,
          hasStarred: true,
          checkedAt: new Date().toISOString(),
        });
        return;
      }
    } catch (error: any) {
      if (
        error.code === 'ENOTFOUND' || 
        error.code === 'ETIMEDOUT' || 
        error.stderr?.includes('404') || 
        error.stderr?.includes('Could not resolve')
      ) {
        return;
      }
      throw error;
    }
  }

  console.log();
  console.log(pc.cyan("  Help us grow! ⭐"));
  console.log(pc.gray("  Starring tokscale helps others discover the project.\n"));

  const userChoice = await promptUserToStar();

  if (userChoice === 'decline') {
    console.log();
    return;
  }

  if (!ghExists) {
    console.log();
    console.log(pc.yellow("  GitHub CLI (gh) not found."));
    console.log(pc.white("  Please star the repo manually:"));
    console.log(pc.cyan("  https://github.com/junhoyeo/tokscale\n"));
    return;
  }

  console.log(pc.gray("  Starring repository..."));
  const starred = await attemptToStarRepo();

  if (starred) {
    console.log(pc.green("  ✓ Starred! Thank you for your support.\n"));
    saveStarCache({
      username,
      hasStarred: true,
      checkedAt: new Date().toISOString(),
    });
  } else {
    console.log(pc.yellow("  Failed to star via gh CLI."));
    console.log(pc.gray("  Continuing to submit...\n"));
  }
}

export async function submit(options: SubmitOptions = {}): Promise<void> {
  const credentials = loadCredentials();
  if (!credentials) {
    console.log(pc.yellow("\n  Not logged in."));
    console.log(pc.gray("  Run 'tokscale login' first.\n"));
    process.exit(1);
  }

  await handleStarPrompt(credentials.username);

  console.log(pc.cyan("\n  Tokscale - Submit Usage Data\n"));

  console.log(pc.gray("  Scanning local session data..."));

  const hasFilter = options.opencode || options.claude || options.codex || options.gemini || options.cursor || options.amp || options.droid || options.openclaw || options.pi;
  let sources: SourceType[] | undefined;
  let includeCursor = true;
  if (hasFilter) {
    sources = [];
    if (options.opencode) sources.push("opencode");
    if (options.claude) sources.push("claude");
    if (options.codex) sources.push("codex");
    if (options.gemini) sources.push("gemini");
    if (options.cursor) sources.push("cursor");
    if (options.amp) sources.push("amp");
    if (options.droid) sources.push("droid");
    if (options.openclaw) sources.push("openclaw");
    if (options.pi) sources.push("pi");
    includeCursor = sources.includes("cursor");
  }

  // Filter out cursor from local sources (it's handled separately via sync)
  const localSources = sources?.filter((s): s is Exclude<SourceType, "cursor"> => s !== "cursor");
  const { sinceTs, untilTs } = getTimestampFilters(options.since, options.until);

  let data: TokenContributionData;
  try {
    // Two-phase processing (same as TUI) for consistency:
    // Phase 1: Parse local sources + sync cursor in parallel
    const [localMessages, cursorSync] = await Promise.all([
      parseLocalSourcesAsync({
        sources: localSources,
        since: options.since,
        until: options.until,
        year: options.year,
        sinceTs,
        untilTs,
      }),
      includeCursor && isCursorLoggedIn()
        ? syncCursorCache()
        : Promise.resolve({ synced: false, rows: 0, error: undefined }),
    ]);

    if (includeCursor && cursorSync.error && (cursorSync.synced || hasCursorUsageCache())) {
      const prefix = cursorSync.synced ? "Cursor sync warning" : "Cursor sync failed; using cached data";
      console.log(pc.yellow(`  ${prefix}: ${cursorSync.error}`));
    }

    // Phase 2: Finalize with pricing (combines local + cursor)
    // Single subprocess call ensures consistent pricing for both report and graph
    const { report, graph } = await finalizeReportAndGraphAsync({
      localMessages,
      includeCursor: includeCursor && (cursorSync.synced || hasCursorUsageCache()),
      since: options.since,
      until: options.until,
      year: options.year,
      sinceTs,
      untilTs,
    });

    // Use graph structure for submission, report's cost for display
    data = graph;
    data.summary.totalCost = report.totalCost;
  } catch (error) {
    console.error(pc.red(`\n  Error generating data: ${(error as Error).message}\n`));
    process.exit(1);
  }

  // Step 4: Show summary
  console.log(pc.white("  Data to submit:"));
  console.log(pc.gray(`    Date range: ${data.meta.dateRange.start} to ${data.meta.dateRange.end}`));
  console.log(pc.gray(`    Active days: ${data.summary.activeDays}`));
  console.log(pc.gray(`    Total tokens: ${data.summary.totalTokens.toLocaleString()}`));
  console.log(pc.gray(`    Total cost: ${formatCurrency(data.summary.totalCost)}`));
  console.log(pc.gray(`    Sources: ${data.summary.sources.join(", ")}`));
  console.log(pc.gray(`    Models: ${data.summary.models.length} models`));
  console.log();

  if (data.summary.totalTokens === 0) {
    console.log(pc.yellow("  No usage data found to submit.\n"));
    return;
  }

  // Step 5: Dry run check
  if (options.dryRun) {
    console.log(pc.yellow("  Dry run - not submitting data.\n"));
    return;
  }

  // Step 6: Submit to server
  console.log(pc.gray("  Submitting to server..."));

  const baseUrl = getApiBaseUrl();

  try {
    const response = await fetch(`${baseUrl}/api/submit`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${credentials.token}`,
      },
      body: JSON.stringify(data),
    });

    const result: SubmitResponse = await response.json();

    if (!response.ok) {
      console.error(pc.red(`\n  Error: ${result.error || "Submission failed"}`));
      if (result.details) {
        for (const detail of result.details) {
          console.error(pc.gray(`    - ${detail}`));
        }
      }
      console.log();
      process.exit(1);
    }

    // Success!
    console.log(pc.green("\n  Successfully submitted!"));
    console.log();
    console.log(pc.white("  Summary:"));
    console.log(pc.gray(`    Submission ID: ${result.submissionId}`));
    console.log(pc.gray(`    Total tokens: ${result.metrics?.totalTokens?.toLocaleString()}`));
    console.log(pc.gray(`    Total cost: ${formatCurrency(result.metrics?.totalCost || 0)}`));
    console.log(pc.gray(`    Active days: ${result.metrics?.activeDays}`));
    console.log();
    console.log(pc.cyan(`  View your profile: ${baseUrl}/u/${credentials.username}`));
    console.log();

    if (result.warnings && result.warnings.length > 0) {
      console.log(pc.yellow("  Warnings:"));
      for (const warning of result.warnings) {
        console.log(pc.gray(`    - ${warning}`));
      }
      console.log();
    }
  } catch (error) {
    console.error(pc.red(`\n  Error: Failed to connect to server.`));
    console.error(pc.gray(`  ${(error as Error).message}\n`));
    process.exit(1);
  }
}
