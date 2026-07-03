import { describe, expect, it } from "vitest";
import { getModelColor } from "../../src/components/profile/modelColors";

const OPUS_RED = "#DC2626";
const CLAUDE_AMBER = "#D97706";
const HAIKU_GREEN = "#059669";
const UNKNOWN_GRAY = "#6B7280";

describe("getModelColor", () => {
  it("renders Opus red even when the id carries the claude- prefix", () => {
    // Regression: the generic "claude" key used to match first, so every
    // real Opus id (they all start with "claude-") rendered amber.
    expect(getModelColor("claude-opus-4-6")).toBe(OPUS_RED);
    expect(getModelColor("claude-opus-4-5-thinking-high")).toBe(OPUS_RED);
    expect(getModelColor("claude-4-5-opus-high-thinking")).toBe(OPUS_RED);
  });

  it("renders Fable in the same red as Opus", () => {
    expect(getModelColor("claude-fable-5")).toBe(OPUS_RED);
    expect(getModelColor("fable-5")).toBe(OPUS_RED);
    expect(getModelColor("claude-fable-5")).toBe(getModelColor("claude-opus-4-6"));
  });

  it("renders Haiku green even when the id carries the claude- prefix", () => {
    expect(getModelColor("claude-haiku-4-5")).toBe(HAIKU_GREEN);
  });

  it("keeps Sonnet and bare claude ids amber", () => {
    expect(getModelColor("claude-sonnet-5")).toBe(CLAUDE_AMBER);
    expect(getModelColor("claude-4-5-sonnet-thinking")).toBe(CLAUDE_AMBER);
    expect(getModelColor("claude-3-5")).toBe(CLAUDE_AMBER);
  });

  it("falls back to gray for unknown models", () => {
    expect(getModelColor("fugu")).toBe(UNKNOWN_GRAY);
  });
});
