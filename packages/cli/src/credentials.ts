/**
 * Tokscale CLI Credentials Manager
 * Stores and retrieves API tokens for authenticated CLI operations
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

export interface Credentials {
  token: string;
  username: string;
  avatarUrl?: string;
  createdAt: string;
}

const OLD_CONFIG_DIR = path.join(os.homedir(), ".tokscale");
const CONFIG_DIR = path.join(os.homedir(), ".config", "tokscale");
const OLD_CREDENTIALS_FILE = path.join(OLD_CONFIG_DIR, "credentials.json");
const CREDENTIALS_FILE = path.join(CONFIG_DIR, "credentials.json");

/**
 * Ensure the config directory exists
 */
function ensureConfigDir(): void {
  if (!fs.existsSync(CONFIG_DIR)) {
    fs.mkdirSync(CONFIG_DIR, { recursive: true, mode: 0o700 });
  }
}

/**
 * Migrate credentials from old path (~/.token-tracker) to new XDG path (~/.config/token-tracker)
 */
function migrateFromOldPath(): void {
  try {
    if (!fs.existsSync(CREDENTIALS_FILE) && fs.existsSync(OLD_CREDENTIALS_FILE)) {
      ensureConfigDir();
      fs.copyFileSync(OLD_CREDENTIALS_FILE, CREDENTIALS_FILE);
      fs.chmodSync(CREDENTIALS_FILE, 0o600);
      // Delete old file after successful migration
      fs.unlinkSync(OLD_CREDENTIALS_FILE);
      // Try to remove old directory if empty
      try {
        fs.rmdirSync(OLD_CONFIG_DIR);
      } catch {
        // Directory not empty (cursor files may exist) - ignore
      }
    }
  } catch {
    // Migration failed - continue with normal operation (old path may still work)
  }
}

/**
 * Save credentials to disk
 */
export function saveCredentials(credentials: Credentials): void {
  ensureConfigDir();
  fs.writeFileSync(CREDENTIALS_FILE, JSON.stringify(credentials, null, 2), {
    encoding: "utf-8",
    mode: 0o600, // Read/write for owner only
  });
}

/**
 * Load credentials from disk
 * Returns null if no credentials are stored or file is invalid
 */
export function loadCredentials(): Credentials | null {
  migrateFromOldPath();
  try {
    if (!fs.existsSync(CREDENTIALS_FILE)) {
      return null;
    }
    const data = fs.readFileSync(CREDENTIALS_FILE, "utf-8");
    const parsed = JSON.parse(data);

    // Validate structure
    if (!parsed.token || !parsed.username) {
      return null;
    }

    return parsed as Credentials;
  } catch {
    return null;
  }
}

/**
 * Clear stored credentials
 */
export function clearCredentials(): boolean {
  try {
    if (fs.existsSync(CREDENTIALS_FILE)) {
      fs.unlinkSync(CREDENTIALS_FILE);
      return true;
    }
    return false;
  } catch {
    return false;
  }
}

/**
 * Check if user is logged in
 */
export function isLoggedIn(): boolean {
  return loadCredentials() !== null;
}

/**
 * Get the API base URL
 */
export function getApiBaseUrl(): string {
  return process.env.TOKSCALE_API_URL || "https://tokens.inevitable.team";
}

/**
 * Get the credentials file path (for debugging)
 */
export function getCredentialsPath(): string {
  return CREDENTIALS_FILE;
}
