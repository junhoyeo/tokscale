const DEFAULT_AUTH_RETURN_TO = "/leaderboard";
const RETURN_TO_BASE_URL = "https://tokscale.invalid";

function hasUnsafeRelativeStart(value: string): boolean {
  return !value.startsWith("/") || value.startsWith("//");
}

export function sanitizeAuthReturnTo(value: string | null | undefined): string {
  if (!value) {
    return DEFAULT_AUTH_RETURN_TO;
  }

  const raw = value.trim();
  if (!raw || raw.includes("\\")) {
    return DEFAULT_AUTH_RETURN_TO;
  }

  let decoded: string;
  try {
    decoded = decodeURIComponent(raw);
  } catch {
    return DEFAULT_AUTH_RETURN_TO;
  }

  if (
    decoded.includes("\\") ||
    hasUnsafeRelativeStart(raw) ||
    hasUnsafeRelativeStart(decoded)
  ) {
    return DEFAULT_AUTH_RETURN_TO;
  }

  const baseUrl = new URL(RETURN_TO_BASE_URL);
  let parsed: URL;
  let decodedParsed: URL;
  try {
    parsed = new URL(raw, baseUrl);
    decodedParsed = new URL(decoded, baseUrl);
  } catch {
    return DEFAULT_AUTH_RETURN_TO;
  }

  if (
    parsed.origin !== baseUrl.origin ||
    decodedParsed.origin !== baseUrl.origin ||
    parsed.username ||
    parsed.password ||
    decodedParsed.username ||
    decodedParsed.password
  ) {
    return DEFAULT_AUTH_RETURN_TO;
  }

  return `${parsed.pathname}${parsed.search}${parsed.hash}` || DEFAULT_AUTH_RETURN_TO;
}
