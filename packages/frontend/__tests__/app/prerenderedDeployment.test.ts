import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const connection = vi.fn(async () => undefined);
const getCommunitySnapshot = vi.fn();
const getFeaturedHuman = vi.fn();
const getModelLeaderboard = vi.fn();
const getHomeCapital = vi.fn();

const font = () => ({ variable: "font", className: "font" });
vi.mock("next/server", () => ({ connection }));
vi.mock("next/font/google", () => ({ JetBrains_Mono: font, Figtree: font, Instrument_Serif: font }));
vi.mock("nextjs-toploader", () => ({ default: () => null }));
vi.mock("react-toastify", () => ({ ToastContainer: () => null }));
vi.mock("@/components/analytics/WebAnalytics", () => ({ WebAnalytics: () => null }));
vi.mock("@/lib/providers", () => ({ Providers: () => null }));
vi.mock("@/components/layout/Navigation", () => ({ Navigation: () => null }));
vi.mock("@/components/home/HomePage", () => ({ HomePage: () => null }));
vi.mock("@/lib/explore", () => ({ getCommunitySnapshot, getFeaturedHuman, getModelLeaderboard }));
vi.mock("@/lib/capital/ledger/home", () => ({ getHomeCapital }));

const coverage = { start: "2026-10-13", end: "2026-11-11" };

beforeEach(() => {
  vi.clearAllMocks();
  getCommunitySnapshot.mockResolvedValue({ humans: [], models: [], agents: [], coverage });
  getFeaturedHuman.mockResolvedValue({ human: null, coverage });
  getModelLeaderboard.mockResolvedValue({ rows: [], coverage });
  getHomeCapital.mockResolvedValue(null);
});

afterEach(() => vi.unstubAllEnvs());

describe("root layout metadata", () => {
  it("waits for a request where the public origin is only known at runtime", async () => {
    vi.stubEnv("VERCEL", "");
    const { generateMetadata } = await import("../../src/app/layout");
    await generateMetadata();
    expect(connection).toHaveBeenCalledTimes(1);
  });

  it("does not force every route to render per request on a prerendered deployment", async () => {
    vi.stubEnv("VERCEL", "1");
    const { generateMetadata } = await import("../../src/app/layout");
    const metadata = await generateMetadata();
    expect(connection).not.toHaveBeenCalled();
    expect(metadata.metadataBase?.toString()).toBe("https://tokscale.ai/");
  });
});

describe("home page", () => {
  async function renderHome() {
    const { default: Page } = await import("../../src/app/(main)/page");
    const tree = (await Page()) as ReactElement<{ children: ReactElement<Record<string, unknown>>[] }>;
    return tree.props.children[1].props;
  }

  it("renders without a section that failed to load on a per-request render", async () => {
    vi.stubEnv("VERCEL", "");
    getCommunitySnapshot.mockRejectedValue(new Error("database unavailable"));
    const props = await renderHome();
    expect(props.snapshot).toBeNull();
    expect(props.modelsCoverage).toEqual(coverage);
  });

  it("fails the rebuild of a prerendered page instead of publishing it without a section", async () => {
    vi.stubEnv("VERCEL", "1");
    getCommunitySnapshot.mockRejectedValue(new Error("database unavailable"));
    await expect(renderHome()).rejects.toThrow("database unavailable");
  });
});
