import { createCanvas, loadImage, GlobalFonts } from "@napi-rs/canvas";
import { Resvg } from "@resvg/resvg-js";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import pc from "picocolors";
import {
  parseLocalSourcesAsync,
  finalizeReportAndGraphAsync,
  type ParsedMessages,
} from "./native.js";
import { syncCursorCache, loadCursorCredentials } from "./cursor.js";
import { loadCredentials } from "./credentials.js";
import type { SourceType } from "./graph-types.js";

interface WrappedData {
  year: string;
  firstDay: string;
  totalDays: number;
  activeDays: number;
  totalTokens: number;
  totalCost: number;
  currentStreak: number;
  longestStreak: number;
  topModels: Array<{ name: string; cost: number; tokens: number }>;
  topClients: Array<{ name: string; cost: number; tokens: number }>;
  topAgents?: Array<{ name: string; cost: number; tokens: number; messages: number }>;
  contributions: Array<{ date: string; level: 0 | 1 | 2 | 3 | 4 }>;
  totalMessages: number;
}

export interface WrappedOptions {
  output?: string;
  year?: string;
  sources?: SourceType[];
  short?: boolean;
  includeAgents?: boolean;
  pinSisyphus?: boolean;
}

const SCALE = 2;
const IMAGE_WIDTH = 1200 * SCALE;
const IMAGE_HEIGHT = 1200 * SCALE;
const PADDING = 56 * SCALE;

const COLORS = {
  background: "#10121C",
  textPrimary: "#ffffff",
  textSecondary: "#888888",
  textMuted: "#555555",
  accent: "#00B2FF",
  grade0: "#141A25",
  grade1: "#00B2FF44",
  grade2: "#00B2FF88",
  grade3: "#00B2FFCC",
  grade4: "#00B2FF",
};

const SOURCE_DISPLAY_NAMES: Record<string, string> = {
  opencode: "OpenCode",
  claude: "Claude Code",
  codex: "Codex CLI",
  gemini: "Gemini CLI",
  cursor: "Cursor IDE",
  amp: "Amp",
};

const ASSETS_BASE_URL = "https://tokscale.ai/assets/logos";

const PINNED_AGENTS = ["Sisyphus", "Planner-Sisyphus"];

function normalizeAgentName(agent: string): string {
  const agentLower = agent.toLowerCase();

  if (agentLower.includes("plan")) {
    if (agentLower.includes("omo") || agentLower.includes("sisyphus")) {
      return "Planner-Sisyphus";
    }
    return agent;
  }

  if (agentLower === "omo" || agentLower === "sisyphus") {
    return "Sisyphus";
  }

  return agent;
}

const CLIENT_LOGO_URLS: Record<string, string> = {
  "OpenCode": `${ASSETS_BASE_URL}/opencode.png`,
  "Claude Code": `${ASSETS_BASE_URL}/claude.jpg`,
  "Codex CLI": `${ASSETS_BASE_URL}/openai.jpg`,
  "Gemini CLI": `${ASSETS_BASE_URL}/gemini.png`,
  "Cursor IDE": `${ASSETS_BASE_URL}/cursor.jpg`,
  "Amp": `${ASSETS_BASE_URL}/amp.png`,
};

const PROVIDER_LOGO_URLS: Record<string, string> = {
  "anthropic": `${ASSETS_BASE_URL}/claude.jpg`,
  "openai": `${ASSETS_BASE_URL}/openai.jpg`,
  "google": `${ASSETS_BASE_URL}/gemini.png`,
  "xai": `${ASSETS_BASE_URL}/grok.jpg`,
  "zai": `${ASSETS_BASE_URL}/zai.jpg`,
};

function getProviderFromModel(modelId: string): string | null {
  const lower = modelId.toLowerCase();
  if (lower.includes("claude") || lower.includes("opus") || lower.includes("sonnet") || lower.includes("haiku")) {
    return "anthropic";
  }
  if (lower.includes("gpt") || lower.includes("o1") || lower.includes("o3") || lower.includes("codex")) {
    return "openai";
  }
  if (lower.includes("gemini")) {
    return "google";
  }
  if (lower.includes("grok")) {
    return "xai";
  }
  if (lower.includes("glm") || lower.includes("pickle")) {
    return "zai";
  }
  return null;
}

const TOKSCALE_LOGO_SVG_URL = "https://tokscale.ai/tokscale-logo.svg";
const TOKSCALE_LOGO_PNG_SIZE = 400;

function getImageCacheDir(): string {
  return path.join(os.homedir(), ".cache", "tokscale", "images");
}

