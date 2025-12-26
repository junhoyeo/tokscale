/**
 * Tokscale CLI Submit Command
 * Submits local token usage data to the social platform
 */

import pc from "picocolors";
import { loadCredentials, getApiBaseUrl } from "./credentials.js";
import { PricingFetcher } from "./pricing.js";
import {
  isNativeAvailable,
  generateGraphWithPricingAsync,
} from "./native.js";
import type { TokenContributionData } from "./graph-types.js";
import { formatCurrency } from "./table.js";

interface SubmitOptions {
  opencode?: boolean;
  claude?: boolean;
  codex?: boolean;
  gemini?: boolean;
  cursor?: boolean;
  amp?: boolean;
  since?: string;
  until?: string;
  year?: string;
  dryRun?: boolean;
  quiet?: boolean;
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

type SourceType = "opencode" | "claude" | "codex" | "gemini" | "cursor" | "amp";

/**
 * Submit command - sends usage data to the platform
 */
export async function submit(options: SubmitOptions = {}): Promise<void> {
  const { quiet } = options;

  const logIfNotQuiet = (...args: Parameters<typeof console.log>) => {
    if (!quiet) console.log(...args);
  };

  // Step 1: Check if logged in
  const credentials = loadCredentials();
  if (!credentials) {
    if (!quiet) {
      console.log(pc.yellow("\n  Not logged in."));
      console.log(pc.gray("  Run 'tokscale login' first.\n"));
    } else {
      console.error("Error: Not logged in. Run 'tokscale login' first.");
    }
    process.exit(1);
  }

  // Step 2: Log native module status (TS fallback available)
  if (!isNativeAvailable()) {
    logIfNotQuiet(pc.yellow("\n  Note: Using TypeScript fallback (native module not available)"));
    logIfNotQuiet(pc.gray("  Run 'bun run build:core' for faster processing.\n"));
  }

  logIfNotQuiet(pc.cyan("\n  Tokscale - Submit Usage Data\n"));

  // Step 3: Generate graph data
  logIfNotQuiet(pc.gray("  Scanning local session data..."));

  const fetcher = new PricingFetcher();
  await fetcher.fetchPricing();
  const pricingEntries = fetcher.toPricingEntries();

  // Determine sources
  const hasFilter = options.opencode || options.claude || options.codex || options.gemini || options.cursor || options.amp;
  let sources: SourceType[] | undefined;
  if (hasFilter) {
    sources = [];
    if (options.opencode) sources.push("opencode");
    if (options.claude) sources.push("claude");
    if (options.codex) sources.push("codex");
    if (options.gemini) sources.push("gemini");
    if (options.cursor) sources.push("cursor");
    if (options.amp) sources.push("amp");
  }

  let data: TokenContributionData;
  try {
    data = await generateGraphWithPricingAsync({
      sources,
      pricing: pricingEntries,
      since: options.since,
      until: options.until,
      year: options.year,
    });
  } catch (error) {
    console.error(pc.red(`\n  Error generating data: ${(error as Error).message}\n`));
    process.exit(1);
  }

  // Step 4: Show summary
  logIfNotQuiet(pc.white("  Data to submit:"));
  logIfNotQuiet(pc.gray(`    Date range: ${data.meta.dateRange.start} to ${data.meta.dateRange.end}`));
  logIfNotQuiet(pc.gray(`    Active days: ${data.summary.activeDays}`));
  logIfNotQuiet(pc.gray(`    Total tokens: ${data.summary.totalTokens.toLocaleString()}`));
  logIfNotQuiet(pc.gray(`    Total cost: ${formatCurrency(data.summary.totalCost)}`));
  logIfNotQuiet(pc.gray(`    Sources: ${data.summary.sources.join(", ")}`));
  logIfNotQuiet(pc.gray(`    Models: ${data.summary.models.length} models`));
  logIfNotQuiet();

  if (data.summary.totalTokens === 0) {
    logIfNotQuiet(pc.yellow("  No usage data found to submit.\n"));
    return;
  }

  // Step 5: Dry run check
  if (options.dryRun) {
    logIfNotQuiet(pc.yellow("  Dry run - not submitting data.\n"));
    return;
  }

  // Step 6: Submit to server
  logIfNotQuiet(pc.gray("  Submitting to server..."));

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
      logIfNotQuiet();
      process.exit(1);
    }

    // Success!
    logIfNotQuiet(pc.green("\n  Successfully submitted!"));
    logIfNotQuiet();
    logIfNotQuiet(pc.white("  Summary:"));
    logIfNotQuiet(pc.gray(`    Submission ID: ${result.submissionId}`));
    logIfNotQuiet(pc.gray(`    Total tokens: ${result.metrics?.totalTokens?.toLocaleString()}`));
    logIfNotQuiet(pc.gray(`    Total cost: ${formatCurrency(result.metrics?.totalCost || 0)}`));
    logIfNotQuiet(pc.gray(`    Active days: ${result.metrics?.activeDays}`));
    logIfNotQuiet();
    logIfNotQuiet(pc.cyan(`  View your profile: ${baseUrl}/u/${credentials.username}`));
    logIfNotQuiet();

    if (result.warnings && result.warnings.length > 0) {
      logIfNotQuiet(pc.yellow("  Warnings:"));
      for (const warning of result.warnings) {
        logIfNotQuiet(pc.gray(`    - ${warning}`));
      }
      logIfNotQuiet();
    }
  } catch (error) {
    console.error(pc.red(`\n  Error: Failed to connect to server.`));
    console.error(pc.gray(`  ${(error as Error).message}\n`));
    process.exit(1);
  }
}
