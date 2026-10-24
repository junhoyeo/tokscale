import { describe, expect, it } from "vitest";
import { canonicalModelId, isIdentifiedModelKey, modelKey, stripParenthesizedReasoningTier } from "@/lib/explore/modelKey";

// Mirrors test_normalize_model_for_grouping and
// test_normalize_syntactic_handles_non_ascii_names in
// crates/tokscale-core/src/lib.rs (alias config empty, so grouping ==
// canonical_model_id == normalize_syntactic).
describe("canonicalModelId (parity with tokscale-core normalize_syntactic)", () => {
  const cases: Array<[string, string]> = [
    ["claude-opus-4-5-20251101", "claude-opus-4-5"],
    ["claude-sonnet-4-5-20250929", "claude-sonnet-4-5"],
    ["claude-sonnet-4-20250514", "claude-sonnet-4"],
    ["claude-opus-4.5", "claude-opus-4-5"],
    ["claude-sonnet-4.5", "claude-sonnet-4-5"],
    ["claude-opus-4.6", "claude-opus-4-6"],
    ["anthropic/claude-4-6-sonnet", "claude-sonnet-4-6"],
    ["anthropic/claude-4-5-haiku", "claude-haiku-4-5"],
    ["anthropic/claude-4-6-opus", "claude-opus-4-6"],
    ["gpt-5.2", "gpt-5.2"],
    ["gpt-5.4(xhigh)", "gpt-5.4"],
    ["gpt-5.4(high)", "gpt-5.4"],
    ["gpt-5.4(minimal)", "gpt-5.4"],
    ["gpt-5.4(auto)", "gpt-5.4"],
    ["gpt-5.4(none)", "gpt-5.4"],
    ["gpt-5.4(weirdgarbage)", "gpt-5.4(weirdgarbage)"],
    ["claude-sonnet-4.5(high)", "claude-sonnet-4-5"],
    ["gemini-3-pro(auto)", "gemini-3-pro"],
    ["gemini-2.5-pro", "gemini-2.5-pro"],
    ["claude-opus-4-5-high", "claude-opus-4-5-high"],
    ["claude-opus-4-5-thinking-high", "claude-opus-4-5-thinking-high"],
    ["claude-sonnet-4-5-high", "claude-sonnet-4-5-high"],
    ["claude-4-sonnet", "claude-4-sonnet"],
    ["claude-4-opus-thinking", "claude-4-opus-thinking"],
    ["big-pickle", "big-pickle"],
    ["grok-code", "grok-code"],
    ["claude-opus-4.5-20251101", "claude-opus-4-5"],
    // test_normalize_syntactic_handles_non_ascii_names
    ["按需模型", "按需模型"],
    ["按需模型-20241022", "按需模型"],
    ["model-按需", "model-按需"],
    ["🚀-20241022", "🚀"],
    ["🚀-１２３４５６７８", "🚀-１２３４５６７８"],
    ["-12345678", "-12345678"],
  ];

  it.each(cases)("%s -> %s", (input, expected) => {
    expect(canonicalModelId(input)).toBe(expected);
  });

  it("is idempotent", () => {
    for (const [input] of cases) {
      const once = canonicalModelId(input);
      expect(canonicalModelId(once)).toBe(once);
    }
  });

  it("only strips the documented reasoning tiers", () => {
    expect(stripParenthesizedReasoningTier("gpt-5.4(xhigh)")).toBe("gpt-5.4");
    expect(stripParenthesizedReasoningTier("gpt-5.4 (high)")).toBeNull();
    expect(stripParenthesizedReasoningTier("(high)")).toBeNull();
    expect(stripParenthesizedReasoningTier("gpt-5.4(8192)")).toBeNull();
  });

  it("does not fold an anthropic prefix that is not major-minor-family", () => {
    expect(canonicalModelId("anthropic/claude-opus-4-7")).toBe("anthropic/claude-opus-4-7");
    expect(canonicalModelId("anthropic/claude-4-6-fable")).toBe("anthropic/claude-4-6-fable");
  });
});

describe("modelKey", () => {
  it("keeps canonical ids that are already URL-safe", () => {
    expect(modelKey("claude-opus-4-5-20251101")).toBe("claude-opus-4-5");
    expect(modelKey("gpt-5.6-sol")).toBe("gpt-5.6-sol");
  });

  it("drops routing prefixes so routed and direct usage share one row", () => {
    expect(modelKey("openai/gpt-5.5")).toBe("gpt-5.5");
    expect(modelKey("anthropic/claude-opus-4-7")).toBe("claude-opus-4-7");
    expect(modelKey("anthropic/claude-opus-5.5")).toBe("claude-opus-5-5");
    expect(modelKey("accounts/fireworks/routers/kimi-k2p5-turbo")).toBe("kimi-k2p5-turbo");
    expect(modelKey("@cf/moonshotai/kimi-k2.6")).toBe("kimi-k2.6");
  });

  it("replaces characters that are unsafe in a URL segment", () => {
    expect(modelKey("minimax/minimax-m3:free")).toBe("minimax-m3-free");
    expect(modelKey("qwen3:4b")).toBe("qwen3-4b");
    expect(modelKey("Claude Opus 5 (Auto Intelligence)")).toBe("claude-opus-5-auto-intelligence");
    expect(modelKey("gemma-4-e4b-it@q4_k_s")).toBe("gemma-4-e4b-it-q4_k_s");
  });

  it("never returns an empty key", () => {
    expect(modelKey("")).toBe("unknown");
    expect(modelKey("///")).toBe("unknown");
  });

  it("caps very long ids", () => {
    expect(modelKey("x".repeat(300)).length).toBeLessThanOrEqual(96);
  });

  it("marks placeholders and router choices as unidentified", () => {
    expect(isIdentifiedModelKey(modelKey("unknown"))).toBe(false);
    expect(isIdentifiedModelKey(modelKey("<synthetic>"))).toBe(false);
    expect(isIdentifiedModelKey(modelKey("openrouter/auto"))).toBe(false);
    expect(isIdentifiedModelKey(modelKey("Auto Balanced"))).toBe(false);
    expect(isIdentifiedModelKey(modelKey("claude-opus-5"))).toBe(true);
  });
});
