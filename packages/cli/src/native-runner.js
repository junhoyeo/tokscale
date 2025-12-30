#!/usr/bin/env bun
// Shim for bunx: when Bun loads src/ directly instead of dist/,
// this file redirects to the TypeScript source.
await import("./native-runner.ts");
