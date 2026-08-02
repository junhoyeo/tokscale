import { describe, expect, it } from "vitest";
import {
  escapeLikePattern,
  hasDirectives,
  parseSearchDirectives,
} from "@/lib/leaderboard/searchDirectives";

describe("parseSearchDirectives", () => {
  it("returns empty directives and original text when no directives present", () => {
    const result = parseSearchDirectives("junhoyeo");
    expect(result).toEqual({ text: "junhoyeo", clients: [], models: [] });
  });

  it("extracts a single client directive", () => {
    const result = parseSearchDirectives("client:opencode");
    expect(result).toEqual({ text: "", clients: ["opencode"], models: [] });
  });

  it("extracts a single model directive", () => {
    const result = parseSearchDirectives("model:claude-sonnet-4");
    expect(result).toEqual({ text: "", clients: [], models: ["claude-sonnet-4"] });
  });

  it("extracts multiple client directives", () => {
    const result = parseSearchDirectives("client:opencode client:claude");
    expect(result).toEqual({ text: "", clients: ["opencode", "claude"], models: [] });
  });

  it("extracts mixed directives with free text", () => {
    const result = parseSearchDirectives("client:opencode junhoyeo model:gpt-5");
    expect(result).toEqual({
      text: "junhoyeo",
      clients: ["opencode"],
      models: ["gpt-5"],
    });
  });

  it("lowercases directive values", () => {
    const result = parseSearchDirectives("client:OpenCode model:Claude-Sonnet-4");
    expect(result).toEqual({
      text: "",
      clients: ["opencode"],
      models: ["claude-sonnet-4"],
    });
  });

  it("handles directives case-insensitively", () => {
    const result = parseSearchDirectives("Client:amp MODEL:gpt-5");
    expect(result).toEqual({
      text: "",
      clients: ["amp"],
      models: ["gpt-5"],
    });
  });

  it("trims extra whitespace from remaining text", () => {
    const result = parseSearchDirectives("  client:opencode   some user   model:gpt-5  ");
    expect(result).toEqual({
      text: "some user",
      clients: ["opencode"],
      models: ["gpt-5"],
    });
  });

  it("returns empty text for empty input", () => {
    const result = parseSearchDirectives("");
    expect(result).toEqual({ text: "", clients: [], models: [] });
  });

  it("handles directive at end of string", () => {
    const result = parseSearchDirectives("someuser client:claude");
    expect(result).toEqual({
      text: "someuser",
      clients: ["claude"],
      models: [],
    });
  });

  it("does not capture trailing punctuation in directive values", () => {
    const result = parseSearchDirectives("client:opencode, model:gpt-5.");
    expect(result).toEqual({
      text: ",",
      clients: ["opencode"],
      models: ["gpt-5"],
    });
  });

  it("preserves dots within model names", () => {
    const result = parseSearchDirectives("model:claude-3.5-sonnet");
    expect(result).toEqual({
      text: "",
      clients: [],
      models: ["claude-3.5-sonnet"],
    });
  });
});

describe("hasDirectives", () => {
  it("returns false when no directives", () => {
    expect(hasDirectives({ text: "foo", clients: [], models: [] })).toBe(false);
  });

  it("returns true when clients present", () => {
    expect(hasDirectives({ text: "", clients: ["opencode"], models: [] })).toBe(true);
  });

  it("returns true when models present", () => {
    expect(hasDirectives({ text: "", clients: [], models: ["gpt-5"] })).toBe(true);
  });

  it("returns true when both present", () => {
    expect(hasDirectives({ text: "", clients: ["claude"], models: ["gpt-5"] })).toBe(true);
  });
});

describe("escapeLikePattern", () => {
  it("escapes SQL LIKE wildcard and escape characters", () => {
    expect(escapeLikePattern("gpt_5%\\")).toBe("gpt\\_5\\%\\\\");
  });
});
