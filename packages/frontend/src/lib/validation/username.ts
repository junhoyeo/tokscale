export const GITHUB_USERNAME_REGEX = /^[a-zA-Z0-9-]{1,39}$/;

export function isValidGitHubUsername(username: string): boolean {
  return GITHUB_USERNAME_REGEX.test(username);
}
