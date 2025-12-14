#!/usr/bin/env bun
/**
 * Native Runner - Subprocess for non-blocking native Rust calls
 * 
 * This script runs in a separate process to keep the main event loop free
 * for UI rendering (e.g., spinner animation).
 * 
 * Communication: stdin (JSON input) -> stdout (JSON output)
 * No temp files needed - pure Unix IPC pattern.
 */

import nativeCore from "tokscale-core";

const MAX_INPUT_SIZE = 50 * 1024 * 1024; // 50MB
const STDIN_TIMEOUT_MS = 30_000; // 30s

interface NativeRunnerRequest {
  method: string;
  args: unknown[];
}

async function readStdinWithLimits(): Promise<string> {
  const chunks: Buffer[] = [];
  let totalSize = 0;
  let timeoutId: ReturnType<typeof setTimeout> | null = null;

  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => reject(new Error(`Stdin read timed out after ${STDIN_TIMEOUT_MS}ms`)), STDIN_TIMEOUT_MS);
  });

  const readPromise = (async () => {
    for await (const chunk of process.stdin) {
      const buf = Buffer.from(chunk as ArrayBuffer);
      totalSize += buf.length;
      if (totalSize > MAX_INPUT_SIZE) {
        throw new Error(`Input exceeds maximum size of ${MAX_INPUT_SIZE} bytes (${Math.round(MAX_INPUT_SIZE / 1024 / 1024)}MB)`);
      }
      chunks.push(buf);
    }
    return Buffer.concat(chunks).toString("utf-8");
  })();

  try {
    const result = await Promise.race([readPromise, timeoutPromise]);
    return result;
  } finally {
    if (timeoutId) clearTimeout(timeoutId);
  }
}

async function main() {
  const input = await readStdinWithLimits();
  
  if (!input.trim()) {
    process.stderr.write(JSON.stringify({ error: "No input received" }));
    process.exit(1);
  }
  
  let request: NativeRunnerRequest;
  try {
    request = JSON.parse(input) as NativeRunnerRequest;
  } catch (e) {
    throw new Error(`Malformed JSON input: ${(e as Error).message}`);
  }
  
  const { method, args } = request;
  
  if (!Array.isArray(args) || args.length === 0) {
    throw new Error(`Invalid args for method '${method}': expected at least 1 argument`);
  }
  
  let result: unknown;
  
  switch (method) {
    case "parseLocalSources":
      result = nativeCore.parseLocalSources(args[0] as Parameters<typeof nativeCore.parseLocalSources>[0]);
      break;
    case "finalizeReport":
      result = nativeCore.finalizeReport(args[0] as Parameters<typeof nativeCore.finalizeReport>[0]);
      break;
    case "finalizeMonthlyReport":
      result = nativeCore.finalizeMonthlyReport(args[0] as Parameters<typeof nativeCore.finalizeMonthlyReport>[0]);
      break;
    case "finalizeGraph":
      result = nativeCore.finalizeGraph(args[0] as Parameters<typeof nativeCore.finalizeGraph>[0]);
      break;
    case "generateGraphWithPricing":
      result = nativeCore.generateGraphWithPricing(args[0] as Parameters<typeof nativeCore.generateGraphWithPricing>[0]);
      break;
    default:
      throw new Error(`Unknown method: ${method}`);
  }
  
  // Write result to stdout (no newline - pure JSON)
  process.stdout.write(JSON.stringify(result));
}

main().catch((e) => {
  const error = e as Error;
  process.stderr.write(JSON.stringify({ 
    error: error.message,
    stack: error.stack,
  }));
  process.exit(1);
});
