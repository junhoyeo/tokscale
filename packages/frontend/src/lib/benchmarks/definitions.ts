/**
 * The tracked benchmarks: what each one measures, who publishes it, its
 * unit, license and attribution. Human-maintained; the sync script only
 * fills in results (src/content/benchmarks/*.json).
 *
 * No Tokscale composite score exists. Each benchmark keeps its own unit,
 * source and conditions.
 */
import type { SourceId } from "./schema";

export type BenchmarkCategory = "overall" | "agentic-coding" | "web-development" | "agents" | "reasoning" | "knowledge" | "history";

export const CATEGORY_ORDER: readonly BenchmarkCategory[] = [
  "overall",
  "agentic-coding",
  "web-development",
  "agents",
  "reasoning",
  "knowledge",
  "history",
];

export const CATEGORY_LABELS: Record<BenchmarkCategory, string> = {
  overall: "Overall",
  "agentic-coding": "Agentic coding",
  "web-development": "Web development",
  agents: "Agents",
  reasoning: "Reasoning",
  knowledge: "Knowledge",
  history: "History",
};

/**
 * Units. "percent" scores are stored as 0..1 fractions; "index" is the
 * Epoch Capabilities Index; "rating" is an arena rating (Bradley-Terry);
 * "effect" is Agent Arena's estimated net improvement.
 */
export type BenchmarkUnit = "percent" | "index" | "rating" | "effect";

export interface License {
  name: string;
  url: string | null;
}

export interface BenchmarkDefinition {
  slug: string;
  name: string;
  /** Column header on narrow tables. */
  shortName: string;
  category: BenchmarkCategory;
  /** Which snapshot carries the results; null for link-only benchmarks. */
  source: SourceId | null;
  /** Benchmark name or version inside that snapshot. */
  sourceName: string | null;
  /** Who builds and runs the benchmark. */
  publisher: string;
  /** Who collected the results, when it is not the publisher. */
  collectedBy: string | null;
  url: string;
  /** One or two plain sentences: what a high score means. */
  measures: string;
  unit: BenchmarkUnit;
  /** "% solved", "ECI", "Arena score". */
  unitLabel: string;
  /** How results are run: effort, agent or harness, trials. */
  conditions: string;
  license: License;
  /** Attribution line shown wherever the numbers appear. */
  attribution: string;
  methodNotes: readonly string[];
  /** "history": kept for context, not on the main board. "link-only": no results are republished. */
  status: "active" | "history" | "link-only";
  /** Fixed note for a benchmark we know is stale or replaced. */
  statusNote: string | null;
}

const CC_BY: License = { name: "CC BY 4.0", url: "https://creativecommons.org/licenses/by/4.0/" };

const EPOCH_CREDIT = "Epoch AI, \u2018Capabilities & benchmarking\u2019, epoch.ai/benchmarks (CC BY 4.0)";

