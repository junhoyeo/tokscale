#!/usr/bin/env bun
/**
 * Native Runner - Subprocess for non-blocking native Rust calls
 *
 * This script runs in a separate process to keep the main event loop free
 * for UI rendering (e.g., spinner animation).
 *
 * Communication: tmpfile (JSON input) -> tmpfile (JSON output)
 */

import nativeCore from "@tokscale/core";
import { readFileSync, writeFileSync } from "node:fs";

interface NativeRunnerRequest {
  method: string;
  args: unknown[];
}

async function main() {
  const inputFile = process.argv[2];
  const outputFile = process.argv[3];

  if (!inputFile || !outputFile) {
    process.stderr.write(JSON.stringify({ error: "Usage: native-runner <inputFile> <outputFile>" }));
    process.exit(1);
  }

  const input = readFileSync(inputFile, "utf-8");

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
      result = await nativeCore.finalizeReport(args[0] as Parameters<typeof nativeCore.finalizeReport>[0]);
      break;
    case "finalizeMonthlyReport":
      result = await nativeCore.finalizeMonthlyReport(args[0] as Parameters<typeof nativeCore.finalizeMonthlyReport>[0]);
      break;
    case "finalizeGraph":
      result = await nativeCore.finalizeGraph(args[0] as Parameters<typeof nativeCore.finalizeGraph>[0]);
      break;
    case "finalizeReportAndGraph":
      result = await nativeCore.finalizeReportAndGraph(args[0] as Parameters<typeof nativeCore.finalizeReportAndGraph>[0]);
      break;
    default:
      throw new Error(`Unknown method: ${method}`);
  }

  writeFileSync(outputFile, JSON.stringify(result), "utf-8");
}

main().catch((e) => {
  const error = e as Error;
  process.stderr.write(JSON.stringify({
    error: error.message,
    stack: error.stack,
  }));
  process.exit(1);
});
