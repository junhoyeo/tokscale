/**
 * Token Tracker CLI Authentication Commands
 * Handles login, logout, and whoami commands
 */

import pc from "picocolors";
import {
  saveCredentials,
  loadCredentials,
  clearCredentials,
  getApiBaseUrl,
} from "./credentials.js";

const MAX_POLL_ATTEMPTS = 180; // 15 minutes max

interface DeviceCodeResponse {
  deviceCode: string;
  userCode: string;
  verificationUrl: string;
  expiresIn: number;
  interval: number;
}

interface PollResponse {
  status: "pending" | "complete" | "expired";
  token?: string;
  user?: {
    username: string;
    avatarUrl?: string;
  };
  error?: string;
}

/**
 * Login command - initiates device flow authentication
 */
export async function login(): Promise<void> {
  const credentials = loadCredentials();
  if (credentials) {
    console.log(pc.yellow(`\n  Already logged in as ${pc.bold(credentials.username)}`));
    console.log(pc.gray("  Run 'token-tracker logout' to sign out first.\n"));
    return;
  }

  const baseUrl = getApiBaseUrl();

  console.log(pc.cyan("\n  Token Tracker - Login\n"));

  // Step 1: Request device code
  console.log(pc.gray("  Requesting authorization code..."));

  let deviceCodeData: DeviceCodeResponse;
  try {
    const response = await fetch(`${baseUrl}/api/auth/device`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ deviceName: getDeviceName() }),
    });

    if (!response.ok) {
      throw new Error(`Server returned ${response.status}`);
    }

    deviceCodeData = await response.json();
  } catch (error) {
    console.error(pc.red(`\n  Error: Failed to connect to server.`));
    console.error(pc.gray(`  ${(error as Error).message}\n`));
    process.exit(1);
  }

  // Step 2: Display instructions
  console.log();
  console.log(pc.white("  Open this URL in your browser:"));
  console.log(pc.cyan(`  ${deviceCodeData.verificationUrl}\n`));
  console.log(pc.white("  Enter this code:"));
  console.log(pc.bold(pc.green(`  ${deviceCodeData.userCode}\n`)));

  // Try to open browser automatically
  await openBrowser(deviceCodeData.verificationUrl);

  console.log(pc.gray("  Waiting for authorization..."));

  // Step 3: Poll for completion
  let attempts = 0;
  const pollInterval = (deviceCodeData.interval || 5) * 1000;

  while (attempts < MAX_POLL_ATTEMPTS) {
    await sleep(pollInterval);
    attempts++;

    try {
      const response = await fetch(`${baseUrl}/api/auth/device/poll`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ deviceCode: deviceCodeData.deviceCode }),
      });

      const data: PollResponse = await response.json();

      if (data.status === "complete" && data.token && data.user) {
        // Success!
        saveCredentials({
          token: data.token,
          username: data.user.username,
          avatarUrl: data.user.avatarUrl,
          createdAt: new Date().toISOString(),
        });

        console.log(pc.green(`\n  Success! Logged in as ${pc.bold(data.user.username)}`));
        console.log(pc.gray("  You can now use 'token-tracker submit' to share your usage.\n"));
        return;
      }

      if (data.status === "expired") {
        console.error(pc.red("\n  Authorization code expired. Please try again.\n"));
        process.exit(1);
      }

      // Still pending - show a dot to indicate progress
      process.stdout.write(pc.gray("."));
    } catch (error) {
      // Network error - continue polling
      process.stdout.write(pc.red("!"));
    }
  }

  console.error(pc.red("\n\n  Timeout: Authorization took too long. Please try again.\n"));
  process.exit(1);
}

/**
 * Logout command - clears stored credentials
 */
export async function logout(): Promise<void> {
  const credentials = loadCredentials();

  if (!credentials) {
    console.log(pc.yellow("\n  Not logged in.\n"));
    return;
  }

  const username = credentials.username;
  const cleared = clearCredentials();

  if (cleared) {
    console.log(pc.green(`\n  Logged out from ${pc.bold(username)}\n`));
  } else {
    console.error(pc.red("\n  Failed to clear credentials.\n"));
    process.exit(1);
  }
}

/**
 * Whoami command - displays current user info
 */
export async function whoami(): Promise<void> {
  const credentials = loadCredentials();

  if (!credentials) {
    console.log(pc.yellow("\n  Not logged in."));
    console.log(pc.gray("  Run 'token-tracker login' to authenticate.\n"));
    return;
  }

  console.log(pc.cyan("\n  Token Tracker - Account Info\n"));
  console.log(pc.white(`  Username:  ${pc.bold(credentials.username)}`));
  console.log(pc.gray(`  Logged in: ${new Date(credentials.createdAt).toLocaleDateString()}`));
  console.log();
}

/**
 * Get a device name for the token
 */
function getDeviceName(): string {
  const hostname = require("node:os").hostname();
  return `CLI on ${hostname}`;
}

/**
 * Sleep helper
 */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Try to open browser automatically
 */
async function openBrowser(url: string): Promise<void> {
  try {
    const { exec } = await import("node:child_process");
    const platform = process.platform;

    let command: string;
    if (platform === "darwin") {
      command = `open "${url}"`;
    } else if (platform === "win32") {
      command = `start "" "${url}"`;
    } else {
      command = `xdg-open "${url}"`;
    }

    exec(command, (error) => {
      if (error) {
        // Silent fail - user can still open manually
      }
    });
  } catch {
    // Silent fail
  }
}