export const BENCHMARKS: readonly BenchmarkDefinition[] = [
  {
    slug: "epoch-capabilities-index",
    name: "Epoch Capabilities Index",
    shortName: "ECI",
    category: "overall",
    source: "epoch",
    sourceName: "Epoch Capabilities Index",
    publisher: "Epoch AI",
    collectedBy: null,
    url: "https://epoch.ai/benchmarks/eci",
    measures:
      "Combines scores from many benchmarks into one general capability scale, so models can be compared even after single benchmarks saturate.",
    unit: "index",
    unitLabel: "ECI",
    conditions: "One score per model, with the confidence interval Epoch AI publishes. Epoch fits the index jointly across all results, so a model's score can move when new results are added.",
    license: CC_BY,
    attribution: EPOCH_CREDIT,
    methodNotes: [
      "This is Epoch AI's index, not a Tokscale score. Tokscale does not combine benchmarks.",
      "The date shown is the model's release date as listed by Epoch AI.",
    ],
    status: "active",
    statusNote: null,
  },
  {
    slug: "terminal-bench-4",
    name: "Terminal-Bench 4.0",
    shortName: "Terminal-Bench 4",
    category: "agentic-coding",
    source: "tbench",
    sourceName: "Terminal-Bench 4.0",
    publisher: "Terminal-Bench",
    collectedBy: null,
    url: "https://www.tbench.ai/leaderboard",
    measures: "Agents work on tasks inside a terminal. The score is the share of tasks resolved, with a 95% confidence interval.",
    unit: "percent",
    unitLabel: "% resolved",
    conditions: "Each row is one agent and model pair at one reasoning effort, as submitted to the official leaderboard. Cost and tokens are for the full run.",
    license: { name: "Terms not stated", url: null },
    attribution: "Terminal-Bench 4.0 official leaderboard, tbench.ai",
    methodNotes: [
      "Only result rows are stored. No task content is copied.",
      "The site's data terms are not confirmed. Every score links back to the leaderboard.",
      "The same model can appear with several agents. The board shows the best reported pair and names the agent.",
    ],
    status: "active",
    statusNote: null,
  },
  {
    slug: "swe-bench-pro-v2",
    name: "SWE-Bench Pro V2",
    shortName: "SWE-Bench Pro V2",
    category: "agentic-coding",
    source: null,
    sourceName: null,
    publisher: "Scale AI",
    collectedBy: null,
    url: "https://labs.scale.com/leaderboard/swe_bench_pro_public_v2",
    measures:
      "Long-horizon software engineering tasks in public open-source repositories. V2 is a refreshed public split of 642 tasks with a locked evaluation protocol (Scale AI, September 22, 2026).",
    unit: "percent",
    unitLabel: "% resolved",
    conditions: "See Scale AI's leaderboard for the protocol and results.",
    license: { name: "Leaderboard: all rights reserved (Scale AI)", url: null },
    attribution: "SWE-Bench Pro V2 leaderboard, Scale AI",
    methodNotes: ["Tokscale links to this leaderboard but does not copy its results until the terms are confirmed."],
    status: "link-only",
    statusNote: "Results not republished. Open the leaderboard on Scale AI's site.",
  },
  {
    slug: "cursorbench",
    name: "CursorBench",
    shortName: "CursorBench",
    category: "agentic-coding",
    source: "epoch",
    sourceName: "CursorBench",
    publisher: "Cursor",
    collectedBy: "Epoch AI",
    url: "https://cursor.com/cursorbench",
    measures: "Cursor evaluates agents on ambiguous, multi-file tasks taken from real Cursor sessions.",
    unit: "percent",
    unitLabel: "score",
    conditions: "Run by Cursor at each reasoning level. Cost, tokens and steps are averages per task.",
    license: CC_BY,
    attribution: `CursorBench results by Cursor, collected by ${EPOCH_CREDIT}`,
    methodNotes: ["Results are Epoch AI's copy of Cursor's published table and can lag the live page."],
    status: "active",
    statusNote: null,
  },
  {
    slug: "deepswe",
    name: "DeepSWE",
    shortName: "DeepSWE",
    category: "agentic-coding",
    source: "epoch",
    sourceName: "DeepSWE",
    publisher: "Datacurve",
    collectedBy: "Epoch AI",
    url: "https://deepswe.datacurve.ai/",
    measures: "Coding agents on original, long-horizon software engineering tasks. The score is pass@1.",
    unit: "percent",
    unitLabel: "pass@1",
    conditions: "One harness (mini-swe-agent) for every model, at each reasoning effort, averaged over several runs.",
    license: CC_BY,
    attribution: `DeepSWE results by Datacurve, collected by ${EPOCH_CREDIT}`,
    methodNotes: ["Results are Epoch AI's copy of the DeepSWE leaderboard and can lag the live page."],
    status: "active",
    statusNote: null,
  },
  {
    slug: "frontierswe",
    name: "FrontierSWE",
    shortName: "FrontierSWE",
    category: "agentic-coding",
    source: "epoch",
    sourceName: "FrontierSWE",
    publisher: "FrontierSWE",
    collectedBy: "Epoch AI",
    url: "https://www.frontierswe.com/",
    measures: "Very long software engineering tasks at the edge of human ability. The score is the mean of five trials per task.",
    unit: "percent",
    unitLabel: "mean@5",
    conditions: "One harness (proximus) for every model. Cost and duration are averages per task.",
    license: CC_BY,
    attribution: `FrontierSWE results, collected by ${EPOCH_CREDIT}`,
    methodNotes: ["Results are Epoch AI's copy of the FrontierSWE leaderboard and can lag the live page."],
    status: "active",
    statusNote: null,
  },
  {
    slug: "webdev-arena",
    name: "WebDev Arena",
    shortName: "WebDev Arena",
    category: "web-development",
    source: "lmarena",
    sourceName: "webdev",
    publisher: "LMArena",
    collectedBy: null,
    url: "https://arena.ai/leaderboard",
    measures:
      "Human preference for web development: people compare what two anonymous models build for the same prompt and vote. Scores are Bradley-Terry ratings with 95% confidence intervals.",
    unit: "rating",
    unitLabel: "Arena score",
    conditions: "The overall category of LMArena's webdev board (now called Code Arena). Reasoning effort is part of each entry's name.",
    license: CC_BY,
    attribution: "LMArena leaderboard dataset (lmarena-ai/leaderboard-dataset on Hugging Face), CC BY 4.0",
    methodNotes: ["Ratings from different arenas are on different scales and cannot be compared with each other."],
    status: "active",
    statusNote: null,
  },
  {
    slug: "agent-arena",
    name: "Agent Arena",
    shortName: "Agent Arena",
    category: "agents",
    source: "lmarena",
    sourceName: "agent",
    publisher: "LMArena",
    collectedBy: null,
    url: "https://arena.ai/blog/agent-arena-methodology/",
    measures:
      "Signals from real agent sessions on Arena (task success, user feedback, tool errors) turned into an estimated net improvement per orchestrator model, with 95% confidence intervals.",
    unit: "effect",
    unitLabel: "net improvement",
    conditions: "Arena's Agent Mode harness. Reasoning effort is part of each entry's name.",
    license: CC_BY,
    attribution: "LMArena leaderboard dataset (lmarena-ai/leaderboard-dataset on Hugging Face), CC BY 4.0",
    methodNotes: ["Scores are causal effect estimates, not win rates or ratings. Compare them only within this board."],
    status: "active",
    statusNote: null,
  },
  {
    slug: "gpqa-diamond",
    name: "GPQA Diamond",
    shortName: "GPQA Diamond",
    category: "reasoning",
    source: "epoch",
    sourceName: "GPQA diamond",
    publisher: "Epoch AI",
    collectedBy: null,
    url: "https://epoch.ai/benchmarks/gpqa-diamond",
    measures: "Hard multiple-choice questions in biology, chemistry and physics, written by PhD-level experts.",
    unit: "percent",
    unitLabel: "% correct",
    conditions: "Run by Epoch AI at each reasoning effort.",
    license: CC_BY,
    attribution: EPOCH_CREDIT,
    methodNotes: [],
    status: "active",
    statusNote: null,
  },
  {
    slug: "hle",
    name: "Humanity's Last Exam",
    shortName: "HLE",
    category: "reasoning",
    source: "epoch",
    sourceName: "HLE",
    publisher: "Center for AI Safety and Scale AI",
    collectedBy: "Epoch AI",
    url: "https://lastexam.ai/",
    measures: "2,500 expert-written questions across more than 100 academic subjects that need deep, specialised knowledge.",
    unit: "percent",
    unitLabel: "% correct",
    conditions: "Externally reported results collected by Epoch AI. The reasoning effort is often not reported.",
    license: CC_BY,
    attribution: `Humanity's Last Exam results, collected by ${EPOCH_CREDIT}`,
    methodNotes: [],
    status: "active",
    statusNote: null,
  },
  {
    slug: "frontiermath-tiers-1-3",
    name: "FrontierMath Tiers 1\u20133 (v2)",
    shortName: "FrontierMath",
    category: "reasoning",
    source: "epoch",
    sourceName: "FrontierMath-Tiers-1-3-v2-Private",
    publisher: "Epoch AI",
    collectedBy: null,
    url: "https://epoch.ai/benchmarks/frontiermath-tiers-1-3-v2",
    measures: "Expert-written math problems from advanced undergraduate level to early career research.",
    unit: "percent",
    unitLabel: "% solved",
    conditions: "Run by Epoch AI at each reasoning effort.",
    license: CC_BY,
    attribution: EPOCH_CREDIT,
    methodNotes: [],
    status: "active",
    statusNote: null,
  },
  {
    slug: "simpleqa-verified",
    name: "SimpleQA Verified",
    shortName: "SimpleQA Verified",
    category: "knowledge",
    source: "epoch",
    sourceName: "SimpleQA Verified",
    publisher: "Epoch AI",
    collectedBy: null,
    url: "https://epoch.ai/benchmarks/simpleqa-verified",
    measures: "1,000 short factual questions about politics, science and technology, art, sports, geography, music and more.",
    unit: "percent",
    unitLabel: "% correct",
    conditions: "Run by Epoch AI at each reasoning effort.",
    license: CC_BY,
    attribution: EPOCH_CREDIT,
    methodNotes: [],
    status: "active",
    statusNote: null,
  },
  {
    slug: "swe-bench-verified",
    name: "SWE-bench Verified",
    shortName: "SWE-bench Verified",
    category: "history",
    source: "epoch",
    sourceName: "SWE-Bench verified",
    publisher: "Epoch AI",
    collectedBy: null,
    url: "https://epoch.ai/benchmarks/swe-bench-verified",
    measures: "GitHub issues from real Python repositories: can the model write a valid fix?",
    unit: "percent",
    unitLabel: "% resolved",
    conditions: "Run by Epoch AI. Epoch upgraded its SWE-bench Verified scaffold in February 2026.",
    license: CC_BY,
    attribution: EPOCH_CREDIT,
    methodNotes: ["Kept as history: the official SWE-bench leaderboard's newest entry is from February 2026."],
    status: "history",
    statusNote: "History only. Newer agentic coding benchmarks are on the main board.",
  },
];

const BY_SLUG = new Map(BENCHMARKS.map((b) => [b.slug, b] as const));

export function getBenchmarkDefinition(slug: string): BenchmarkDefinition | null {
  return BY_SLUG.get(slug) ?? null;
}

/** Benchmarks whose results come from `source`. */
export function benchmarksForSource(source: SourceId): BenchmarkDefinition[] {
  return BENCHMARKS.filter((b) => b.source === source);
}
