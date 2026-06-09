import { NextRequest } from "next/server";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const getUserEmbedStats = vi.fn();
const getUserEmbedContributions = vi.fn();
const renderProfileEmbedErrorSvg = vi.fn();
const renderProfileEmbedSvg = vi.fn();
const renderMinimalEmbedSvg = vi.fn();
const renderTerminalEmbedSvg = vi.fn();
const renderGraphEmbedSvg = vi.fn();
const renderIsometric3DEmbedSvg = vi.fn();
const renderIsometric3DErrorSvg = vi.fn();
const isValidGitHubUsername = vi.fn();

vi.mock("@/lib/embed/getUserEmbedStats", () => ({
  getUserEmbedStats,
  getUserEmbedContributions,
}));

vi.mock("@/lib/embed/renderProfileEmbedSvg", () => ({
  renderProfileEmbedErrorSvg,
  renderProfileEmbedSvg,
}));

vi.mock("@/lib/embed/renderMinimalEmbedSvg", () => ({ renderMinimalEmbedSvg }));
vi.mock("@/lib/embed/renderTerminalEmbedSvg", () => ({ renderTerminalEmbedSvg }));
vi.mock("@/lib/embed/renderGraphEmbedSvg", () => ({ renderGraphEmbedSvg }));

vi.mock("@/lib/embed/renderIsometric3DSvg", () => ({
  renderIsometric3DEmbedSvg,
  renderIsometric3DErrorSvg,
}));

vi.mock("@/lib/validation/username", () => ({
  isValidGitHubUsername,
}));

type ModuleExports = typeof import("../../src/app/api/embed/[username]/svg/route");

let GET: ModuleExports["GET"];

function statsFor(username: string) {
  return {
    user: { id: "user-1", username, displayName: "The Octocat", avatarUrl: null },
    stats: { totalTokens: 100, totalCost: 2, submissionCount: 1, rank: 5, updatedAt: null },
  };
}

function request(query: string) {
  return GET(
    new NextRequest(`http://localhost:3000/api/embed/octocat/svg${query}`),
    { params: Promise.resolve({ username: "octocat" }) },
  );
}

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/embed/[username]/svg/route");
  GET = routeModule.GET;
});

beforeEach(() => {
  getUserEmbedStats.mockReset();
  getUserEmbedContributions.mockReset();
  renderProfileEmbedErrorSvg.mockReset();
  renderProfileEmbedSvg.mockReset();
  renderMinimalEmbedSvg.mockReset();
  renderTerminalEmbedSvg.mockReset();
  renderGraphEmbedSvg.mockReset();
  renderIsometric3DEmbedSvg.mockReset();
  renderIsometric3DErrorSvg.mockReset();
  isValidGitHubUsername.mockReset();

  isValidGitHubUsername.mockReturnValue(true);
  renderIsometric3DEmbedSvg.mockReturnValue("<svg>3d</svg>");
  renderProfileEmbedSvg.mockReturnValue("<svg>classic</svg>");
  renderMinimalEmbedSvg.mockReturnValue("<svg>minimal</svg>");
  renderTerminalEmbedSvg.mockReturnValue("<svg>terminal</svg>");
  renderGraphEmbedSvg.mockReturnValue("<svg>graph</svg>");
  getUserEmbedContributions.mockResolvedValue([]);
});

describe("GET /api/embed/[username]/svg", () => {
  it("redirects mixed-case requests to the canonical embed path", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("OctoCat"));

    const response = await GET(
      new NextRequest("http://localhost:3000/api/embed/octocat/svg?view=3d&theme=light"),
      { params: Promise.resolve({ username: "octocat" }) },
    );

    expect(response.status).toBe(308);
    expect(response.headers.get("location")).toBe(
      "http://localhost:3000/api/embed/OctoCat/svg?view=3d&theme=light",
    );
    expect(getUserEmbedContributions).not.toHaveBeenCalled();
    expect(renderIsometric3DEmbedSvg).not.toHaveBeenCalled();
  });

  it("renders the 3D embed when contributions are empty", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("octocat"));
    getUserEmbedContributions.mockResolvedValue([]);

    const response = await request("?view=3d");

    expect(response.status).toBe(200);
    expect(renderIsometric3DEmbedSvg).toHaveBeenCalledWith(
      expect.objectContaining({ user: expect.objectContaining({ username: "octocat" }) }),
      [],
      { theme: "dark", compact: false },
    );
    expect(renderIsometric3DErrorSvg).not.toHaveBeenCalled();
    await expect(response.text()).resolves.toBe("<svg>3d</svg>");
  });

  it("renders the classic card by default", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("octocat"));

    const response = await request("");

    expect(response.status).toBe(200);
    expect(renderProfileEmbedSvg).toHaveBeenCalled();
    expect(renderMinimalEmbedSvg).not.toHaveBeenCalled();
    await expect(response.text()).resolves.toBe("<svg>classic</svg>");
  });

  it("dispatches to the minimal template", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("octocat"));

    const response = await request("?template=minimal");

    expect(renderMinimalEmbedSvg).toHaveBeenCalled();
    expect(renderProfileEmbedSvg).not.toHaveBeenCalled();
    await expect(response.text()).resolves.toBe("<svg>minimal</svg>");
  });

  it("dispatches to the terminal template", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("octocat"));

    const response = await request("?template=terminal");

    expect(renderTerminalEmbedSvg).toHaveBeenCalled();
    await expect(response.text()).resolves.toBe("<svg>terminal</svg>");
  });

  it("dispatches to the graph template", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("octocat"));

    const response = await request("?template=graph");

    expect(renderGraphEmbedSvg).toHaveBeenCalled();
    await expect(response.text()).resolves.toBe("<svg>graph</svg>");
  });

  it("passes color and per-metric number formats to the template renderer", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("octocat"));

    await request("?template=graph&color=purple&tokens=full&cost=compact");

    expect(renderGraphEmbedSvg).toHaveBeenCalledWith(
      expect.objectContaining({ user: expect.objectContaining({ username: "octocat" }) }),
      expect.objectContaining({ color: "purple", tokensFormat: "full", costFormat: "compact" }),
    );
  });

  it("ignores an unknown color and falls back to the classic template", async () => {
    getUserEmbedStats.mockResolvedValue(statsFor("octocat"));

    await request("?template=bogus&color=bogus");

    expect(renderProfileEmbedSvg).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({ color: null }),
    );
  });

  it.each(["orbit", "vitals", "blueprint", "receipt"])(
    "renders the %s template end to end",
    async (tpl) => {
      getUserEmbedStats.mockResolvedValue(statsFor("octocat"));

      const response = await request(`?template=${tpl}`);

      expect(response.status).toBe(200);
      expect(response.headers.get("content-type")).toContain("image/svg+xml");
      await expect(response.text()).resolves.toContain("<svg");
    },
  );

});
