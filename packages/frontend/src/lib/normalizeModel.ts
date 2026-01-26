export function normalizeDisplayModelId(modelId: string): string {
  let s = modelId.toLowerCase();

  if (s.startsWith("antigravity-")) {
    s = s.slice("antigravity-".length);
  }

  s = stripOuterModelFamily(s);

  const slashIdx = s.lastIndexOf("/");
  if (slashIdx !== -1) {
    s = s.slice(slashIdx + 1);
  }

  s = s.replace(/-(19|20)\d{6}$/, "");
  s = s.replace(/-preview-\d{2}-\d{2}$/, "");
  s = s.replace(/-exp-\d{2}-\d{2}$/, "");
  s = s.replace(/-preview$/, "");
  s = s.replace(/-exp$/, "");
  s = s.replace(/-latest$/, "");

  s = stripTierSuffixes(s);
  s = stripTrailingIteration(s);
  s = stripClaudeThinking(s);
  s = stripMaxSuffix(s);
  s = stripClaudeThinking(s);
  s = fixClaudeWrongOrder(s);
  s = normalizeVersionSeparator(s);

  return s;
}

const TIER_WORDS = new Set(["low", "high", "fast", "free", "extra", "medium", "xhigh"]);

function stripTierSuffixes(s: string): string {
  const parts = s.split("-");
  if (parts.length <= 1) return s;
  let end = parts.length;
  while (end > 1 && TIER_WORDS.has(parts[end - 1])) {
    end--;
  }
  if (end === parts.length) return s;
  return parts.slice(0, end).join("-");
}

function stripOuterModelFamily(s: string): string {
  const firstDash = s.indexOf("-");
  if (firstDash === -1) return s;
  const rest = s.slice(firstDash + 1);
  if (
    rest.startsWith("claude-") ||
    rest.startsWith("gpt-") ||
    rest.startsWith("gemini-")
  ) {
    return rest;
  }
  return s;
}

function fixClaudeWrongOrder(s: string): string {
  if (!s.startsWith("claude-")) return s;
  const rest = s.slice("claude-".length);
  const dashIdx = rest.indexOf("-");
  if (dashIdx === -1) return s;
  const maybeVersion = rest.slice(0, dashIdx);
  const maybeFamily = rest.slice(dashIdx + 1);
  const versionNum = parseInt(maybeVersion.split(".")[0], 10);
  if (
    versionNum >= 4 &&
    /^[\d.]+$/.test(maybeVersion) &&
    maybeFamily.length > 0 &&
    /^[a-z]/.test(maybeFamily)
  ) {
    const familyWord = maybeFamily.split("-")[0];
    if (familyWord === "opus" || familyWord === "sonnet" || familyWord === "haiku") {
      const afterFamily = maybeFamily.slice(familyWord.length);
      return `claude-${familyWord}${afterFamily}-${maybeVersion}`;
    }
  }
  return s;
}

function stripMaxSuffix(s: string): string {
  if (s.includes("codex-max")) return s;
  return s.replace(/-max$/, "");
}

function stripTrailingIteration(s: string): string {
  const lastDash = s.lastIndexOf("-");
  if (lastDash <= 0) return s;
  const suffix = s.slice(lastDash + 1);
  if (!/^\d+$/.test(suffix)) return s;
  const base = s.slice(0, lastDash);
  if (!base.includes("-")) return s;
  const isMultiDigit = suffix.length >= 2;
  const afterMax = base.endsWith("-max");
  if (
    (isMultiDigit && (s.startsWith("claude-") || s.startsWith("gemini-"))) ||
    afterMax
  ) {
    return base;
  }
  return s;
}

function stripClaudeThinking(s: string): string {
  if (!s.startsWith("claude-")) return s;
  s = s.replace(/-high-thinking$/, "");
  s = s.replace(/-thinking$/, "");
  return s;
}

export function formatModelDisplayName(normalizedId: string): string {
  return normalizedId
    .split("-")
    .map((part) => {
      if (/^[\d.]+$/.test(part)) return part;
      if (part === "gpt") return "GPT";
      if (part === "glm") return "GLM";
      if (/^o\d/.test(part)) return part;
      return part.charAt(0).toUpperCase() + part.slice(1);
    })
    .join(" ");
}

function normalizeVersionSeparator(s: string): string {
  const chars = s.split("");
  for (let i = 1; i < chars.length - 1; i++) {
    if (
      chars[i] === "-" &&
      /\d/.test(chars[i - 1]) &&
      /\d/.test(chars[i + 1]) &&
      (i < 2 || !/\d/.test(chars[i - 2])) &&
      (i >= chars.length - 2 || !/\d/.test(chars[i + 2]))
    ) {
      chars[i] = ".";
    }
  }
  return chars.join("");
}
