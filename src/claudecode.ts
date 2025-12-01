/**
 * Claude Code (Anthropic official) and Codex CLI data readers
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

export interface ClaudeCodeUsageData {
  source: "claude" | "codex";
  model: string;
  messageCount: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

/**
 * Individual message with timestamp for graph generation
 */
export interface ClaudeCodeMessageWithTimestamp {
  source: "claude" | "codex";
  model: string;
  timestamp: number; // Unix milliseconds
  tokens: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
  };
}

// ============================================================================
// Claude Code (Anthropic official) - ~/.claude/projects/
// ============================================================================

export function getClaudeCodeProjectsPath(): string {
  return path.join(os.homedir(), ".claude", "projects");
}

export function readClaudeCodeSessions(): ClaudeCodeUsageData[] {
  const projectsPath = getClaudeCodeProjectsPath();

  if (!fs.existsSync(projectsPath)) {
    return [];
  }

  const modelUsage = new Map<string, ClaudeCodeUsageData>();
  const files = findJsonlFiles(projectsPath);

  for (const file of files) {
    try {
      const content = fs.readFileSync(file, "utf-8");
      const lines = content.split(/\r?\n/);

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;

        try {
          const entry = JSON.parse(trimmed);

          // Process assistant messages with usage data
          if (entry.type === "assistant" && entry.message?.usage) {
            const model = entry.message.model || "unknown";
            const usage = entry.message.usage;

            // Get or create usage data for this model
            let data = modelUsage.get(model);
            if (!data) {
              data = {
                source: "claude",
                model,
                messageCount: 0,
                input: 0,
                output: 0,
                cacheRead: 0,
                cacheWrite: 0,
              };
              modelUsage.set(model, data);
            }

            data.messageCount++;
            data.input += usage.input_tokens || 0;
            data.output += usage.output_tokens || 0;
            data.cacheRead += usage.cache_read_input_tokens || 0;
            data.cacheWrite += usage.cache_creation_input_tokens || 0;
          }
        } catch {
          // Skip malformed lines
        }
      }
    } catch {
      // Skip unreadable files
    }
  }

  return Array.from(modelUsage.values());
}

/**
 * Read Claude Code messages with timestamp for graph generation
 */
export function readClaudeCodeMessagesWithTimestamp(): ClaudeCodeMessageWithTimestamp[] {
  const projectsPath = getClaudeCodeProjectsPath();

  if (!fs.existsSync(projectsPath)) {
    return [];
  }

  const messages: ClaudeCodeMessageWithTimestamp[] = [];
  const files = findJsonlFiles(projectsPath);

  for (const file of files) {
    try {
      const content = fs.readFileSync(file, "utf-8");
      const lines = content.split(/\r?\n/);

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;

        try {
          const entry = JSON.parse(trimmed);

          // Process assistant messages with usage data and timestamp
          if (entry.type === "assistant" && entry.message?.usage && entry.timestamp) {
            const model = entry.message.model || "unknown";
            const usage = entry.message.usage;
            const timestamp = new Date(entry.timestamp).getTime();

            // Skip invalid timestamps
            if (isNaN(timestamp)) continue;

            messages.push({
              source: "claude",
              model,
              timestamp,
              tokens: {
                input: usage.input_tokens || 0,
                output: usage.output_tokens || 0,
                cacheRead: usage.cache_read_input_tokens || 0,
                cacheWrite: usage.cache_creation_input_tokens || 0,
              },
            });
          }
        } catch {
          // Skip malformed lines
        }
      }
    } catch {
      // Skip unreadable files
    }
  }

  return messages;
}

// ============================================================================
// Codex CLI (OpenAI) - ~/.codex/sessions/
// ============================================================================

interface CodexTokenCountInfo {
  last_token_usage?: {
    input_tokens?: number;
    cached_input_tokens?: number;
    cache_read_input_tokens?: number;
    output_tokens?: number;
    reasoning_output_tokens?: number;
    total_tokens?: number;
  };
  total_token_usage?: {
    input_tokens?: number;
    cached_input_tokens?: number;
    cache_read_input_tokens?: number;
    output_tokens?: number;
    reasoning_output_tokens?: number;
    total_tokens?: number;
  };
  model?: string;
  model_name?: string;
}

