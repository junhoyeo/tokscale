import { randomBytes, createHash } from "crypto";

/**
 * Generate a cryptographically secure random string.
 */
export function generateRandomString(length: number): string {
  return randomBytes(length).toString("hex").slice(0, length);
}

/**
 * Generate a human-readable user code for device flow.
 * Format: XXXX-XXXX (uppercase alphanumeric, no ambiguous chars)
 */
export function generateUserCode(): string {
  // Exclude ambiguous characters: 0, O, I, L, 1
  const chars = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
  let code = "";
  const bytes = randomBytes(8);

  for (let i = 0; i < 8; i++) {
    code += chars[bytes[i] % chars.length];
    if (i === 3) code += "-";
  }

  return code;
}

/**
 * Generate an API token with prefix.
 * Format: tt_<random>
 */
export function generateApiToken(): string {
  return `tt_${randomBytes(24).toString("hex")}`;
}

/**
 * Hash a token using SHA256 for secure storage comparison.
 */
export function hashToken(token: string): string {
  return createHash("sha256").update(token).digest("hex");
}

/**
 * Generate a device code (internal use, not shown to user).
 */
export function generateDeviceCode(): string {
  return randomBytes(16).toString("hex");
}
