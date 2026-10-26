/** Words that stay lowercase inside a Title Case label (unless they start a segment). */
const SMALL_WORDS = new Set([
  "a", "an", "and", "as", "at", "but", "by", "for", "from", "in", "into", "nor", "of", "on", "or", "per", "the", "to", "up", "via", "vs", "with",
]);

/** Acronyms, numbers, mixed case (xAI, SpaceX) and domains are kept exactly as written. */
function keepAsWritten(word: string): boolean {
  return /\d/.test(word) || /[.@/]/.test(word) || /[A-Z].*[A-Z]/.test(word) || /[a-z][A-Z]/.test(word);
}

function capitalize(word: string): string {
  const i = word.search(/[A-Za-z]/);
  return i < 0 ? word : word.slice(0, i) + word[i].toUpperCase() + word.slice(i + 1);
}

/**
 * Title Case for eyebrows and labels: "Equity raised · 2026 so far" -> "Equity Raised · 2026 So Far",
 * "Trusted by professionals at" -> "Trusted by Professionals at". Each " · " segment starts with a capital.
 */
export function titleCase(text: string): string {
  return text
    .split(" · ")
    .map((segment) => {
      let first = true;
      return segment
        .split(/(\s+)/)
        .map((token) => {
          if (!token.trim()) return token;
          const isFirst = first;
          first = false;
          if (keepAsWritten(token)) return token;
          const bare = token.toLowerCase().replace(/[^a-z]/g, "");
          if (!isFirst && SMALL_WORDS.has(bare)) return token.toLowerCase();
          return capitalize(token);
        })
        .join("");
    })
    .join(" · ");
}
