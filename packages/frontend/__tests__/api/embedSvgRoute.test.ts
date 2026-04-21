import { NextRequest } from "next/server";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const getUserEmbedStats = vi.fn();
const getUserEmbedContributions = vi.fn();
const renderProfileEmbedErrorSvg = vi.fn();
const renderProfileEmbedSvg = vi.fn();
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

vi.mock("@/lib/embed/renderIsometric3DSvg", () => ({
  renderIsometric3DEmbedSvg,
  renderIsometric3DErrorSvg,
}));

vi.mock("@/lib/validation/username", () => ({
  isValidGitHubUsername,
}));

type ModuleExports = typeof import("../../src/app/api/embed/[username]/svg/route");

let GET: ModuleExports["GET"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/embed/[username]/svg/route");
  GET = routeModule.GET;
});

beforeEach(() => {
  getUserEmbedStats.mockReset();
  getUserEmbedContributions.mockReset();
  renderProfileEmbedErrorSvg.mockReset();
  renderProfileEmbedSvg.mockReset();
  renderIsometric3DEmbedSvg.mockReset();
  renderIsometric3DErrorSvg.mockReset();
  isValidGitHubUsername.mockReset();

  isValidGitHubUsername.mockReturnValue(true);
  renderIsometric3DEmbedSvg.mockReturnValue("<svg>3d</svg>");
});

describe("GET /api/embed/[username]/svg", () => {
  it("renders the 3D embed when contributions are empty", async () => {
    getUserEmbedStats.mockResolvedValue({
      user: {
        id: "user-1",
        username: "octocat",
        displayName: "The Octocat",
        avatarUrl: null,
      },
      stats: {
        totalTokens: 0,
        totalCost: 0,
        submissionCount: 0,
        rank: null,
        updatedAt: null,
      },
    });
    getUserEmbedContributions.mockResolvedValue([]);

    const response = await GET(
      new NextRequest("http://localhost:3000/api/embed/octocat/svg?view=3d"),
      { params: Promise.resolve({ username: "octocat" }) },
    );

    expect(response.status).toBe(200);
    expect(renderIsometric3DEmbedSvg).toHaveBeenCalledWith(
      expect.objectContaining({ user: expect.objectContaining({ username: "octocat" }) }),
      [],
      { theme: "dark", compact: false },
    );
    expect(renderIsometric3DErrorSvg).not.toHaveBeenCalled();
    await expect(response.text()).resolves.toBe("<svg>3d</svg>");
  });
});
