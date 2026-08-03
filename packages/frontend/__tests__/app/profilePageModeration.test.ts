import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const loadPublicProfileForPage = vi.fn();
const loadPublicProfileDevicesForPage = vi.fn();
const getSession = vi.fn();
const getModerationNotice = vi.fn();

vi.mock("next/navigation", () => ({
  notFound: vi.fn(),
  permanentRedirect: vi.fn(),
}));

vi.mock("@/lib/publicProfileData", () => ({ loadPublicProfileForPage }));
vi.mock("@/lib/publicProfileDevices", () => ({ loadPublicProfileDevicesForPage }));
vi.mock("@/lib/auth/session", () => ({ getSession }));
vi.mock("@/lib/moderation/notice", () => ({ getModerationNotice }));
vi.mock("@/lib/seo/urls", () => ({ profileUrl: vi.fn() }));
vi.mock("../../src/app/u/[username]/ProfilePageClient", () => ({
  default: () => null,
}));

type ModuleExports = typeof import("../../src/app/u/[username]/page");

let ProfilePage: ModuleExports["default"];

const profileData = {
  user: {
    id: "profile-owner",
    username: "owner",
    displayName: null,
    avatarUrl: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    rank: null,
  },
  stats: {
    totalTokens: 0,
    totalCost: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    submissionCount: 0,
    activeDays: 0,
    sessionCount: 0,
  },
  dateRange: { start: null, end: null },
  updatedAt: null,
  clients: [],
  models: [],
  contributions: [],
};

async function renderProfile() {
  return ProfilePage({
    params: Promise.resolve({ username: "owner" }),
    searchParams: Promise.resolve({}),
  });
}

beforeAll(async () => {
  ({ default: ProfilePage } = await import("../../src/app/u/[username]/page"));
});

beforeEach(() => {
  loadPublicProfileForPage.mockReset();
  loadPublicProfileDevicesForPage.mockReset();
  getSession.mockReset();
  getModerationNotice.mockReset();

  loadPublicProfileForPage.mockResolvedValue({ kind: "data", data: profileData });
  loadPublicProfileDevicesForPage.mockResolvedValue([]);
});

describe("profile moderation notice delivery", () => {
  it("forwards the notice only to the profile owner", async () => {
    const notice = {
      tone: "pending" as const,
      message: "Your account is currently withheld from the leaderboard pending review.",
    };
    getSession.mockResolvedValue({ id: "profile-owner" });
    getModerationNotice.mockResolvedValue(notice);

    const page = await renderProfile();

    expect(getModerationNotice).toHaveBeenCalledWith("profile-owner");
    expect(page.props.moderationNotice).toEqual(notice);
  });

  it("does not look up or forward a notice to anonymous visitors", async () => {
    getSession.mockResolvedValue(null);

    const page = await renderProfile();

    expect(getModerationNotice).not.toHaveBeenCalled();
    expect(page.props.moderationNotice).toBeNull();
  });

  it("does not look up or forward a notice to a different authenticated user", async () => {
    getSession.mockResolvedValue({ id: "another-user" });

    const page = await renderProfile();

    expect(getModerationNotice).not.toHaveBeenCalled();
    expect(page.props.moderationNotice).toBeNull();
  });

  it("preserves a moderation lookup failure for the page error boundary", async () => {
    getSession.mockResolvedValue({ id: "profile-owner" });
    getModerationNotice.mockRejectedValue(new Error("database unavailable"));

    await expect(renderProfile()).rejects.toThrow("database unavailable");
  });
});
