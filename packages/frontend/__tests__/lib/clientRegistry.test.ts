import { existsSync, readFileSync, statSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  SOURCE_COLORS,
  SOURCE_DISPLAY_NAMES,
  SOURCE_LOGOS,
} from "../../src/lib/constants";
import { validateSubmission } from "../../src/lib/validation/submission";

const coreClientsPath = fileURLToPath(
  new URL("../../../../crates/tokscale-core/src/clients.rs", import.meta.url)
);
const githubAssetsPath = fileURLToPath(
  new URL("../../../../.github/assets/", import.meta.url)
);
const GITHUB_ASSET_URL_PREFIX =
  "https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/";

function coreClientIds(): string[] {
  const source = readFileSync(coreClientsPath, "utf8");
  const registry = source.match(/define_clients!\(([\s\S]*?)\n\);/);
  expect(registry).not.toBeNull();
  return Array.from(registry![1].matchAll(/\bid:\s*"([^"]+)"/g), (match) => match[1]);
}

function payloadForClient(client: string) {
  return {
    meta: {
      generatedAt: "2024-12-02T00:00:00.000Z",
      version: "2.1.1",
      dateRange: { start: "2024-12-01", end: "2024-12-01" },
    },
    summary: {
      totalTokens: 1500,
      totalCost: 1.5,
      totalDays: 1,
      activeDays: 1,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: [client],
      models: ["claude-sonnet-4"],
    },
    years: [
      {
        year: "2024",
        totalTokens: 1500,
        totalCost: 1.5,
        range: { start: "2024-12-01", end: "2024-12-01" },
      },
    ],
    contributions: [
      {
        date: "2024-12-01",
        totals: { tokens: 1500, cost: 1.5, messages: 5 },
        intensity: 2,
        tokenBreakdown: {
          input: 1000,
          output: 500,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        clients: [
          {
            client,
            modelId: "claude-sonnet-4",
            tokens: {
              input: 1000,
              output: 500,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 1.5,
            messages: 5,
          },
        ],
      },
    ],
  };
}

describe("frontend client registry", () => {
  it("accepts trae submissions", () => {
    const result = validateSubmission(payloadForClient("trae"));

    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it("accepts every core client id in submission validation", () => {
    const rejected = coreClientIds().filter((client) => {
      const result = validateSubmission(payloadForClient(client));
      return !result.valid;
    });

    expect(rejected).toEqual([]);
  });

  it("has labels, logos, and colors for every core client id", () => {
    const displayNames: Record<string, string> = SOURCE_DISPLAY_NAMES;
    const logos: Record<string, string> = SOURCE_LOGOS;
    const colors: Record<string, string> = SOURCE_COLORS;
    const missing = coreClientIds().flatMap((client) => {
      const fields = [];
      if (!displayNames[client]) fields.push(`${client}:display`);
      if (!logos[client]) fields.push(`${client}:logo`);
      if (!colors[client]) fields.push(`${client}:color`);
      return fields;
    });

    expect(missing).toEqual([]);
  });

  it("backs every GitHub CDN logo with a checked-in asset", () => {
    const logos: Record<string, string> = SOURCE_LOGOS;
    const missing = Object.entries(logos).flatMap(([client, logo]) => {
      if (!logo.startsWith(GITHUB_ASSET_URL_PREFIX)) return [];

      const asset = logo.slice(GITHUB_ASSET_URL_PREFIX.length);
      const assetPath = resolve(githubAssetsPath, asset);
      const pathFromAssets = relative(githubAssetsPath, assetPath);
      if (
        !asset ||
        pathFromAssets.startsWith("..") ||
        isAbsolute(pathFromAssets) ||
        !existsSync(assetPath) ||
        !statSync(assetPath).isFile()
      ) {
        return [`${client}:${asset}`];
      }
      return [];
    });

    expect(missing).toEqual([]);
  });
});
