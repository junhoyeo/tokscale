#!/usr/bin/env npx tsx
/**
 * Synthetic Data Generator for Benchmarks
 * 
 * Generates realistic test data for all 4 session sources:
 * - OpenCode: Individual JSON files
 * - Claude Code: JSONL files
 * - Codex CLI: JSONL files with token_count events
 * - Gemini CLI: JSON session files
 * 
 * Usage: npx tsx benchmarks/generate.ts [--output <dir>] [--scale <multiplier>]
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { randomUUID } from "node:crypto";

// =============================================================================
// Configuration
// =============================================================================

interface GeneratorConfig {
  outputDir: string;
  scale: number; // Multiplier for data volume (1 = default, 2 = double, etc.)
  
  // Data volume (will be multiplied by scale)
  opencode: {
    sessions: number;
    messagesPerSession: number;
  };
  claude: {
    projects: number;
    filesPerProject: number;
    entriesPerFile: number;
  };
  codex: {
    sessions: number;
    eventsPerSession: number;
  };
  gemini: {
    projects: number;
    sessionsPerProject: number;
    messagesPerSession: number;
  };
  
  // Time range
  startDate: Date;
  endDate: Date;
}

const DEFAULT_CONFIG: GeneratorConfig = {
  outputDir: "./benchmarks/synthetic-data",
  scale: 1,
  
  opencode: {
    sessions: 50,
    messagesPerSession: 10,
  },
  claude: {
    projects: 10,
    filesPerProject: 5,
    entriesPerFile: 100,
  },
  codex: {
    sessions: 30,
    eventsPerSession: 80,
  },
  gemini: {
    projects: 5,
    sessionsPerProject: 4,
    messagesPerSession: 50,
  },
  
  startDate: new Date("2025-06-01"),
  endDate: new Date("2025-12-01"),
};

// =============================================================================
// Model definitions
// =============================================================================

const OPENCODE_MODELS = [
  { modelID: "claude-sonnet-4-20250514", providerID: "anthropic" },
  { modelID: "claude-3-5-sonnet-20241022", providerID: "anthropic" },
  { modelID: "gpt-4o", providerID: "openai" },
  { modelID: "gemini-2.0-flash", providerID: "google" },
];

const CLAUDE_MODELS = [
  "claude-sonnet-4-20250514",
  "claude-3-5-sonnet-20241022",
  "claude-3-opus-20240229",
];

const CODEX_MODELS = [
  "gpt-4o",
  "gpt-4o-mini",
  "o1-preview",
  "o1-mini",
];

const GEMINI_MODELS = [
  "gemini-2.0-flash-exp",
  "gemini-1.5-pro",
  "gemini-1.5-flash",
];

// =============================================================================
// Utility functions
// =============================================================================

function randomInt(min: number, max: number): number {
  return Math.floor(Math.random() * (max - min + 1)) + min;
}

function randomFloat(min: number, max: number): number {
  return Math.random() * (max - min) + min;
}

function randomChoice<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)];
}

function randomTimestamp(start: Date, end: Date): number {
  return start.getTime() + Math.random() * (end.getTime() - start.getTime());
}

function generateSessionId(): string {
  return `ses_${randomUUID().replace(/-/g, "").substring(0, 24)}`;
}

function generateMessageId(): string {
  return `msg_${randomUUID().replace(/-/g, "").substring(0, 24)}`;
}

function generateProjectHash(): string {
  return randomUUID().replace(/-/g, "").substring(0, 16);
}

function ensureDir(dir: string): void {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
}

// =============================================================================
// OpenCode Generator
// =============================================================================

interface OpenCodeMessage {
  id: string;
  sessionID: string;
  role: "assistant";
  modelID: string;
  providerID: string;
  cost: number;
  tokens: {
    input: number;
    output: number;
    reasoning: number;
    cache: {
      read: number;
      write: number;
    };
  };
  time: {
    created: number;
    completed: number;
  };
}

function generateOpenCodeData(config: GeneratorConfig): void {
  const baseDir = path.join(config.outputDir, ".local/share/opencode/storage/message");
  ensureDir(baseDir);
  
  const sessions = Math.ceil(config.opencode.sessions * config.scale);
  const messagesPerSession = config.opencode.messagesPerSession;
  
  let totalMessages = 0;
  
  for (let s = 0; s < sessions; s++) {
    const sessionId = generateSessionId();
    const sessionDir = path.join(baseDir, sessionId);
    ensureDir(sessionDir);
    
    const sessionStart = randomTimestamp(config.startDate, config.endDate);
    
    for (let m = 0; m < messagesPerSession; m++) {
      const model = randomChoice(OPENCODE_MODELS);
      const messageId = generateMessageId();
      
      const inputTokens = randomInt(1000, 30000);
      const outputTokens = randomInt(500, 15000);
      const reasoningTokens = Math.random() > 0.7 ? randomInt(100, 5000) : 0;
      const cacheRead = Math.random() > 0.5 ? randomInt(500, 10000) : 0;
      const cacheWrite = Math.random() > 0.8 ? randomInt(100, 2000) : 0;
      
      const created = sessionStart + m * randomInt(10000, 60000);
      const completed = created + randomInt(1000, 30000);
      
      const message: OpenCodeMessage = {
        id: messageId,
        sessionID: sessionId,
        role: "assistant",
        modelID: model.modelID,
        providerID: model.providerID,
        cost: randomFloat(0.001, 0.5),
        tokens: {
          input: inputTokens,
          output: outputTokens,
          reasoning: reasoningTokens,
          cache: {
            read: cacheRead,
            write: cacheWrite,
          },
        },
        time: {
          created,
          completed,
        },
      };
      
      const filePath = path.join(sessionDir, `${messageId}.json`);
      fs.writeFileSync(filePath, JSON.stringify(message, null, 2));
      totalMessages++;
    }
  }
  
  console.log(`  OpenCode: ${totalMessages} messages in ${sessions} sessions`);
}

// =============================================================================
// Claude Code Generator
// =============================================================================

interface ClaudeEntry {
  type: "user" | "assistant";
  timestamp: string;
  message?: {
    model: string;
    usage: {
      input_tokens: number;
      output_tokens: number;
      cache_read_input_tokens: number;
      cache_creation_input_tokens: number;
    };
  };
  content?: string;
}

function generateClaudeData(config: GeneratorConfig): void {
  const baseDir = path.join(config.outputDir, ".claude/projects");
  ensureDir(baseDir);
  
  const projects = Math.ceil(config.claude.projects * config.scale);
  const filesPerProject = config.claude.filesPerProject;
  const entriesPerFile = config.claude.entriesPerFile;
  
  let totalEntries = 0;
  
  for (let p = 0; p < projects; p++) {
    const projectName = `project-${generateProjectHash()}`;
    const projectDir = path.join(baseDir, projectName);
    ensureDir(projectDir);
    
    for (let f = 0; f < filesPerProject; f++) {
      const fileName = `session-${generateSessionId()}.jsonl`;
      const filePath = path.join(projectDir, fileName);
      
      const lines: string[] = [];
      const sessionStart = randomTimestamp(config.startDate, config.endDate);
      
      for (let e = 0; e < entriesPerFile; e++) {
        const isAssistant = e % 2 === 1; // Alternate user/assistant
        const model = randomChoice(CLAUDE_MODELS);
        const timestamp = new Date(sessionStart + e * randomInt(5000, 30000)).toISOString();
        
        if (isAssistant) {
          const entry: ClaudeEntry = {
            type: "assistant",
            timestamp,
            message: {
              model,
              usage: {
                input_tokens: randomInt(1000, 30000),
                output_tokens: randomInt(500, 15000),
                cache_read_input_tokens: Math.random() > 0.5 ? randomInt(500, 10000) : 0,
                cache_creation_input_tokens: Math.random() > 0.8 ? randomInt(100, 2000) : 0,
              },
            },
          };
          lines.push(JSON.stringify(entry));
          totalEntries++;
        } else {
          const entry: ClaudeEntry = {
            type: "user",
            timestamp,
            content: "User message content...",
          };
          lines.push(JSON.stringify(entry));
        }
      }
      
      fs.writeFileSync(filePath, lines.join("\n") + "\n");
    }
  }
  
  console.log(`  Claude: ${totalEntries} assistant entries in ${projects * filesPerProject} files`);
}

// =============================================================================
// Codex Generator
// =============================================================================

interface CodexEntry {
  type: "turn_context" | "event_msg";
  timestamp: string;
  payload: {
    type?: string;
    model?: string;
    info?: {
      last_token_usage?: {
        input_tokens: number;
        output_tokens: number;
        cached_input_tokens: number;
      };
      total_token_usage?: {
        input_tokens: number;
        output_tokens: number;
        cached_input_tokens: number;
      };
    };
  };
}

function generateCodexData(config: GeneratorConfig): void {
  const baseDir = path.join(config.outputDir, ".codex/sessions");
  ensureDir(baseDir);
  
  const sessions = Math.ceil(config.codex.sessions * config.scale);
  const eventsPerSession = config.codex.eventsPerSession;
  
  let totalEvents = 0;
  
  for (let s = 0; s < sessions; s++) {
    const sessionId = generateSessionId();
    const filePath = path.join(baseDir, `${sessionId}.jsonl`);
    
    const lines: string[] = [];
    const sessionStart = randomTimestamp(config.startDate, config.endDate);
    const model = randomChoice(CODEX_MODELS);
    
    // Add turn_context at start
    const turnContext: CodexEntry = {
      type: "turn_context",
      timestamp: new Date(sessionStart).toISOString(),
      payload: {
        model,
      },
    };
    lines.push(JSON.stringify(turnContext));
    
    let totalInput = 0;
    let totalOutput = 0;
    let totalCached = 0;
    
    for (let e = 0; e < eventsPerSession; e++) {
      const inputTokens = randomInt(500, 10000);
      const outputTokens = randomInt(200, 5000);
      const cachedTokens = Math.random() > 0.5 ? randomInt(200, 5000) : 0;
      
      totalInput += inputTokens;
      totalOutput += outputTokens;
      totalCached += cachedTokens;
      
      const timestamp = new Date(sessionStart + e * randomInt(5000, 30000)).toISOString();
      
      const tokenEvent: CodexEntry = {
        type: "event_msg",
        timestamp,
        payload: {
          type: "token_count",
          info: {
            last_token_usage: {
              input_tokens: inputTokens,
              output_tokens: outputTokens,
              cached_input_tokens: cachedTokens,
            },
            total_token_usage: {
              input_tokens: totalInput,
              output_tokens: totalOutput,
              cached_input_tokens: totalCached,
            },
          },
        },
      };
      lines.push(JSON.stringify(tokenEvent));
      totalEvents++;
    }
    
    fs.writeFileSync(filePath, lines.join("\n") + "\n");
  }
  
  console.log(`  Codex: ${totalEvents} token events in ${sessions} sessions`);
}

// =============================================================================
// Gemini Generator
// =============================================================================

interface GeminiMessage {
  id: string;
  timestamp: string;
  type: "user" | "gemini";
  content: string;
  tokens?: {
    input: number;
    output: number;
    cached: number;
    thoughts: number;
    tool: number;
    total: number;
  };
  model?: string;
}

interface GeminiSession {
  sessionId: string;
  projectHash: string;
  startTime: string;
  lastUpdated: string;
  messages: GeminiMessage[];
}

function generateGeminiData(config: GeneratorConfig): void {
  const baseDir = path.join(config.outputDir, ".gemini/tmp");
  ensureDir(baseDir);
  
  const projects = Math.ceil(config.gemini.projects * config.scale);
  const sessionsPerProject = config.gemini.sessionsPerProject;
  const messagesPerSession = config.gemini.messagesPerSession;
  
  let totalMessages = 0;
  
  for (let p = 0; p < projects; p++) {
    const projectHash = generateProjectHash();
    const chatsDir = path.join(baseDir, projectHash, "chats");
    ensureDir(chatsDir);
    
    for (let s = 0; s < sessionsPerProject; s++) {
      const sessionId = generateSessionId();
      const sessionStart = randomTimestamp(config.startDate, config.endDate);
      
      const messages: GeminiMessage[] = [];
      
      for (let m = 0; m < messagesPerSession; m++) {
        const isGemini = m % 2 === 1;
        const model = randomChoice(GEMINI_MODELS);
        const timestamp = new Date(sessionStart + m * randomInt(5000, 30000)).toISOString();
        
        if (isGemini) {
          const inputTokens = randomInt(1000, 30000);
          const outputTokens = randomInt(500, 15000);
          const cachedTokens = Math.random() > 0.5 ? randomInt(500, 10000) : 0;
          const thoughtTokens = Math.random() > 0.7 ? randomInt(100, 3000) : 0;
          const toolTokens = Math.random() > 0.9 ? randomInt(50, 500) : 0;
          
          messages.push({
            id: generateMessageId(),
            timestamp,
            type: "gemini",
            content: "Assistant response content...",
            model,
            tokens: {
              input: inputTokens,
              output: outputTokens,
              cached: cachedTokens,
              thoughts: thoughtTokens,
              tool: toolTokens,
              total: inputTokens + outputTokens + cachedTokens + thoughtTokens + toolTokens,
            },
          });
          totalMessages++;
        } else {
          messages.push({
            id: generateMessageId(),
            timestamp,
            type: "user",
            content: "User message content...",
          });
        }
      }
      
      const session: GeminiSession = {
        sessionId,
        projectHash,
        startTime: new Date(sessionStart).toISOString(),
        lastUpdated: messages[messages.length - 1]?.timestamp || new Date(sessionStart).toISOString(),
        messages,
      };
      
      const filePath = path.join(chatsDir, `session-${sessionId}.json`);
      fs.writeFileSync(filePath, JSON.stringify(session, null, 2));
    }
  }
  
  console.log(`  Gemini: ${totalMessages} gemini messages in ${projects * sessionsPerProject} sessions`);
}

// =============================================================================
// Main
// =============================================================================

function parseArgs(): Partial<GeneratorConfig> {
  const args = process.argv.slice(2);
  const config: Partial<GeneratorConfig> = {};
  
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--output" && args[i + 1]) {
      config.outputDir = args[++i];
    } else if (args[i] === "--scale" && args[i + 1]) {
      config.scale = parseFloat(args[++i]);
    } else if (args[i] === "--help") {
      console.log(`
Synthetic Data Generator for Benchmarks

Usage: npx tsx benchmarks/generate.ts [options]

Options:
  --output <dir>   Output directory (default: ./benchmarks/synthetic-data)
  --scale <n>      Scale multiplier for data volume (default: 1)
  --help           Show this help message

Examples:
  npx tsx benchmarks/generate.ts                    # Generate default dataset
  npx tsx benchmarks/generate.ts --scale 2          # Generate 2x data
  npx tsx benchmarks/generate.ts --output /tmp/data # Custom output directory
`);
      process.exit(0);
    }
  }
  
  return config;
}

function main() {
  const userConfig = parseArgs();
  const config: GeneratorConfig = { ...DEFAULT_CONFIG, ...userConfig };
  
  console.log(`\nGenerating synthetic benchmark data...`);
  console.log(`  Output: ${config.outputDir}`);
  console.log(`  Scale: ${config.scale}x`);
  console.log(`  Date range: ${config.startDate.toISOString().split("T")[0]} to ${config.endDate.toISOString().split("T")[0]}`);
  console.log();
  
  // Clean output directory
  if (fs.existsSync(config.outputDir)) {
    fs.rmSync(config.outputDir, { recursive: true });
  }
  ensureDir(config.outputDir);
  
  // Generate data for each source
  generateOpenCodeData(config);
  generateClaudeData(config);
  generateCodexData(config);
  generateGeminiData(config);
  
  // Write metadata
  const metadata = {
    generatedAt: new Date().toISOString(),
    config: {
      scale: config.scale,
      dateRange: {
        start: config.startDate.toISOString(),
        end: config.endDate.toISOString(),
      },
    },
  };
  fs.writeFileSync(
    path.join(config.outputDir, "metadata.json"),
    JSON.stringify(metadata, null, 2)
  );
  
  console.log(`\nDone! Synthetic data generated at: ${config.outputDir}`);
}

main();
