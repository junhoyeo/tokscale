const GITHUB_AVATAR_HOST = "avatars.githubusercontent.com";

/**
 * GitHub serves an avatar at up to 460 px unless asked for less. Request the
 * size that is drawn, doubled for dense screens, so a list of small avatars
 * costs a few KB each instead of tens of KB. Other hosts are left alone.
 */
export function sizedAvatarUrl(avatarUrl: string, displayPx: number): string {
  try {
    const url = new URL(avatarUrl);
    if (url.hostname !== GITHUB_AVATAR_HOST) return avatarUrl;
    url.searchParams.set("s", String(Math.round(displayPx * 2)));
    return url.toString();
  } catch {
    return avatarUrl;
  }
}

/** An account's avatar at `displayPx`; without a stored one, its current GitHub picture. */
export function avatarSrc(avatarUrl: string | null | undefined, username: string, displayPx: number): string {
  if (avatarUrl) return sizedAvatarUrl(avatarUrl, displayPx);
  return `https://github.com/${encodeURIComponent(username)}.png?size=${Math.round(displayPx * 2)}`;
}
