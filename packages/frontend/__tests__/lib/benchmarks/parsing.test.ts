import { describe, expect, it } from "vitest";
import { csvDate, csvNumber, parseCsv, parseCsvRecords, roundDerived } from "@/lib/benchmarks/csv";
import { agentClientKey, normalizeEffort, splitDisplayName, splitEpochVersion } from "@/lib/benchmarks/conditions";
import { candidateKeys, resolveModelKey, slugifyModelName, aliasRuleId, type AliasRule } from "@/lib/benchmarks/mapping";

describe("parseCsv", () => {
  it("handles quotes, doubled quotes, commas and newlines inside quotes", () => {
    const text = 'a,b,c\n1,"x, y","say ""hi""\nthere"\n2,,z\n';
    expect(parseCsv(text)).toEqual([
      ["a", "b", "c"],
      ["1", "x, y", 'say "hi"\nthere'],
      ["2", "", "z"],
    ]);
  });

  it("handles CRLF, a byte-order mark, blank lines and a missing final newline", () => {
    expect(parseCsv("\ufeffa,b\r\n\r\n1,2")).toEqual([
      ["a", "b"],
      ["1", "2"],
    ]);
  });

  it("maps rows to header names and fills missing cells with empty strings", () => {
    expect(parseCsvRecords("Model version,Score\nm1_max,0.5\nm2")).toEqual([
      { "Model version": "m1_max", Score: "0.5" },
      { "Model version": "m2", Score: "" },
    ]);
  });

  it("reads numbers and dates and rejects empty or invalid cells", () => {
    expect(csvNumber("0.25")).toBe(0.25);
    expect(csvNumber("")).toBeNull();
    expect(csvNumber("nan")).toBeNull();
    expect(csvNumber("abc")).toBeNull();
    expect(csvDate("2026-10-15T18:52:04.000Z")).toBe("2026-10-15");
    expect(csvDate("Oct 15")).toBeNull();
    expect(roundDerived(0.6476000000000001)).toBe(0.6476);
  });
});

describe("conditions", () => {
  it("normalizes effort labels from every source", () => {
    expect(normalizeEffort("Max")).toBe("max");
    expect(normalizeEffort("Extra High")).toBe("xhigh");
    expect(normalizeEffort("xHigh")).toBe("xhigh");
    expect(normalizeEffort("none")).toBe("none");
    expect(normalizeEffort("unknown")).toBeNull();
    expect(normalizeEffort("")).toBeNull();
    expect(normalizeEffort("toString")).toBeNull();
  });

  it("splits Epoch model versions into id and conditions", () => {
    expect(splitEpochVersion("claude-opus-5-5_max")).toEqual({ base: "claude-opus-5-5", effort: "max", thinkingBudget: null, effortUnknown: false });
    expect(splitEpochVersion("claude-sonnet-4-5_32K")).toMatchObject({ base: "claude-sonnet-4-5", effort: null, thinkingBudget: "32K" });
    expect(splitEpochVersion("gpt-6-astra_unknown")).toMatchObject({ base: "gpt-6-astra", effort: null, effortUnknown: true });
    expect(splitEpochVersion("muse-spark")).toMatchObject({ base: "muse-spark", effort: null });
    // A suffix that is not a condition stays part of the id.
    expect(splitEpochVersion("Qwen2.5-0-GGUF")).toMatchObject({ base: "Qwen2.5-0-GGUF" });
  });

  it("splits display names with condition groups and keeps unknown groups", () => {
    expect(splitDisplayName("Claude Opus 5.5 (Max)")).toMatchObject({ name: "Claude Opus 5.5", effort: "max", agent: null });
    expect(splitDisplayName("GPT-5.5 (xHigh, Codex Harness)")).toMatchObject({ name: "GPT-5.5", effort: "xhigh", agent: "Codex" });
    expect(splitDisplayName("Claude Opus 4.5 (High, 32K, 20251101)")).toMatchObject({
      name: "Claude Opus 4.5-20251101",
      effort: "high",
      thinkingBudget: "32K",
    });
    expect(splitDisplayName("Grok 4.1 (Thinking)")).toMatchObject({ name: "Grok 4.1", effort: null, qualifier: "thinking" });
    expect(splitDisplayName("Qwen3.8-Max (0902)")).toMatchObject({ name: "Qwen3.8-Max (0902)", effort: null });
    expect(splitDisplayName("Nemotron-3.5 (NVFP4)").name).toBe("Nemotron-3.5 (NVFP4)");
    expect(splitDisplayName("Kimi K3").name).toBe("Kimi K3");
  });

  it("maps agent labels to Tokscale client keys only when Tokscale tracks them", () => {
    expect(agentClientKey("Claude Code")).toBe("claude");
    expect(agentClientKey("Codex")).toBe("codex");
    expect(agentClientKey("Grok Build")).toBe("grok");
    expect(agentClientKey("mini-SWE-agent")).toBeNull();
    expect(agentClientKey(null)).toBeNull();
  });
});

