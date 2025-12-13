import {
  parseLocalSourcesNative,
  finalizeReportNative,
  finalizeMonthlyReportNative,
  finalizeGraphNative,
} from "./native.js";

const outputFile = process.argv[2];
if (!outputFile) {
  console.error(JSON.stringify({ error: "Usage: native-runner.ts <output-file>" }));
  process.exit(1);
}

try {
  const input = await Bun.stdin.text();
  
  const parsed = JSON.parse(input);
  if (!parsed.method || !Array.isArray(parsed.args)) {
    throw new Error("Invalid input: missing method or args");
  }
  
  const { method, args } = parsed;
  
  let result: unknown;
  switch (method) {
    case "parseLocalSources":
      result = parseLocalSourcesNative(args[0]);
      break;
    case "finalizeReport":
      result = finalizeReportNative(args[0]);
      break;
    case "finalizeMonthlyReport":
      result = finalizeMonthlyReportNative(args[0]);
      break;
    case "finalizeGraph":
      result = finalizeGraphNative(args[0]);
      break;
    default:
      throw new Error(`Unknown method: ${method}`);
  }
  
  await Bun.write(outputFile, JSON.stringify(result));
} catch (e) {
  const error = e as Error;
  console.error(JSON.stringify({ 
    error: error.message, 
    stack: error.stack 
  }));
  process.exit(1);
}
