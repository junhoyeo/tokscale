import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const getModelDetail = vi.fn();
const getModelSummary = vi.fn();

vi.mock("next/navigation", () => ({ notFound: vi.fn() }));
vi.mock("@/lib/explore", () => ({ getModelDetail, getModelSummary }));
vi.mock("@/lib/benchmarks/usage", () => ({ getModelMixes: vi.fn() }));
vi.mock("@/components/layout/Navigation", () => ({ Navigation: () => null }));
vi.mock("@/components/layout/PublicFooter", () => ({ PublicFooter: () => null }));
vi.mock("@/components/explore/ModelDetailView", () => ({ ModelDetailView: () => null }));
vi.mock("@/components/explore/ModelNoUsageView", () => ({ ModelNoUsageView: () => null }));

const row = {
  key: "tokscale-test-model",
  name: "Tokscale Test Model",
  lab: "Test Lab",
  labSlug: null,
  logoUrl: null,
  rank: 3,
  activeUsers: 1234,
  previousActiveUsers: 1000,
  adoption: 0.2,
  tokens: 5_600_000_000,
  estimatedCost: 10,
  isNew: false,
  topAgents: [],
};

type ModuleExports = typeof import("../../src/app/(main)/models/[key]/page");

let generateMetadata: ModuleExports["generateMetadata"];

function metadataFor(key: string) {
  return generateMetadata({ params: Promise.resolve({ key }), searchParams: Promise.resolve({}) });
}

// The page pulls in the benchmark and catalog snapshots, which load slowly when the whole suite runs in parallel.
beforeAll(async () => {
  ({ generateMetadata } = await import("../../src/app/(main)/models/[key]/page"));
}, 60_000);

beforeEach(() => vi.clearAllMocks());

describe("model page metadata", () => {
  it("describes the model from its leaderboard row, without loading the detail", async () => {
    getModelSummary.mockResolvedValue(row);

    const metadata = await metadataFor(row.key);

    expect(metadata.title).toBe("Tokscale Test Model by Test Lab - Usage, Active Users, Est. Cost | Tokscale");
    expect(metadata.description).toContain("1,234 active users");
    expect(metadata.alternates?.canonical).toBe("https://tokscale.ai/models/tokscale-test-model");
    expect(getModelSummary.mock.calls).toEqual([[row.key, "30d"]]);
    expect(getModelDetail).not.toHaveBeenCalled();
  });

  it("falls back to all time when the model has no usage in the default period", async () => {
    getModelSummary.mockImplementation(async (_key: string, period: string) => (period === "all" ? row : null));

    const metadata = await metadataFor(row.key);

    expect(metadata.title).toContain("Tokscale Test Model");
    expect(getModelSummary.mock.calls).toEqual([[row.key, "30d"], [row.key, "all"]]);
  });

  it("reports a model without usage or research as not found", async () => {
    getModelSummary.mockResolvedValue(null);

    expect(await metadataFor(row.key)).toEqual({ title: "Model not found | Tokscale", robots: { index: false } });
  });
});
