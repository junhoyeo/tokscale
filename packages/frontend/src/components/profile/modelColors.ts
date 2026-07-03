const MODEL_COLORS: Record<string, string> = {
  // Anthropic families use the same orange ramp as the TUI's ANTHROPIC_SHADES
  // (crates/tokscale-cli/src/tui/ui/widgets.rs): brightness encodes the family
  // hierarchy — fable darkest, then opus, sonnet, haiku, generic claude palest.
  // Lookup is first-match, so family keys must precede the generic "claude"
  // fallback: real IDs like "claude-opus-4-6" contain both keys and would
  // otherwise never reach their family color.
  "fable": "#DA7756",
  "opus": "#DF886B",
  "sonnet": "#E39980",
  "haiku": "#E8AA95",
  "claude": "#ECB8A6",
  "gpt": "#10B981",
  "chatgpt": "#10B981",
  "o1": "#6366F1",
  "o3": "#8B5CF6",
  "gemini": "#3B82F6",
  "deepseek": "#06B6D4",
  "codex": "#F59E0B",
  "kimi": "#A855F7",
  "qwen": "#1A73E8",
};

export function getModelColor(modelName: string): string {
  // Match keys as delimited tokens (optionally followed by a version digit,
  // e.g. "gpt4"), not raw substrings — otherwise ids like "unfabled-x" would
  // land in the Anthropic palette. Mirrors the TUI's delimited matching in
  // get_provider_from_model.
  const tokens = modelName.toLowerCase().split(/[^a-z0-9]+/);
  for (const [key, color] of Object.entries(MODEL_COLORS)) {
    for (const token of tokens) {
      if (token === key || (token.startsWith(key) && /^\d/.test(token.slice(key.length)))) {
        return color;
      }
    }
  }
  return "#6B7280";
}
