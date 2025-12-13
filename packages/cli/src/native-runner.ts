#!/usr/bin/env bun

import nativeCore from "@0xinevitable/token-tracker-core";

interface NativeRunnerRequest {
  method: string;
  args: unknown[];
}

async function runNativeMethodFromStdin() {
  const chunks: Buffer[] = [];
  
  for await (const chunk of process.stdin) {
    chunks.push(Buffer.from(chunk as ArrayBuffer));
  }
  
  const input = Buffer.concat(chunks).toString("utf-8");
  const { method, args } = JSON.parse(input) as NativeRunnerRequest;
  
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
    default:
      throw new Error(`Unknown method: ${method}`);
  }
  
  process.stdout.write(JSON.stringify(result));
}

runNativeMethodFromStdin().catch((e) => {
  process.stderr.write(JSON.stringify({ error: (e as Error).message }));
  process.exit(1);
});
