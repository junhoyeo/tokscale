import { describe, expect, it } from "vitest";
import { getModelColor } from "../../src/components/profile/modelColors";

// The Claude family ramp mirrors the TUI's ANTHROPIC_SHADES: darker = higher
// tier. See crates/tokscale-cli/src/tui/ui/widgets.rs.
const FABLE_ORANGE = "#DA7756";
const OPUS_ORANGE = "#DF886B";
const SONNET_ORANGE = "#E39980";
const HAIKU_ORANGE = "#E8AA95";
const CLAUDE_ORANGE = "#ECB8A6";
const UNKNOWN_GRAY = "#6B7280";

function luminance(hex: string): number {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return 0.299 * r + 0.587 * g + 0.114 * b;
}

describe("getModelColor", () => {
  it("resolves family colors even when the id carries the claude- prefix", () => {
    // Regression: the generic "claude" key used to match first, so every
    // real Opus/Haiku id (they all start with "claude-") got the fallback.
    expect(getModelColor("claude-opus-4-6")).toBe(OPUS_ORANGE);
    expect(getModelColor("claude-opus-4-5-thinking-high")).toBe(OPUS_ORANGE);
    expect(getModelColor("claude-4-5-opus-high-thinking")).toBe(OPUS_ORANGE);
    expect(getModelColor("claude-haiku-4-5")).toBe(HAIKU_ORANGE);
  });

  it("renders Fable in the darkest Claude orange, one step above Opus", () => {
    expect(getModelColor("claude-fable-5")).toBe(FABLE_ORANGE);
    expect(getModelColor("fable-5")).toBe(FABLE_ORANGE);
  });

  it("orders the Claude ramp by family tier: fable > opus > sonnet > haiku > generic", () => {
    expect(getModelColor("claude-sonnet-5")).toBe(SONNET_ORANGE);
    expect(getModelColor("claude-3-5")).toBe(CLAUDE_ORANGE);

    const ramp = [FABLE_ORANGE, OPUS_ORANGE, SONNET_ORANGE, HAIKU_ORANGE, CLAUDE_ORANGE];
    for (let i = 1; i < ramp.length; i++) {
      expect(luminance(ramp[i])).toBeGreaterThan(luminance(ramp[i - 1]));
    }
  });

  it("falls back to gray for unknown models", () => {
    expect(getModelColor("fugu")).toBe(UNKNOWN_GRAY);
  });

  it("matches keys as delimited tokens, not raw substrings", () => {
    // Regression (PR #808 review): includes() matching painted unrelated ids
    // containing "fable"/"opus" fragments in Anthropic colors.
    expect(getModelColor("unfabled-5")).toBe(UNKNOWN_GRAY);
    expect(getModelColor("fableton-1")).toBe(UNKNOWN_GRAY);
    // But version digits may trail a key without a delimiter...
    expect(getModelColor("gpt4")).toBe(getModelColor("gpt-4"));
    expect(getModelColor("qwen2.5-72b")).toBe(getModelColor("qwen-max"));
    // ...and bracketed context markers tokenize away cleanly.
    expect(getModelColor("claude-fable-5[1m]")).toBe(FABLE_ORANGE);
    expect(getModelColor("chatgpt-4o-latest")).toBe(getModelColor("gpt-4o"));
  });
});