function getFontCacheDir(): string {
  return path.join(os.homedir(), ".cache", "tokscale", "fonts");
}

async function fetchAndCacheImage(url: string, filename: string): Promise<string> {
  const cacheDir = getImageCacheDir();
  if (!fs.existsSync(cacheDir)) {
    fs.mkdirSync(cacheDir, { recursive: true });
  }
  
  const cachedPath = path.join(cacheDir, filename);
  
  if (!fs.existsSync(cachedPath)) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Failed to fetch ${url}`);
    const buffer = await response.arrayBuffer();
    fs.writeFileSync(cachedPath, Buffer.from(buffer));
  }
  
  return cachedPath;
}

async function fetchSvgAndConvertToPng(svgUrl: string, filename: string, size: number): Promise<string> {
  const cacheDir = getImageCacheDir();
  if (!fs.existsSync(cacheDir)) {
    fs.mkdirSync(cacheDir, { recursive: true });
  }
  
  const cachedPath = path.join(cacheDir, filename);
  
  if (!fs.existsSync(cachedPath)) {
    const response = await fetch(svgUrl);
    if (!response.ok) throw new Error(`Failed to fetch ${svgUrl}`);
    const svgText = await response.text();
    
    const resvg = new Resvg(svgText, {
      fitTo: { mode: "width", value: size },
    });
    const pngData = resvg.render();
    fs.writeFileSync(cachedPath, pngData.asPng());
  }
  
  return cachedPath;
}

const FIGTREE_FONTS = [
  { weight: "400", file: "Figtree-Regular.ttf", url: "https://fonts.gstatic.com/s/figtree/v9/_Xmz-HUzqDCFdgfMsYiV_F7wfS-Bs_d_QF5e.ttf" },
  { weight: "700", file: "Figtree-Bold.ttf", url: "https://fonts.gstatic.com/s/figtree/v9/_Xmz-HUzqDCFdgfMsYiV_F7wfS-Bs_eYR15e.ttf" },
];

let fontsRegistered = false;

async function ensureFontsLoaded(): Promise<void> {
  if (fontsRegistered) return;
  
  const cacheDir = getFontCacheDir();
  if (!fs.existsSync(cacheDir)) {
    fs.mkdirSync(cacheDir, { recursive: true });
  }

  for (const font of FIGTREE_FONTS) {
    const fontPath = path.join(cacheDir, font.file);
    
    if (!fs.existsSync(fontPath)) {
      const response = await fetch(font.url);
      if (!response.ok) continue;
      const buffer = await response.arrayBuffer();
      fs.writeFileSync(fontPath, Buffer.from(buffer));
    }

    if (fs.existsSync(fontPath)) {
      GlobalFonts.registerFromPath(fontPath, "Figtree");
    }
  }

  fontsRegistered = true;
}

async function loadWrappedData(options: WrappedOptions): Promise<WrappedData> {
  const year = options.year || new Date().getFullYear().toString();
  const sources = options.sources || ["opencode", "claude", "codex", "gemini", "cursor", "amp", "droid"];
  const localSources = sources.filter(s => s !== "cursor") as ("opencode" | "claude" | "codex" | "gemini" | "amp" | "droid")[];
  const includeCursor = sources.includes("cursor");

  const since = `${year}-01-01`;
  const until = `${year}-12-31`;

  const phase1Results = await Promise.allSettled([
    includeCursor && loadCursorCredentials() ? syncCursorCache() : Promise.resolve({ synced: false, rows: 0 }),
    localSources.length > 0
      ? parseLocalSourcesAsync({ sources: localSources, since, until, year })
      : Promise.resolve({ messages: [], opencodeCount: 0, claudeCount: 0, codexCount: 0, geminiCount: 0, ampCount: 0, droidCount: 0, processingTimeMs: 0 } as ParsedMessages),
  ]);

  const cursorSync = phase1Results[0].status === "fulfilled" 
    ? phase1Results[0].value 
    : { synced: false, rows: 0 };
  const localMessages = phase1Results[1].status === "fulfilled" 
    ? phase1Results[1].value 
    : null;

  const emptyMessages: ParsedMessages = {
    messages: [],
    opencodeCount: 0,
    claudeCount: 0,
    codexCount: 0,
    geminiCount: 0,
    ampCount: 0,
    droidCount: 0,
    processingTimeMs: 0,
  };

  const { report, graph } = await finalizeReportAndGraphAsync({
    localMessages: localMessages || emptyMessages,
    includeCursor: includeCursor && cursorSync.synced,
    since,
    until,
    year,
  });

  const modelMap = new Map<string, { cost: number; tokens: number }>();
  for (const entry of report.entries) {
    const displayName = formatModelName(entry.model);
    const existing = modelMap.get(displayName) || { cost: 0, tokens: 0 };
    modelMap.set(displayName, {
      cost: existing.cost + entry.cost,
      tokens: existing.tokens + entry.input + entry.output + entry.cacheRead + entry.cacheWrite,
    });
  }
  const topModels = Array.from(modelMap.entries())
    .map(([name, data]) => ({ name, ...data }))
    .sort((a, b) => b.cost - a.cost)
    .slice(0, 3);

  const clientMap = new Map<string, { cost: number; tokens: number }>();
  for (const entry of report.entries) {
    const displayName = SOURCE_DISPLAY_NAMES[entry.source] || entry.source;
    const existing = clientMap.get(displayName) || { cost: 0, tokens: 0 };
    clientMap.set(displayName, {
      cost: existing.cost + entry.cost,
      tokens: existing.tokens + entry.input + entry.output + entry.cacheRead + entry.cacheWrite,
    });
  }
  const topClients = Array.from(clientMap.entries())
    .map(([name, data]) => ({ name, ...data }))
    .sort((a, b) => b.cost - a.cost)
    .slice(0, 3);

  let topAgents: Array<{ name: string; cost: number; tokens: number; messages: number }> | undefined;
  if (options.includeAgents !== false && localMessages) {
    const agentMap = new Map<string, { cost: number; tokens: number; messages: number }>();
    for (const msg of localMessages.messages) {
      if (msg.source === "opencode" && msg.agent) {
        const normalizedAgent = normalizeAgentName(msg.agent);
        const existing = agentMap.get(normalizedAgent) || { cost: 0, tokens: 0, messages: 0 };

        const msgTokens = msg.input + msg.output + msg.cacheRead + msg.cacheWrite + msg.reasoning;

        agentMap.set(normalizedAgent, {
          cost: existing.cost,
          tokens: existing.tokens + msgTokens,
          messages: existing.messages + 1,
        });
      }
    }

    let agentsList = Array.from(agentMap.entries())
      .map(([name, data]) => ({ name, ...data }));

    if (options.pinSisyphus !== false) {
      const pinned = agentsList.filter(a => PINNED_AGENTS.includes(a.name));
      const unpinned = agentsList.filter(a => !PINNED_AGENTS.includes(a.name));

      pinned.sort((a, b) => PINNED_AGENTS.indexOf(a.name) - PINNED_AGENTS.indexOf(b.name));
      unpinned.sort((a, b) => b.messages - a.messages);

      agentsList = [...pinned, ...unpinned.slice(0, 2)];
    } else {
      agentsList.sort((a, b) => b.messages - a.messages);
      agentsList = agentsList.slice(0, 3);
    }

    topAgents = agentsList.length > 0 ? agentsList : undefined;
  }

  const maxCost = Math.max(...graph.contributions.map(c => c.totals.cost), 1);
  const contributions = graph.contributions.map(c => ({
    date: c.date,
    level: calculateIntensity(c.totals.cost, maxCost),
  }));

  const sortedDates = contributions.map(c => c.date).filter(d => d.startsWith(year)).sort();
  const { currentStreak, longestStreak } = calculateStreaks(sortedDates);

  const firstDay = sortedDates.length > 0 ? sortedDates[0] : `${year}-01-01`;

  return {
    year,
    firstDay,
    totalDays: graph.summary.totalDays,
    activeDays: graph.summary.activeDays,
    totalTokens: graph.summary.totalTokens,
    totalCost: graph.summary.totalCost,
    currentStreak,
    longestStreak,
    topModels,
    topClients,
    topAgents,
    contributions,
    totalMessages: report.totalMessages,
  };
}

function calculateIntensity(cost: number, maxCost: number): 0 | 1 | 2 | 3 | 4 {
  if (cost === 0 || maxCost === 0) return 0;
  const ratio = cost / maxCost;
  if (ratio >= 0.75) return 4;
  if (ratio >= 0.5) return 3;
  if (ratio >= 0.25) return 2;
  return 1;
}

function calculateStreaks(sortedDates: string[]): { currentStreak: number; longestStreak: number } {
  if (sortedDates.length === 0) return { currentStreak: 0, longestStreak: 0 };

  const todayStr = new Date().toISOString().split("T")[0];
  let currentStreak = 0;
  let longestStreak = 0;
  let streak = 1;

  for (let i = sortedDates.length - 1; i >= 0; i--) {
    if (i === sortedDates.length - 1) {
      const daysDiff = dateDiffDays(sortedDates[i], todayStr);
      if (daysDiff <= 1) {
        currentStreak = 1;
      } else {
        break;
      }
    } else {
      const daysDiff = dateDiffDays(sortedDates[i], sortedDates[i + 1]);
      if (daysDiff === 1) {
        currentStreak++;
      } else {
        break;
      }
    }
  }

  for (let i = 1; i < sortedDates.length; i++) {
    const daysDiff = dateDiffDays(sortedDates[i - 1], sortedDates[i]);
    if (daysDiff === 1) {
      streak++;
    } else {
      longestStreak = Math.max(longestStreak, streak);
      streak = 1;
    }
  }
  longestStreak = Math.max(longestStreak, streak);

  return { currentStreak, longestStreak };
}

function dateDiffDays(date1: string, date2: string): number {
  const d1 = new Date(date1 + "T00:00:00Z");
  const d2 = new Date(date2 + "T00:00:00Z");
  return Math.abs(Math.round((d2.getTime() - d1.getTime()) / (1000 * 60 * 60 * 24)));
}

function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000_000) return `${(tokens / 1_000_000_000).toFixed(2)}B`;
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(2)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}K`;
  return tokens.toString();
}

