#!/usr/bin/env node
/**
 * Thin wrapper forwarding to @tokscale/cli for reliable `npm install -g tokscale`
 * across all package managers (npm, yarn, pnpm, bun).
 */
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';

const require = createRequire(import.meta.url);
const cliPkgPath = require.resolve('@tokscale/cli/package.json');
const cliPkg = require(cliPkgPath);
const cliBinPath = join(dirname(cliPkgPath), cliPkg.bin.tokscale || cliPkg.main);

const child = spawn(process.execPath, [cliBinPath, ...process.argv.slice(2)], {
  stdio: 'inherit',
  windowsHide: true,
});

child.on('close', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  }
  process.exit(code ?? 1);
});

child.on('error', (err) => {
  console.error('Failed to start tokscale:', err.message);
  process.exit(1);
});
