import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  after: vi.fn(),
  track: vi.fn(),
}));

vi.mock("next/server", () => ({ after: mocks.after }));
vi.mock("@vercel/analytics/server", () => ({ track: mocks.track }));

import { trackServerEvent } from "@/lib/analytics/server";

let originalVercel: string | undefined;
let pending: Promise<unknown> | undefined;

function callbackRequest(): Request {
  return new Request("https://tokscale.ai/api/auth/github/callback?code=oauth-code&state=csrf", {
    headers: {
      "user-agent": "Mozilla/5.0",
      "x-forwarded-for": "203.0.113.7",
      cookie: "tt_session=secret",
      authorization: "Bearer secret",
      referer: "https://tokscale.ai/device?code=ABCD-1234",
    },
  });
}

beforeEach(() => {
  originalVercel = process.env.VERCEL;
  delete process.env.VERCEL;
  pending = undefined;
  mocks.track.mockReset();
  mocks.after.mockReset();
  mocks.after.mockImplementation((callback: () => unknown) => {
    pending = Promise.resolve(callback());
  });
});

afterEach(() => {
  if (originalVercel === undefined) {
    delete process.env.VERCEL;
  } else {
    process.env.VERCEL = originalVercel;
  }
});

describe("trackServerEvent", () => {
  it("does nothing outside Vercel", () => {
    trackServerEvent("signed_up", callbackRequest());

    expect(mocks.after).not.toHaveBeenCalled();
    expect(mocks.track).not.toHaveBeenCalled();
  });

  it("forwards only the user agent, the client IP and the site origin", async () => {
    process.env.VERCEL = "1";
    mocks.track.mockResolvedValue(undefined);

    trackServerEvent("signed_up", callbackRequest());
    await pending;

    expect(mocks.track).toHaveBeenCalledExactlyOnceWith("signed_up", undefined, {
      headers: {
        "user-agent": "Mozilla/5.0",
        "x-forwarded-for": "203.0.113.7",
        referer: "https://tokscale.ai",
      },
    });
  });

  it("runs tracking after the response and swallows a rejection", async () => {
    process.env.VERCEL = "1";
    mocks.track.mockRejectedValue(new Error("analytics unavailable"));

    expect(() => trackServerEvent("group_created", callbackRequest())).not.toThrow();
    expect(mocks.after).toHaveBeenCalledOnce();
    await expect(pending).resolves.toBeUndefined();
  });
});
