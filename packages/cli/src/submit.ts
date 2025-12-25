/**
 * Tokscale CLI Submit Command
 * Submits local token usage data to the social platform
 */

import pc from "picocolors";
import { loadCredentials, getApiBaseUrl, type Credentials } from "./credentials.js";
import { PricingFetcher } from "./pricing.js";
import {
  isNativeAvailable,
  generateGraphWithPricingAsync,
} from "./native.js";
import type { TokenContributionData, DailyContribution, SourceContribution } from "./graph-types.js";
import { formatCurrency } from "./table.js";

interface SubmitOptions {
  opencode?: boolean;
  claude?: boolean;
  codex?: boolean;
  gemini?: boolean;
  cursor?: boolean;
  since?: string;
  until?: string;
  year?: string;
  dryRun?: boolean;
  full?: boolean;
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

type SourceType = "opencode" | "claude" | "codex" | "gemini" | "cursor";

type ChecksumResponse = Record<string, Record<string, string>>;

interface SourceBreakdownForHash {
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
  models: Record<string, {
    tokens: number;
    cost: number;
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    reasoning: number;
    messages: number;
  }>;
}

function hashSourceBreakdown(data: SourceBreakdownForHash): string {
  const sortedModels = Object.keys(data.models || {})
    .sort()
    .reduce((acc, key) => {
      const m = data.models[key];
      acc[key] = {
        tokens: m.tokens,
        cost: Math.round(m.cost * 10000),
        input: m.input,
        output: m.output,
        cacheRead: m.cacheRead,
        cacheWrite: m.cacheWrite,
        reasoning: m.reasoning || 0,
        messages: m.messages,
      };
      return acc;
    }, {} as Record<string, unknown>);

  const normalized = {
    tokens: data.tokens,
    cost: Math.round(data.cost * 10000),
    input: data.input,
    output: data.output,
    cacheRead: data.cacheRead,
    cacheWrite: data.cacheWrite,
    reasoning: data.reasoning || 0,
    messages: data.messages,
    models: sortedModels,
  };

  const content = JSON.stringify(normalized);

  let hash = 5381;
  for (let i = 0; i < content.length; i++) {
    hash = ((hash << 5) + hash) + content.charCodeAt(i);
    hash = hash & hash;
  }

  return Math.abs(hash).toString(16).padStart(8, "0");
}

function sourceContributionToBreakdown(sources: SourceContribution[]): Record<string, SourceBreakdownForHash> {
  const result: Record<string, SourceBreakdownForHash> = {};

  for (const source of sources) {
    const { input, output, cacheRead, cacheWrite, reasoning } = source.tokens;
    const totalTokens = input + output + cacheRead + cacheWrite + reasoning;

    if (!result[source.source]) {
      result[source.source] = {
        tokens: 0,
        cost: 0,
        input: 0,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 0,
        models: {},
      };
    }

    const r = result[source.source];
    r.tokens += totalTokens;
    r.cost += source.cost;
    r.input += input;
    r.output += output;
    r.cacheRead += cacheRead;
    r.cacheWrite += cacheWrite;
    r.reasoning += reasoning;
    r.messages += source.messages;

    if (!r.models[source.modelId]) {
      r.models[source.modelId] = {
        tokens: 0, cost: 0, input: 0, output: 0,
        cacheRead: 0, cacheWrite: 0, reasoning: 0, messages: 0,
      };
    }
    const m = r.models[source.modelId];
    m.tokens += totalTokens;
    m.cost += source.cost;
    m.input += input;
    m.output += output;
    m.cacheRead += cacheRead;
    m.cacheWrite += cacheWrite;
    m.reasoning += reasoning;
    m.messages += source.messages;
  }

  return result;
}

async function fetchServerChecksums(
  baseUrl: string,
  credentials: Credentials
): Promise<ChecksumResponse | null> {
  try {
    const response = await fetch(`${baseUrl}/api/submit/checksum`, {
      method: "GET",
      headers: {
        Authorization: `Bearer ${credentials.token}`,
      },
    });

    if (!response.ok) {
      return null;
    }

    const result = await response.json();
    return result.checksums || {};
  } catch {
    return null;
  }
}

function computeLocalChecksums(
  data: TokenContributionData
): Record<string, Record<string, string>> {
  const checksums: Record<string, Record<string, string>> = {};

  for (const day of data.contributions) {
    const sourceBreakdowns = sourceContributionToBreakdown(day.sources);
    checksums[day.date] = {};

    for (const [sourceName, breakdown] of Object.entries(sourceBreakdowns)) {
      checksums[day.date][sourceName] = hashSourceBreakdown(breakdown);
    }
  }

  return checksums;
}

function computeDiff(
  data: TokenContributionData,
  localChecksums: Record<string, Record<string, string>>,
  serverChecksums: ChecksumResponse
): TokenContributionData {
  const changedContributions: DailyContribution[] = [];

  for (const day of data.contributions) {
    const localDayChecksums = localChecksums[day.date] || {};
    const serverDayChecksums = serverChecksums[day.date] || {};

    let hasChanges = false;
    const changedSources: SourceContribution[] = [];

    for (const source of day.sources) {
      const localHash = localDayChecksums[source.source];
      const serverHash = serverDayChecksums[source.source];

      if (localHash !== serverHash) {
        hasChanges = true;
        changedSources.push(source);
      }
    }

    if (hasChanges) {
      changedContributions.push({
        ...day,
        sources: changedSources,
      });
    }
  }

  const changedSources = new Set<SourceType>();
  const changedModels = new Set<string>();
  let totalTokens = 0;
  let totalCost = 0;

  for (const day of changedContributions) {
    for (const source of day.sources) {
      changedSources.add(source.source);
      changedModels.add(source.modelId);
      const t = source.tokens;
      totalTokens += t.input + t.output + t.cacheRead + t.cacheWrite + t.reasoning;
      totalCost += source.cost;
    }
  }

  return {
    meta: data.meta,
    summary: {
      ...data.summary,
      totalTokens,
      totalCost,
      activeDays: changedContributions.length,
      sources: Array.from(changedSources),
      models: Array.from(changedModels),
    },
    years: data.years,
    contributions: changedContributions,
  };
}

export async function submit(options: SubmitOptions = {}): Promise<void> {
  const credentials = loadCredentials();
  if (!credentials) {
    console.log(pc.yellow("\n  Not logged in."));
    console.log(pc.gray("  Run 'tokscale login' first.\n"));
    process.exit(1);
  }

  if (!isNativeAvailable()) {
    console.log(pc.yellow("\n  Note: Using TypeScript fallback (native module not available)"));
    console.log(pc.gray("  Run 'bun run build:core' for faster processing.\n"));
  }

  console.log(pc.cyan("\n  Tokscale - Submit Usage Data\n"));

  console.log(pc.gray("  Scanning local session data..."));

  const fetcher = new PricingFetcher();
  await fetcher.fetchPricing();
  const pricingEntries = fetcher.toPricingEntries();

  const hasFilter = options.opencode || options.claude || options.codex || options.gemini || options.cursor;
  let sources: SourceType[] | undefined;
  if (hasFilter) {
    sources = [];
    if (options.opencode) sources.push("opencode");
    if (options.claude) sources.push("claude");
    if (options.codex) sources.push("codex");
    if (options.gemini) sources.push("gemini");
    if (options.cursor) sources.push("cursor");
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

  console.log(pc.white("  Local data scanned:"));
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

  if (options.dryRun) {
    console.log(pc.yellow("  Dry run - not submitting data.\n"));
    return;
  }

  const baseUrl = getApiBaseUrl();
  let dataToSubmit = data;
  let isDiffMode = false;

  if (!options.full) {
    console.log(pc.gray("  Fetching server checksums for diff..."));
    const serverChecksums = await fetchServerChecksums(baseUrl, credentials);

    if (serverChecksums && Object.keys(serverChecksums).length > 0) {
      const localChecksums = computeLocalChecksums(data);
      dataToSubmit = computeDiff(data, localChecksums, serverChecksums);
      isDiffMode = true;

      if (dataToSubmit.contributions.length === 0) {
        console.log(pc.green("\n  Already up to date! No changes to submit.\n"));
        console.log(pc.cyan(`  View your profile: ${baseUrl}/u/${credentials.username}\n`));
        return;
      }

      console.log(pc.white("  Changes detected:"));
      console.log(pc.gray(`    Days with changes: ${dataToSubmit.contributions.length}`));
      console.log(pc.gray(`    Tokens in diff: ${dataToSubmit.summary.totalTokens.toLocaleString()}`));
      console.log(pc.gray(`    Cost in diff: ${formatCurrency(dataToSubmit.summary.totalCost)}`));
      console.log();
    } else {
      console.log(pc.gray("  First submission or server unreachable, uploading full data..."));
    }
  }

  console.log(pc.gray(isDiffMode ? "  Submitting changes..." : "  Submitting to server..."));

  try {
    const response = await fetch(`${baseUrl}/api/submit`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${credentials.token}`,
      },
      body: JSON.stringify(dataToSubmit),
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

    console.log(pc.green("\n  Successfully submitted!"));
    console.log();
    console.log(pc.white("  Summary:"));
    console.log(pc.gray(`    Submission ID: ${result.submissionId}`));
    console.log(pc.gray(`    Total tokens: ${result.metrics?.totalTokens?.toLocaleString()}`));
    console.log(pc.gray(`    Total cost: ${formatCurrency(result.metrics?.totalCost || 0)}`));
    console.log(pc.gray(`    Active days: ${result.metrics?.activeDays}`));
    if (isDiffMode) {
      console.log(pc.gray(`    Mode: incremental (diff-based)`));
    }
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