export function getCodexSessionsPath(): string {
  const codexHome = process.env.CODEX_HOME || path.join(os.homedir(), ".codex");
  return path.join(codexHome, "sessions");
}

export function readCodexSessions(): ClaudeCodeUsageData[] {
  const sessionsPath = getCodexSessionsPath();

  if (!fs.existsSync(sessionsPath)) {
    return [];
  }

  const modelUsage = new Map<string, ClaudeCodeUsageData>();
  const files = findJsonlFiles(sessionsPath);

  for (const file of files) {
    try {
      const content = fs.readFileSync(file, "utf-8");
      const lines = content.split(/\r?\n/);

      let currentModel: string | undefined;
      let previousTotals: {
        input: number;
        cached: number;
        output: number;
      } | null = null;

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;

        try {
          const entry = JSON.parse(trimmed);

          // Extract model from turn_context
          if (entry.type === "turn_context" && entry.payload) {
            const model = extractModel(entry.payload);
            if (model) currentModel = model;
            continue;
          }

          // Process token_count events
          if (entry.type === "event_msg" && entry.payload?.type === "token_count") {
            const info = entry.payload.info as CodexTokenCountInfo | undefined;
            if (!info) continue;

            // Extract model from payload
            const payloadModel = extractModel(entry.payload);
            if (payloadModel) currentModel = payloadModel;

            const model = currentModel || "unknown";

            // Get usage data
            const lastUsage = info.last_token_usage;
            const totalUsage = info.total_token_usage;

            let delta = {
              input: 0,
              cached: 0,
              output: 0,
            };

            if (lastUsage) {
              delta = {
                input: lastUsage.input_tokens || 0,
                cached: lastUsage.cached_input_tokens || lastUsage.cache_read_input_tokens || 0,
                output: lastUsage.output_tokens || 0,
              };
            } else if (totalUsage && previousTotals) {
              delta = {
                input: Math.max((totalUsage.input_tokens || 0) - previousTotals.input, 0),
                cached: Math.max(
                  (totalUsage.cached_input_tokens || totalUsage.cache_read_input_tokens || 0) -
                    previousTotals.cached,
                  0
                ),
                output: Math.max((totalUsage.output_tokens || 0) - previousTotals.output, 0),
              };
            }

            if (totalUsage) {
              previousTotals = {
                input: totalUsage.input_tokens || 0,
                cached: totalUsage.cached_input_tokens || totalUsage.cache_read_input_tokens || 0,
                output: totalUsage.output_tokens || 0,
              };
            }

            // Skip empty deltas
            if (delta.input === 0 && delta.cached === 0 && delta.output === 0) {
              continue;
            }

            // Aggregate by model
            let usage = modelUsage.get(model);
            if (!usage) {
              usage = {
                source: "codex",
                model,
                messageCount: 0,
                input: 0,
                output: 0,
                cacheRead: 0,
                cacheWrite: 0,
              };
              modelUsage.set(model, usage);
            }

            usage.messageCount++;
            usage.input += delta.input;
            usage.output += delta.output;
            usage.cacheRead += delta.cached;
          }
        } catch {
          // Skip malformed lines
        }
      }
    } catch {
      // Skip unreadable files
    }
  }

  return Array.from(modelUsage.values());
}

/**
 * Read Codex messages with timestamp for graph generation
 */