function formatCost(cost: number): string {
  if (cost >= 1000) return `$${(cost / 1000).toFixed(2)}K`;
  return `$${cost.toFixed(2)}`;
}

const MODEL_DISPLAY_NAMES: Record<string, string> = {
  "claude-sonnet-4-20250514": "Claude Sonnet 4",
  "claude-3-5-sonnet-20241022": "Claude 3.5 Sonnet",
  "claude-3-5-sonnet-20240620": "Claude 3.5 Sonnet",
  "claude-3-opus-20240229": "Claude 3 Opus",
  "claude-3-haiku-20240307": "Claude 3 Haiku",
  "gpt-4o": "GPT-4o",
  "gpt-4o-mini": "GPT-4o Mini",
  "gpt-4-turbo": "GPT-4 Turbo",
  "o1": "o1",
  "o1-mini": "o1 Mini",
  "o1-preview": "o1 Preview",
  "o3-mini": "o3 Mini",
  "gemini-2.5-pro": "Gemini 2.5 Pro",
  "gemini-2.5-flash": "Gemini 2.5 Flash",
  "gemini-2.0-flash": "Gemini 2.0 Flash",
  "gemini-1.5-pro": "Gemini 1.5 Pro",
  "gemini-1.5-flash": "Gemini 1.5 Flash",
  "grok-3": "Grok 3",
  "grok-3-mini": "Grok 3 Mini",
};