describe("model mapping", () => {
  it("slugifies display names", () => {
    expect(slugifyModelName("GPT-6.1 Sol")).toBe("gpt-6.1-sol");
    expect(slugifyModelName("  Claude  Opus 5.5 ")).toBe("claude-opus-5.5");
  });

  it("builds candidates from ids before names, with exact and normalized methods", () => {
    expect(candidateKeys({ externalId: "x", ids: ["claude-opus-5-5"], names: ["Claude Opus 5.5"] })).toEqual([
      { key: "claude-opus-5-5", method: "exact" },
    ]);
    // Dated snapshots fold into the model.
    expect(candidateKeys({ externalId: "x", ids: ["gpt-5.4-2026-03-05"] }).map((c) => c.key)).toEqual(["gpt-5.4-2026-03-05", "gpt-5.4"]);
    // Plain ids come before host-prefixed ones.
    expect(candidateKeys({ externalId: "x", ids: ["fireworks/kimi-k2p5", "kimi-k2.5"] })[0]).toEqual({ key: "kimi-k2.5", method: "exact" });
  });

  it("adds the Claude prefix for Anthropic family names and joins brand and version", () => {
    const keys = candidateKeys({ externalId: "Opus 5.5", names: ["Opus 5.5"], org: "Anthropic" }).map((c) => c.key);
    expect(keys[0]).toBe("claude-opus-5-5");
    expect(candidateKeys({ externalId: "Qwen 3.8 Max", names: ["Qwen 3.8 Max"] }).map((c) => c.key)).toContain("qwen3.8-max");
    // No prefix without the Anthropic organization.
    expect(candidateKeys({ externalId: "Opus 5.5", names: ["Opus 5.5"] }).map((c) => c.key)).not.toContain("claude-opus-5-5");
  });

  it("only maps to keys that exist and never guesses", () => {
    const known = new Set(["claude-opus-5-5", "gpt-6.1-sol"]);
    const none = new Map<string, AliasRule>();
    expect(resolveModelKey("tbench", { externalId: "Opus 5.5", names: ["Opus 5.5"], org: "Anthropic" }, known, none)).toEqual({
      key: "claude-opus-5-5",
      method: "normalized",
      reviewed: false,
    });
    expect(resolveModelKey("epoch", { externalId: "gemini-4-argon", ids: ["gemini-4-argon"] }, known, none)).toEqual({
      key: null,
      method: null,
      reviewed: false,
    });
  });

  it("lets a manual alias win, including a reviewed no-match, and keeps reviewed flags", () => {
    const known = new Set(["claude-opus-5-5", "muse-wonder-1.0"]);
    const rules = new Map<string, AliasRule>([
      [aliasRuleId("lmarena", "Muse Wonder"), { source: "lmarena", externalId: "Muse Wonder", modelKey: "muse-wonder-1.0", method: "manual", reviewed: false }],
      [aliasRuleId("lmarena", "Claude Opus 5.5"), { source: "lmarena", externalId: "Claude Opus 5.5", modelKey: null, method: "manual", reviewed: true }],
      [aliasRuleId("epoch", "claude-opus-5-5"), { source: "epoch", externalId: "claude-opus-5-5", modelKey: "claude-opus-5-5", method: "exact", reviewed: true }],
    ]);
    expect(resolveModelKey("lmarena", { externalId: "Muse Wonder", names: ["Muse Wonder"] }, known, rules)).toEqual({
      key: "muse-wonder-1.0",
      method: "manual",
      reviewed: false,
    });
    expect(resolveModelKey("lmarena", { externalId: "Claude Opus 5.5", names: ["Claude Opus 5.5"] }, known, rules).key).toBeNull();
    expect(resolveModelKey("epoch", { externalId: "claude-opus-5-5", ids: ["claude-opus-5-5"] }, known, rules)).toEqual({
      key: "claude-opus-5-5",
      method: "exact",
      reviewed: true,
    });
  });
});
