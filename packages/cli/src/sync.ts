/**
 * Tokscale CLI Sync Command
 * Manages automatic hourly sync via crontab (macOS/Linux) or Task Scheduler (Windows)
 */

import { execSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import pc from "picocolors";
import { loadCredentials } from "./credentials.js";

const CONFIG_DIR = path.join(os.homedir(), ".config", "tokscale");
const LOG_FILE = path.join(CONFIG_DIR, "sync.log");
const WINDOWS_TASK_NAME = "TokscaleSync";
const WINDOWS_SCRIPT_FILE = path.join(CONFIG_DIR, "sync-task.cmd");
const CRON_MARKER = "# TOKSCALE_SYNC_MANAGED";

function ensureConfigDir(): void {
  if (!fs.existsSync(CONFIG_DIR)) {
    fs.mkdirSync(CONFIG_DIR, { recursive: true, mode: 0o700 });
  }
}

function getTokscalePath(): string {
  return process.argv[1];
}

// Escape single quotes for shell: replace ' with '\''
function escapePathForShell(filePath: string): string {
  return filePath.replace(/'/g, "'\\''");
}

function isWindows(): boolean {
  return process.platform === "win32";
}

// =============================================================================
// Crontab (macOS/Linux)
// =============================================================================

function buildCronEntry(): string {
  const tokscalePath = escapePathForShell(getTokscalePath());
  const logPath = escapePathForShell(LOG_FILE);
  return `0 * * * * '${tokscalePath}' submit --quiet >> '${logPath}' 2>&1 ${CRON_MARKER}`;
}

function setupCrontab(): { success: boolean; error?: string } {
  try {
    ensureConfigDir();
    const cronEntry = buildCronEntry();
    // Remove existing tokscale sync entries (by marker), then add new one
    // Uses || true to handle empty crontab gracefully
    // Use printf with single quotes to prevent shell injection if path contains $() or backticks
    const escapedEntry = cronEntry.replace(/'/g, "'\\''");
    const command = `(crontab -l 2>/dev/null | grep -v '${CRON_MARKER}' || true) | { cat; printf '%s\\n' '${escapedEntry}'; } | crontab -`;
    execSync(command, { stdio: "pipe" });
    return { success: true };
  } catch (error) {
    return { success: false, error: (error as Error).message };
  }
}

function removeCrontab(): { success: boolean; error?: string } {
  try {
    const command = `(crontab -l 2>/dev/null | grep -v '${CRON_MARKER}' || true) | crontab -`;
    execSync(command, { stdio: "pipe" });
    return { success: true };
  } catch (error) {
    return { success: false, error: (error as Error).message };
  }
}

function checkCrontab(): { exists: boolean; entry?: string; error?: string } {
  try {
    const result = execSync(`crontab -l 2>/dev/null | grep '${CRON_MARKER}' || true`, {
      encoding: "utf-8",
      stdio: ["pipe", "pipe", "pipe"],
    });
    const entry = result.trim();
    return { exists: entry.length > 0, entry: entry || undefined };
  } catch (error) {
    return { exists: false, error: (error as Error).message };
  }
}

// =============================================================================
// Windows Task Scheduler
// =============================================================================

function setupWindowsTask(): { success: boolean; error?: string } {
  try {
    ensureConfigDir();
    const tokscalePath = getTokscalePath();
    
    try {
      execSync(`schtasks /delete /tn "${WINDOWS_TASK_NAME}" /f`, { stdio: "pipe" });
    } catch {
      // Task doesn't exist yet
    }
    
    const scriptContent = `@echo off\r\n"${tokscalePath}" submit --quiet >> "${LOG_FILE}" 2>&1\r\n`;
    fs.writeFileSync(WINDOWS_SCRIPT_FILE, scriptContent);
    
    // Use cmd.exe /c wrapper for robustness with Task Scheduler
    const scheduleCmd = `schtasks /create /tn "${WINDOWS_TASK_NAME}" /sc HOURLY /tr "cmd.exe /c \\"${WINDOWS_SCRIPT_FILE}\\"" /f`;
    execSync(scheduleCmd, { stdio: "pipe" });
    return { success: true };
  } catch (error) {
    return { success: false, error: (error as Error).message };
  }
}

function removeWindowsTask(): { success: boolean; error?: string } {
  try {
    execSync(`schtasks /delete /tn "${WINDOWS_TASK_NAME}" /f`, { stdio: "pipe" });
  } catch (error) {
    const errorMsg = (error as Error).message;
    if (!errorMsg.includes("cannot find") && !errorMsg.includes("task name")) {
      return { success: false, error: errorMsg };
    }
  }
  
  try {
    if (fs.existsSync(WINDOWS_SCRIPT_FILE)) {
      fs.unlinkSync(WINDOWS_SCRIPT_FILE);
    }
  } catch {}
  
  return { success: true };
}

function checkWindowsTask(): { exists: boolean; error?: string } {
  try {
    execSync(`schtasks /query /tn "${WINDOWS_TASK_NAME}"`, { stdio: "pipe" });
    return { exists: true };
  } catch (error) {
    const errorMsg = (error as Error).message;
    if (errorMsg.includes("cannot find") || errorMsg.includes("task name")) {
      return { exists: false };
    }
    return { exists: false, error: errorMsg };
  }
}

// =============================================================================
// Public API
// =============================================================================

export interface SyncSetupOptions {
  interval?: string;
}

export async function setupSync(_options: SyncSetupOptions = {}): Promise<void> {
  const credentials = loadCredentials();
  if (!credentials) {
    console.log(pc.red("\n  Error: Not logged in to Tokscale."));
    console.log(pc.gray("  Run 'tokscale login' first before setting up sync.\n"));
    process.exit(1);
  }

  console.log(pc.cyan("\n  Tokscale - Setup Automatic Sync\n"));
  console.log(pc.gray(`  Logged in as: ${credentials.username}`));
  console.log();

  const platform = isWindows() ? "Windows" : process.platform === "darwin" ? "macOS" : "Linux";
  console.log(pc.gray(`  Platform: ${platform}`));
  console.log(pc.gray(`  CLI path: ${getTokscalePath()}`));
  console.log(pc.gray(`  Log file: ${LOG_FILE}`));
  console.log();

  let result: { success: boolean; error?: string };
  
  if (isWindows()) {
    console.log(pc.gray("  Creating Windows Task Scheduler entry..."));
    result = setupWindowsTask();
  } else {
    console.log(pc.gray("  Adding crontab entry..."));
    result = setupCrontab();
  }

  if (result.success) {
    console.log(pc.green("\n  Success! Automatic sync is now configured."));
    console.log();
    console.log(pc.white("  Schedule: Hourly (at minute 0)"));
    console.log(pc.white("  Command: tokscale submit --quiet"));
    if (isWindows()) {
      console.log(pc.gray(`  Script: ${WINDOWS_SCRIPT_FILE}`));
    }
    console.log(pc.gray(`  Logs: ${LOG_FILE}`));
    console.log();
    console.log(pc.gray("  Your usage data will be automatically submitted every hour."));
    console.log(pc.gray("  Use 'tokscale sync status' to check status."));
    console.log(pc.gray("  Use 'tokscale sync remove' to disable.\n"));
  } else {
    console.log(pc.red(`\n  Error: Failed to set up sync.`));
    console.log(pc.gray(`  ${result.error}\n`));
    process.exit(1);
  }
}

export async function removeSync(): Promise<void> {
  console.log(pc.cyan("\n  Tokscale - Remove Automatic Sync\n"));

  let result: { success: boolean; error?: string };
  
  if (isWindows()) {
    console.log(pc.gray("  Removing Windows Task Scheduler entry..."));
    result = removeWindowsTask();
  } else {
    console.log(pc.gray("  Removing crontab entry..."));
    result = removeCrontab();
  }

  if (result.success) {
    console.log(pc.green("\n  Success! Automatic sync has been removed."));
    console.log(pc.gray("  Your usage data will no longer be automatically submitted.\n"));
  } else {
    console.log(pc.red(`\n  Error: Failed to remove sync.`));
    console.log(pc.gray(`  ${result.error}\n`));
    process.exit(1);
  }
}

export async function syncStatus(): Promise<void> {
  console.log(pc.cyan("\n  Tokscale - Sync Status\n"));

  const platform = isWindows() ? "Windows" : process.platform === "darwin" ? "macOS" : "Linux";
  console.log(pc.gray(`  Platform: ${platform}`));

  if (isWindows()) {
    const status = checkWindowsTask();
    if (status.error) {
      console.log(pc.yellow(`  Status: Unable to check (${status.error})`));
    } else if (status.exists) {
      console.log(pc.green("  Status: Active"));
      console.log(pc.gray(`  Task: ${WINDOWS_TASK_NAME}`));
      console.log(pc.gray(`  Script: ${WINDOWS_SCRIPT_FILE}`));
      console.log(pc.gray(`  Schedule: Hourly`));
      console.log(pc.gray(`  Logs: ${LOG_FILE}`));
    } else {
      console.log(pc.yellow("  Status: Not configured"));
      console.log(pc.gray("  Run 'tokscale sync setup' to enable automatic sync."));
    }
  } else {
    const status = checkCrontab();
    if (status.error) {
      console.log(pc.yellow(`  Status: Unable to check (${status.error})`));
    } else if (status.exists) {
      console.log(pc.green("  Status: Active"));
      console.log(pc.gray(`  Entry: ${status.entry}`));
      console.log(pc.gray(`  Logs: ${LOG_FILE}`));
    } else {
      console.log(pc.yellow("  Status: Not configured"));
      console.log(pc.gray("  Run 'tokscale sync setup' to enable automatic sync."));
    }
  }

  if (fs.existsSync(LOG_FILE)) {
    const stats = fs.statSync(LOG_FILE);
    const lastModified = stats.mtime.toLocaleString();
    console.log(pc.gray(`  Last sync log: ${lastModified}`));
  }

  const credentials = loadCredentials();
  if (!credentials) {
    console.log(pc.yellow("\n  Warning: Not logged in to Tokscale."));
    console.log(pc.gray("  Sync will fail without authentication."));
    console.log(pc.gray("  Run 'tokscale login' to authenticate."));
  } else {
    console.log(pc.gray(`  Logged in as: ${credentials.username}`));
  }

  console.log();
}