function formatModelName(model: string): string {
  if (MODEL_DISPLAY_NAMES[model]) return MODEL_DISPLAY_NAMES[model];
  
  const suffixMatch = model.match(/[-_](high|medium|low)$/i);
  const suffix = suffixMatch ? ` ${suffixMatch[1].charAt(0).toUpperCase()}${suffixMatch[1].slice(1).toLowerCase()}` : "";
  
  const cleaned = model
    .replace(/-20\d{6,8}(-\d+)?$/, "")
    .replace(/-\d{8}$/, "")
    .replace(/:[-\w]+$/, "")
    .replace(/[-_](high|medium|low)$/i, "")
    .replace(/[-_]thinking$/i, "");

  if (/claude[-_]?opus[-_]?4[-_.]?5/i.test(cleaned)) return `Claude Opus 4.5${suffix}`;
  if (/claude[-_]?4[-_]?opus/i.test(cleaned)) return `Claude 4 Opus${suffix}`;
  if (/claude[-_]?opus[-_]?4/i.test(cleaned)) return `Claude Opus 4${suffix}`;
  if (/claude[-_]?sonnet[-_]?4[-_.]?5/i.test(cleaned)) return `Claude Sonnet 4.5${suffix}`;
  if (/claude[-_]?4[-_]?sonnet/i.test(cleaned)) return `Claude 4 Sonnet${suffix}`;
  if (/claude[-_]?sonnet[-_]?4/i.test(cleaned)) return `Claude Sonnet 4${suffix}`;
  if (/claude[-_]?haiku[-_]?4[-_.]?5/i.test(cleaned)) return `Claude Haiku 4.5${suffix}`;
  if (/claude[-_]?4[-_]?haiku/i.test(cleaned)) return `Claude 4 Haiku${suffix}`;
  if (/claude[-_]?haiku[-_]?4/i.test(cleaned)) return `Claude Haiku 4${suffix}`;
  if (/claude[-_]?3[-_.]?7[-_]?sonnet/i.test(cleaned)) return `Claude 3.7 Sonnet${suffix}`;
  if (/claude[-_]?3[-_.]?5[-_]?sonnet/i.test(cleaned)) return `Claude 3.5 Sonnet${suffix}`;
  if (/claude[-_]?3[-_.]?5[-_]?haiku/i.test(cleaned)) return `Claude 3.5 Haiku${suffix}`;
  if (/claude[-_]?3[-_]?opus/i.test(cleaned)) return `Claude 3 Opus${suffix}`;
  if (/claude[-_]?3[-_]?sonnet/i.test(cleaned)) return `Claude 3 Sonnet${suffix}`;
  if (/claude[-_]?3[-_]?haiku/i.test(cleaned)) return `Claude 3 Haiku${suffix}`;
  if (/gpt[-_]?5[-_.]?1/i.test(cleaned)) return `GPT-5.1${suffix}`;
  if (/gpt[-_]?5/i.test(cleaned)) return `GPT-5${suffix}`;
  if (/gpt[-_]?4[-_]?o[-_]?mini/i.test(cleaned)) return `GPT-4o Mini${suffix}`;
  if (/gpt[-_]?4[-_]?o/i.test(cleaned)) return `GPT-4o${suffix}`;
  if (/gpt[-_]?4[-_]?turbo/i.test(cleaned)) return `GPT-4 Turbo${suffix}`;
  if (/gpt[-_]?4/i.test(cleaned)) return `GPT-4${suffix}`;
  if (/^o1[-_]?mini/i.test(cleaned)) return `o1 Mini${suffix}`;
  if (/^o1[-_]?preview/i.test(cleaned)) return `o1 Preview${suffix}`;
  if (/^o3[-_]?mini/i.test(cleaned)) return `o3 Mini${suffix}`;
  if (/^o1$/i.test(cleaned)) return `o1${suffix}`;
  if (/^o3$/i.test(cleaned)) return `o3${suffix}`;
  if (/gemini[-_]?3[-_]?pro/i.test(cleaned)) return `Gemini 3 Pro${suffix}`;
  if (/gemini[-_]?3[-_]?flash/i.test(cleaned)) return `Gemini 3 Flash${suffix}`;
  if (/gemini[-_]?2[-_.]?5[-_]?pro/i.test(cleaned)) return `Gemini 2.5 Pro${suffix}`;
  if (/gemini[-_]?2[-_.]?5[-_]?flash/i.test(cleaned)) return `Gemini 2.5 Flash${suffix}`;
  if (/gemini[-_]?2[-_.]?0[-_]?flash/i.test(cleaned)) return `Gemini 2.0 Flash${suffix}`;
  if (/gemini[-_]?1[-_.]?5[-_]?pro/i.test(cleaned)) return `Gemini 1.5 Pro${suffix}`;
  if (/gemini[-_]?1[-_.]?5[-_]?flash/i.test(cleaned)) return `Gemini 1.5 Flash${suffix}`;
  if (/grok[-_]?3[-_]?mini/i.test(cleaned)) return `Grok Code 3 Mini${suffix}`;
  if (/grok[-_]?3/i.test(cleaned)) return `Grok Code 3${suffix}`;
  if (/grok/i.test(cleaned)) return `Grok Code${suffix}`;
  if (/deepseek[-_]?v3/i.test(cleaned)) return `DeepSeek V3${suffix}`;
  if (/deepseek[-_]?r1/i.test(cleaned)) return `DeepSeek R1${suffix}`;
  if (/deepseek/i.test(cleaned)) return `DeepSeek${suffix}`;

  const baseName = cleaned
    .replace(/^claude[-_]/i, "Claude ")
    .replace(/^gpt[-_]/i, "GPT-")
    .replace(/^gemini[-_]/i, "Gemini ")
    .replace(/^grok[-_]/i, "Grok Code ")
    .split(/[-_]/)
    .filter(Boolean)
    .map(word => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
    .join(" ")
    .trim();
  
  return `${baseName}${suffix}`;
}

function drawRoundedRect(
  ctx: ReturnType<ReturnType<typeof createCanvas>["getContext"]>,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number
) {
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.lineTo(x + width - radius, y);
  ctx.quadraticCurveTo(x + width, y, x + width, y + radius);
  ctx.lineTo(x + width, y + height - radius);
  ctx.quadraticCurveTo(x + width, y + height, x + width - radius, y + height);
  ctx.lineTo(x + radius, y + height);
  ctx.quadraticCurveTo(x, y + height, x, y + height - radius);
  ctx.lineTo(x, y + radius);
  ctx.quadraticCurveTo(x, y, x + radius, y);
  ctx.closePath();
}

function drawContributionGraph(
  ctx: ReturnType<ReturnType<typeof createCanvas>["getContext"]>,
  data: WrappedData,
  x: number,
  y: number,
  width: number,
  height: number
) {
  const year = parseInt(data.year);
  const startDate = new Date(year, 0, 1);
  const endDate = new Date(year, 11, 31);

  const contribMap = new Map(data.contributions.map(c => [c.date, c.level]));

  const DAYS_PER_ROW = 14;
  const totalDays = Math.ceil((endDate.getTime() - startDate.getTime()) / (1000 * 60 * 60 * 24)) + 1;
  const totalRows = Math.ceil(totalDays / DAYS_PER_ROW);
  
  const cellSize = Math.min(
    Math.floor(height / totalRows),
    Math.floor(width / DAYS_PER_ROW)
  );
  const dotRadius = (cellSize - 2 * SCALE) / 2;

  const graphWidth = DAYS_PER_ROW * cellSize;
  const graphHeight = totalRows * cellSize;
  const offsetX = x + (width - graphWidth) / 2;
  const offsetY = y;

  const gradeColors = [COLORS.grade0, COLORS.grade1, COLORS.grade2, COLORS.grade3, COLORS.grade4];

  const currentDate = new Date(startDate);
  let dayIndex = 0;

  while (currentDate <= endDate) {
    const dateStr = currentDate.toISOString().split("T")[0];
    const level = contribMap.get(dateStr) || 0;

    const col = dayIndex % DAYS_PER_ROW;
    const row = Math.floor(dayIndex / DAYS_PER_ROW);

    const centerX = offsetX + col * cellSize + cellSize / 2;
    const centerY = offsetY + row * cellSize + cellSize / 2;

    ctx.beginPath();
    ctx.arc(centerX, centerY, dotRadius, 0, Math.PI * 2);
    ctx.fillStyle = gradeColors[level];
    ctx.fill();

    currentDate.setDate(currentDate.getDate() + 1);
    dayIndex++;
  }
}

function drawStat(
  ctx: ReturnType<ReturnType<typeof createCanvas>["getContext"]>,
  x: number,
  y: number,
  label: string,
  value: string
) {
  ctx.fillStyle = COLORS.textSecondary;
  ctx.font = `${18 * SCALE}px Figtree, sans-serif`;
  ctx.fillText(label, x, y);
  
  ctx.fillStyle = COLORS.textPrimary;
  ctx.font = `bold ${36 * SCALE}px Figtree, sans-serif`;
  ctx.fillText(value, x, y + 48 * SCALE);
}

function formatDate(dateStr: string): string {
  const date = new Date(dateStr + "T00:00:00");
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
}

async function generateWrappedImage(data: WrappedData, options: { short?: boolean; includeAgents?: boolean; pinSisyphus?: boolean } = {}): Promise<Buffer> {
  await ensureFontsLoaded();
  
  const canvas = createCanvas(IMAGE_WIDTH, IMAGE_HEIGHT);
  const ctx = canvas.getContext("2d");

  ctx.fillStyle = COLORS.background;
  ctx.fillRect(0, 0, IMAGE_WIDTH, IMAGE_HEIGHT);

  const leftWidth = IMAGE_WIDTH * 0.45;
  const rightWidth = IMAGE_WIDTH * 0.55;
  const rightX = leftWidth;

  let yPos = PADDING + 24 * SCALE;

  const credentials = loadCredentials();
  const MAX_USERNAME_LENGTH = 30; // GitHub max is 39, but leave room for layout
  const displayUsername = credentials?.username
    ? credentials.username.length > MAX_USERNAME_LENGTH
      ? credentials.username.substring(0, MAX_USERNAME_LENGTH - 1) + '…'
      : credentials.username
    : null;
  const titleText = displayUsername
    ? `@${displayUsername}'s Wrapped ${data.year}`
    : `My Wrapped ${data.year}`;
  ctx.fillStyle = COLORS.textPrimary;
  ctx.font = `bold ${28 * SCALE}px Figtree, sans-serif`;
  ctx.fillText(titleText, PADDING, yPos);
  yPos += 60 * SCALE;

  ctx.fillStyle = COLORS.textSecondary;
  ctx.font = `${20 * SCALE}px Figtree, sans-serif`;
  ctx.fillText("Total Tokens", PADDING, yPos);
  yPos += 64 * SCALE;
  
  ctx.fillStyle = COLORS.grade4;
  ctx.font = `bold ${56 * SCALE}px Figtree, sans-serif`;
  const totalTokensDisplay = options.short 
    ? formatTokens(data.totalTokens)
    : data.totalTokens.toLocaleString();
  ctx.fillText(totalTokensDisplay, PADDING, yPos);
  yPos += 50 * SCALE + 40 * SCALE;

  const logoSize = 32 * SCALE;
  const logoRadius = 6 * SCALE;

  ctx.fillStyle = COLORS.textSecondary;
  ctx.font = `${20 * SCALE}px Figtree, sans-serif`;
  ctx.fillText("Top Models", PADDING, yPos);
  yPos += 48 * SCALE;

  for (let i = 0; i < data.topModels.length; i++) {
    const model = data.topModels[i];
    ctx.fillStyle = COLORS.textPrimary;
    ctx.font = `bold ${32 * SCALE}px Figtree, sans-serif`;
    ctx.fillText(`${i + 1}`, PADDING, yPos);

    const provider = getProviderFromModel(model.name);
    const providerLogoUrl = provider ? PROVIDER_LOGO_URLS[provider] : null;
    let textX = PADDING + 40 * SCALE;

    if (providerLogoUrl) {
      try {
        const filename = `provider-${provider}@2x.jpg`;
        const logoPath = await fetchAndCacheImage(providerLogoUrl, filename);
        const logo = await loadImage(logoPath);
        const logoY = yPos - logoSize + 6 * SCALE;
        const logoX = PADDING + 40 * SCALE;

        ctx.save();
        drawRoundedRect(ctx, logoX, logoY, logoSize, logoSize, logoRadius);
        ctx.clip();
        ctx.drawImage(logo, logoX, logoY, logoSize, logoSize);
        ctx.restore();

        drawRoundedRect(ctx, logoX, logoY, logoSize, logoSize, logoRadius);
        ctx.strokeStyle = "#141A25";
        ctx.lineWidth = 1 * SCALE;
        ctx.stroke();

        textX = logoX + logoSize + 12 * SCALE;
      } catch {
      }
    }

    ctx.fillStyle = COLORS.textPrimary;
    ctx.font = `${32 * SCALE}px Figtree, sans-serif`;
    ctx.fillText(model.name, textX, yPos);
    yPos += 50 * SCALE;
  }
  yPos += 40 * SCALE;

  if (options.includeAgents !== false) {
    ctx.fillStyle = COLORS.textSecondary;
    ctx.font = `${20 * SCALE}px Figtree, sans-serif`;
    ctx.fillText("Top OpenCode Agents", PADDING, yPos);
    yPos += 48 * SCALE;

    const agents = data.topAgents || [];
    const SISYPHUS_COLOR = "#00CED1";
    let rankIndex = 1;

    for (let i = 0; i < agents.length; i++) {
      const agent = agents[i];
      const isSisyphusAgent = PINNED_AGENTS.includes(agent.name);
      const showWithDash = options.pinSisyphus !== false && isSisyphusAgent;

      ctx.fillStyle = showWithDash ? SISYPHUS_COLOR : COLORS.textPrimary;
      ctx.font = `bold ${32 * SCALE}px Figtree, sans-serif`;
      const prefix = showWithDash ? "•" : `${rankIndex}`;
      ctx.fillText(prefix, PADDING, yPos);
      if (!showWithDash) rankIndex++;

      const nameX = PADDING + 40 * SCALE;
      ctx.font = `${32 * SCALE}px Figtree, sans-serif`;
      ctx.fillStyle = isSisyphusAgent ? SISYPHUS_COLOR : COLORS.textPrimary;
      ctx.fillText(agent.name, nameX, yPos);

      const nameWidth = ctx.measureText(agent.name).width;
      ctx.fillStyle = COLORS.textSecondary;
      ctx.fillText(` (${agent.messages.toLocaleString()})`, nameX + nameWidth, yPos);

      yPos += 50 * SCALE;
    }
  } else {
    ctx.fillStyle = COLORS.textSecondary;
    ctx.font = `${20 * SCALE}px Figtree, sans-serif`;
    ctx.fillText("Top Clients", PADDING, yPos);
    yPos += 48 * SCALE;

    for (let i = 0; i < data.topClients.length; i++) {
      const client = data.topClients[i];
      ctx.fillStyle = COLORS.textPrimary;
      ctx.font = `bold ${32 * SCALE}px Figtree, sans-serif`;
      ctx.fillText(`${i + 1}`, PADDING, yPos);

      const logoUrl = CLIENT_LOGO_URLS[client.name];
      if (logoUrl) {
        try {
          const filename = `client-${client.name.toLowerCase().replace(/\s+/g, "-")}@2x.png`;
          const logoPath = await fetchAndCacheImage(logoUrl, filename);
          const logo = await loadImage(logoPath);
          const logoY = yPos - logoSize + 6 * SCALE;

          const logoX = PADDING + 40 * SCALE;
          const logoRadius = 6 * SCALE;

          ctx.save();
          drawRoundedRect(ctx, logoX, logoY, logoSize, logoSize, logoRadius);
          ctx.clip();
          ctx.drawImage(logo, logoX, logoY, logoSize, logoSize);
          ctx.restore();

          drawRoundedRect(ctx, logoX, logoY, logoSize, logoSize, logoRadius);
          ctx.strokeStyle = "#141A25";
          ctx.lineWidth = 1 * SCALE;
          ctx.stroke();
        } catch {
        }
      }

      ctx.font = `${32 * SCALE}px Figtree, sans-serif`;
      ctx.fillText(client.name, PADDING + 40 * SCALE + logoSize + 12 * SCALE, yPos);
      yPos += 50 * SCALE;
    }
  }
  yPos += 40 * SCALE;

  const statsStartY = yPos;
  const statWidth = (leftWidth - PADDING * 2) / 2;

  drawStat(ctx, PADDING, statsStartY, "Messages", data.totalMessages.toLocaleString());
  drawStat(ctx, PADDING + statWidth, statsStartY, "Active Days", `${data.activeDays}`);

  drawStat(ctx, PADDING, statsStartY + 100 * SCALE, "Cost", formatCost(data.totalCost));
  drawStat(ctx, PADDING + statWidth, statsStartY + 100 * SCALE, "Streak", `${data.longestStreak}d`);

  const footerBottomY = IMAGE_HEIGHT - PADDING;
  const tokscaleLogoHeight = 72 * SCALE;
  
  drawContributionGraph(
    ctx,
    data,
    rightX,
    PADDING,
    rightWidth - PADDING,
    IMAGE_HEIGHT - PADDING * 2
  );

  try {
    const logoPath = await fetchSvgAndConvertToPng(TOKSCALE_LOGO_SVG_URL, "tokscale-logo@2x.png", TOKSCALE_LOGO_PNG_SIZE * SCALE);
    const tokscaleLogo = await loadImage(logoPath);
    const logoWidth = (tokscaleLogo.width / tokscaleLogo.height) * tokscaleLogoHeight;
    
    ctx.fillStyle = COLORS.textSecondary;
    ctx.font = `${18 * SCALE}px Figtree, sans-serif`;
    ctx.fillText("github.com/junhoyeo/tokscale", PADDING, footerBottomY);
    
    const logoY = footerBottomY - 18 * SCALE - 16 * SCALE - tokscaleLogoHeight;
    ctx.drawImage(tokscaleLogo, PADDING, logoY, logoWidth, tokscaleLogoHeight);
  } catch {
  }

  return canvas.toBuffer("image/png");
}

export async function generateWrapped(options: WrappedOptions): Promise<string> {
  const data = await loadWrappedData(options);

  const agentsRequested = options.includeAgents !== false;
  const hasAgentData = !!data.topAgents?.length;
  const opencodeEnabled = !options.sources || options.sources.includes("opencode");
  let effectiveIncludeAgents = agentsRequested && hasAgentData;

  if (agentsRequested && opencodeEnabled && !hasAgentData) {
    console.warn(pc.yellow(`\n  ⚠ No OpenCode agent data found for ${data.year}.`));
    console.warn(pc.gray("    Falling back to clients view."));
    console.warn(pc.gray("    Use --clients to always show clients view.\n"));
  }

  const imageBuffer = await generateWrappedImage(data, {
    short: options.short,
    includeAgents: effectiveIncludeAgents,
    pinSisyphus: options.pinSisyphus,
  });

  const outputPath = options.output || `tokscale-${data.year}-wrapped.png`;
  const absolutePath = path.resolve(outputPath);

  fs.writeFileSync(absolutePath, imageBuffer);

  return absolutePath;
}

export { type WrappedData };
