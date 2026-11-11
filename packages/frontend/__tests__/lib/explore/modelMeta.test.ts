import { describe, expect, it } from "vitest";
import { agentRef, labForModel, modelDisplayName, modelLogoUrl, modelRef } from "@/lib/explore/modelMeta";

describe("labForModel", () => {
  it.each([
    ["claude-opus-5", "Anthropic", "anthropic"],
    ["gpt-5.6-sol", "OpenAI", "openai"],
    ["o3", "OpenAI", "openai"],
    ["o4-mini", "OpenAI", "openai"],
    ["codex-mini-latest", "OpenAI", "openai"],
    ["gemini-3-pro", "Google", "google"],
    ["grok-4.6", "xAI", "xai"],
    ["kimi-k3", "Moonshot AI", "moonshot-ai"],
    ["qwen3.6-max-preview", "Alibaba", "alibaba"],
    ["deepseek-v4-pro", "DeepSeek", "deepseek"],
    ["glm-5.3-flash", "Z.ai", "zai"],
    ["mistral-large-3", "Mistral", "mistral"],
    ["devstral-2", "Mistral", "mistral"],
    ["codestral-latest", "Mistral", "mistral"],
    ["minimax-m2.7", "MiniMax", "minimax"],
    ["antigravity-gemini-3.8-flash", "Google", "google"],
    ["cursor-grok-4.6-xhigh", "xAI", "xai"],
    ["zai-glm-4.7", "Z.ai", "zai"],
    ["openai.gpt-oss-20b-1-0", "OpenAI", "openai"],
    ["gemma-4-27b-it", "Google", "google"],
    ["llama3.1-8b", "Meta", "meta"],
    ["nemotron-3-super-free", "NVIDIA", "nvidia"],
    ["nvidia-nemotron-3-super-120b-a12b-nvfp4", "NVIDIA", "nvidia"],
    ["mimo-v2.5-pro", "Xiaomi", "xiaomi"],
    ["composer-2.5-fast", "Cursor", "cursor"],
    ["step-3.5-flash-free", "StepFun", "stepfun"],
    ["longcat-2.5-preview-free", "Meituan", "meituan"],
    ["solar-pro4", "Upstage", "upstage"],
    ["seed-2.0-lite", "ByteDance", "bytedance"],
    ["seed-oss-36b-instruct", "ByteDance", "bytedance"],
    ["kat-coder-pro", "Kwaipilot", "kwaipilot"],
    ["mercury-2", "Inception", "inception"],
    ["trinity-large-preview-free", "Arcee AI", "arcee-ai"],
    ["muse-spark-1.3", "Meta", "meta"],
    ["muse-spark-1.3-contributor-free", "Meta", "meta"],
    ["muse-wonder-1.0", "Meta", "meta"],
  ])("%s -> %s", (key, name, slug) => {
    expect(labForModel(key)).toEqual({ name, slug });
  });

  it.each(["k3", "big-pickle", "muse-code", "museum-1", "unknown", "openclaw", "glamour-1", "grokky", "stepper-1", "seedling", "composer"])("does not guess a lab for %s", (key) => {
    expect(labForModel(key)).toBeNull();
  });
});

describe("modelDisplayName", () => {
  it.each([
    ["claude-opus-4-5", "Claude Opus 4.5"],
    ["claude-opus-5", "Claude Opus 5"],
    ["claude-fable-5-1", "Claude Fable 5.1"],
    ["claude-opus-4-5-thinking-high", "Claude Opus 4.5 Thinking High"],
    ["claude-3-5-sonnet", "Claude 3.5 Sonnet"],
    ["gpt-5.6-sol", "GPT-5.6 Sol"],
    ["gpt-5-codex", "GPT-5 Codex"],
    ["gpt-oss-20b", "GPT OSS 20B"],
    ["o4-mini", "o4-mini"],
    ["gemini-3-pro", "Gemini 3 Pro"],
    ["gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"],
    ["kimi-k3", "Kimi K3"],
    ["qwen-4-coder", "Qwen 4 Coder"],
    ["qwen3.6-max-preview", "Qwen3.6 Max Preview"],
    ["qwq-32b", "QwQ 32B"],
    ["deepseek-v4-pro", "DeepSeek V4 Pro"],
    ["glm-5.3-flash", "GLM-5.3 Flash"],
    ["minimax-m2.7", "MiniMax M2.7"],
    ["grok-4.6", "Grok 4.6"],
    ["codex-mini-latest", "Codex Mini Latest"],
    ["big-pickle", "Big Pickle"],
    ["antigravity-claude-sonnet-4-6-thinking", "Antigravity Claude Sonnet 4.6 Thinking"],
    ["gpt-5.2-xhigh", "GPT-5.2 XHigh"],
  ])("%s -> %s", (key, name) => {
    expect(modelDisplayName(key)).toBe(name);
  });

  it("builds a full ModelRef", () => {
    expect(modelRef("claude-sonnet-5")).toEqual({
      key: "claude-sonnet-5",
      name: "Claude Sonnet 5",
      lab: "Anthropic",
      labSlug: "anthropic",
      logoUrl: "/assets/v2/models/claude.svg",
    });
    expect(modelRef("k3")).toEqual({ key: "k3", name: "K3", lab: null, labSlug: null, logoUrl: null });
  });
});

describe("agentRef", () => {
  it("uses the shared client names and logos", () => {
    const claude = agentRef("claude");
    expect(claude.name).toBe("Claude Code");
    expect(claude.logoUrl).toBe("/assets/v2/agents/client-claude.webp");
  });

  it("folds legacy client aliases", () => {
    expect(agentRef("kilocode").key).toBe("kilo");
  });

  it("keeps unknown clients without guessing", () => {
    expect(agentRef("some-new-agent")).toEqual({ key: "some-new-agent", name: "some-new-agent", logoUrl: null });
    expect(agentRef("constructor")).toEqual({ key: "constructor", name: "constructor", logoUrl: null });
  });
});

describe("modelLogoUrl", () => {
  it.each([
    ["claude-opus-5-5", "claude"],
    ["gpt-6.1-sol", "openai"],
    ["o4-mini", "openai"],
    ["gemini-3-pro", "gemini"],
    ["gemma-4-27b-it", "gemma"],
    ["antigravity-gemini-3.8-flash", "gemini"],
    ["cursor-grok-4.6-xhigh", "grok"],
    ["glm-5.3-flash", "zai"],
    ["devstral-2512-free", "mistral"],
    ["mimo-v2.5", "xiaomimimo"],
  ])("%s -> %s", (key, logo) => {
    expect(modelLogoUrl(key)).toBe(`/assets/v2/models/${logo}.svg`);
  });

  it("returns null for an unknown family", () => {
    expect(modelLogoUrl("big-pickle")).toBeNull();
  });

  it("gives Meta's Muse models the Meta mark", () => {
    expect(modelLogoUrl("muse-spark-1.3-max")).toBe("/assets/v2/models/meta.svg");
  });

  it("is carried on modelRef", () => {
    expect(modelRef("qwen3.6-max-preview").logoUrl).toBe("/assets/v2/models/qwen.svg");
    expect(modelRef("big-pickle").logoUrl).toBeNull();
  });
});
