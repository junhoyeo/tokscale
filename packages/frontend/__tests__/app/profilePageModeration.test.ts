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
  const notice = {
    tone: "pending" as const,
    title: "Withheld from the leaderboard",
    message: "This account is withheld from the leaderboard pending review.",
  };

  it("asks for the owner wording when the viewer owns the profile", async () => {
    getSession.mockResolvedValue({ id: "profile-owner" });
    getModerationNotice.mockResolvedValue(notice);

    const page = await renderProfile();

    expect(getModerationNotice).toHaveBeenCalledWith("profile-owner", "owner");
    expect(page.props.moderationNotice).toEqual(notice);
  });

  it("shows anonymous visitors the public wording", async () => {
    getSession.mockResolvedValue(null);
    getModerationNotice.mockResolvedValue(notice);

    const page = await renderProfile();

    expect(getModerationNotice).toHaveBeenCalledWith("profile-owner", "public");
    expect(page.props.moderationNotice).toEqual(notice);
  });

  it("shows a different authenticated user the public wording", async () => {
    getSession.mockResolvedValue({ id: "another-user" });
    getModerationNotice.mockResolvedValue(notice);

    const page = await renderProfile();

    expect(getModerationNotice).toHaveBeenCalledWith("profile-owner", "public");
    expect(page.props.moderationNotice).toEqual(notice);
  });

  it("falls back to the public wording when the session lookup fails", async () => {
    // Ownership could not be established, so the safe side is the wording that
    // does not address the reader as the account holder.
    getSession.mockRejectedValue(new Error("session store unavailable"));
    getModerationNotice.mockResolvedValue(notice);

    await renderProfile();

    expect(getModerationNotice).toHaveBeenCalledWith("profile-owner", "public");
  });

  it("forwards nothing when the account is not hidden", async () => {
    getSession.mockResolvedValue(null);
    getModerationNotice.mockResolvedValue(null);

    const page = await renderProfile();

    expect(page.props.moderationNotice).toBeNull();
  });

  it("preserves a moderation lookup failure for the owner's error boundary", async () => {
    getSession.mockResolvedValue({ id: "profile-owner" });
    getModerationNotice.mockRejectedValue(new Error("database unavailable"));

    await expect(renderProfile()).rejects.toThrow("database unavailable");
  });

  it("keeps a visitor's profile working when the moderation lookup fails", async () => {
    // This lookup now runs for every viewer, so letting it throw would turn one
    // unhealthy query into every profile page being down. A visitor loses the
    // banner and nothing else.
    getSession.mockResolvedValue({ id: "another-user" });
    getModerationNotice.mockRejectedValue(new Error("database unavailable"));

    const page = await renderProfile();

    expect(page.props.moderationNotice).toBeNull();
  });

  it("keeps an anonymous visitor's profile working when the moderation lookup fails", async () => {
    getSession.mockResolvedValue(null);
    getModerationNotice.mockRejectedValue(new Error("database unavailable"));

    const page = await renderProfile();

    expect(page.props.moderationNotice).toBeNull();
  });
});
