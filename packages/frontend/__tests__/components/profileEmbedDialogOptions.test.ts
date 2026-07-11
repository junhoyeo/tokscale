import { describe, expect, it } from "vitest";
import {
  buildProfileEmbedLinks,
  getEmbedDialogCapabilities,
  type EmbedDialogOptions,
} from "../../src/components/profile/embedDialogOptions";

const defaults: EmbedDialogOptions = {
  color: null,
  compact: false,
  costFormat: "compact",
  graph: false,
  rankFormat: "plain",
  sortBy: "tokens",
  template: "classic",
  theme: "dark",
  tokensFormat: "compact",
  view: "2d",
};

describe("profile embed dialog options", () => {
  it("keeps the default 2D URL minimal but explicit about number formats", () => {
    const links = buildProfileEmbedLinks("octocat", defaults);
    const url = new URL(links.embedUrl);

    expect(url.pathname).toBe("/api/embed/octocat/svg");
    expect(Object.fromEntries(url.searchParams)).toEqual({
      tokens: "compact",
      cost: "compact",
    });
    expect(links.markdownSnippet).toContain(links.embedUrl);
    expect(links.htmlSnippet).toContain("@octocat");
  });

  it("does not advertise or encode a classic graph in compact layout", () => {
    const options = { ...defaults, compact: true, graph: true };
    const url = new URL(buildProfileEmbedLinks("octocat", options).embedUrl);

    expect(getEmbedDialogCapabilities(options).showGraph).toBe(false);
    expect(url.searchParams.get("compact")).toBe("1");
    expect(url.searchParams.has("graph")).toBe(false);
  });

  it("encodes only options the 3D renderer consumes", () => {
    const options: EmbedDialogOptions = {
      ...defaults,
      color: "purple",
      compact: true,
      costFormat: "full",
      graph: true,
      rankFormat: "total",
      sortBy: "cost",
      template: "terminal",
      theme: "light",
      tokensFormat: "full",
      view: "3d",
    };
    const capabilities = getEmbedDialogCapabilities(options);
    const url = new URL(buildProfileEmbedLinks("octocat", options).embedUrl);

    expect(capabilities).toMatchObject({
      showAccent: false,
      showGraph: false,
      showLayout: true,
      showNumberFormats: false,
      showRankFormat: false,
      showTemplate: false,
    });
    expect(Object.fromEntries(url.searchParams)).toEqual({
      view: "3d",
      theme: "light",
      sort: "cost",
      compact: "1",
    });
  });

  it("keeps supported 2D template options in the generated URL", () => {
    const options: EmbedDialogOptions = {
      ...defaults,
      color: "purple",
      costFormat: "full",
      graph: true,
      rankFormat: "percent",
      sortBy: "cost",
      template: "minimal",
      theme: "light",
      tokensFormat: "full",
    };
    const url = new URL(buildProfileEmbedLinks("octocat", options).embedUrl);

    expect(getEmbedDialogCapabilities(options).showGraph).toBe(true);
    expect(Object.fromEntries(url.searchParams)).toEqual({
      theme: "light",
      sort: "cost",
      template: "minimal",
      color: "purple",
      graph: "1",
      rank: "percent",
      tokens: "full",
      cost: "full",
    });
  });

  it("does not show a redundant graph toggle for graph-first templates", () => {
    expect(
      getEmbedDialogCapabilities({
        ...defaults,
        template: "graph",
      }).showGraph,
    ).toBe(false);
    expect(
      getEmbedDialogCapabilities({
        ...defaults,
        template: "vitals",
      }).showGraph,
    ).toBe(false);
  });
});