export function readCodexMessagesWithTimestamp(): ClaudeCodeMessageWithTimestamp[] {
  const sessionsPath = getCodexSessionsPath();

  if (!fs.existsSync(sessionsPath)) {
    return [];
  }

  const messages: ClaudeCodeMessageWithTimestamp[] = [];
  const files = findJsonlFiles(sessionsPath);

  for (const file of files) {
    try {
      const content = fs.readFileSync(file, "utf-8");
      const lines = content.split(/\r?\n/);

      let currentModel: string | undefined;
      let previousTotals: {
        input: number;
        cached: number;
        output: number;
      } | null = null;

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;

        try {
          const entry = JSON.parse(trimmed);

          // Extract model from turn_context
          if (entry.type === "turn_context" && entry.payload) {
            const model = extractModel(entry.payload);
            if (model) currentModel = model;
            continue;
          }

          // Process token_count events with timestamp
          if (entry.type === "event_msg" && entry.payload?.type === "token_count") {
            const info = entry.payload.info as CodexTokenCountInfo | undefined;
            if (!info) continue;

            // Extract model from payload
            const payloadModel = extractModel(entry.payload);
            if (payloadModel) currentModel = payloadModel;

            const model = currentModel || "unknown";

            // Extract timestamp
            const timestamp = entry.timestamp
              ? new Date(entry.timestamp).getTime()
              : entry.payload?.timestamp
              ? new Date(entry.payload.timestamp).getTime()
              : Date.now();

            // Skip invalid timestamps
            if (isNaN(timestamp)) continue;

            // Get usage data
            const lastUsage = info.last_token_usage;
            const totalUsage = info.total_token_usage;

            let delta = {
              input: 0,
              cached: 0,
              output: 0,
            };

            if (lastUsage) {
              delta = {
                input: lastUsage.input_tokens || 0,
                cached: lastUsage.cached_input_tokens || lastUsage.cache_read_input_tokens || 0,
                output: lastUsage.output_tokens || 0,
              };
            } else if (totalUsage && previousTotals) {
              delta = {
                input: Math.max((totalUsage.input_tokens || 0) - previousTotals.input, 0),
                cached: Math.max(
                  (totalUsage.cached_input_tokens || totalUsage.cache_read_input_tokens || 0) -
                    previousTotals.cached,
                  0
                ),
                output: Math.max((totalUsage.output_tokens || 0) - previousTotals.output, 0),
              };
            }

            if (totalUsage) {
              previousTotals = {
                input: totalUsage.input_tokens || 0,
                cached: totalUsage.cached_input_tokens || totalUsage.cache_read_input_tokens || 0,
                output: totalUsage.output_tokens || 0,
              };
            }

            // Skip empty deltas
            if (delta.input === 0 && delta.cached === 0 && delta.output === 0) {
              continue;
            }

            messages.push({
              source: "codex",
              model,
              timestamp,
              tokens: {
                input: delta.input,
                output: delta.output,
                cacheRead: delta.cached,
                cacheWrite: 0, // Codex doesn't track cache write
              },
            });
          }
        } catch {
          // Skip malformed lines
        }
      }
    } catch {
      // Skip unreadable files
    }
  }

  return messages;
}

// ============================================================================
// Utilities
// ============================================================================

function findJsonlFiles(dir: string): string[] {
  const files: string[] = [];

  function walk(currentDir: string) {
    try {
      const entries = fs.readdirSync(currentDir, { withFileTypes: true });
      for (const entry of entries) {
        const fullPath = path.join(currentDir, entry.name);
        if (entry.isDirectory()) {
          walk(fullPath);
        } else if (entry.isFile() && entry.name.endsWith(".jsonl")) {
          files.push(fullPath);
        }
      }
    } catch {
      // Skip inaccessible directories
    }
  }

  walk(dir);
  return files;
}

function extractModel(payload: Record<string, unknown>): string | undefined {
  // Direct model field
  if (typeof payload.model === "string" && payload.model.trim()) {
    return payload.model.trim();
  }

  // model_name field
  if (typeof payload.model_name === "string" && payload.model_name.trim()) {
    return payload.model_name.trim();
  }

  // Nested in info
  const info = payload.info as Record<string, unknown> | undefined;
  if (info) {
    if (typeof info.model === "string" && info.model.trim()) {
      return info.model.trim();
    }
    if (typeof info.model_name === "string" && info.model_name.trim()) {
      return info.model_name.trim();
    }
  }

  return undefined;
}
