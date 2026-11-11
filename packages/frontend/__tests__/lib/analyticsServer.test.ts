import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ after: vi.fn() }));

vi.mock("next/server", () => ({ after: mocks.after }));

import { trackServerEvent } from "@/lib/analytics/server";

const fetchMock = vi.fn();
let originalVercel: string | undefined;

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

function runDeferred(): Promise<unknown> {
  const [callback] = mocks.after.mock.calls[0] as [() => Promise<unknown>];
  return callback();
}

beforeEach(() => {
  originalVercel = process.env.VERCEL;
  delete process.env.VERCEL;
  mocks.after.mockReset();
  fetchMock.mockReset();
  fetchMock.mockResolvedValue(new Response(""));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
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
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("sends the event after the response, without the query, cookies or referer", async () => {
    process.env.VERCEL = "1";

    trackServerEvent("signed_up", callbackRequest());
    expect(fetchMock).not.toHaveBeenCalled();
    await runDeferred();

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("https://tokscale.ai/_vercel/insights/event");
    expect(init.method).toBe("POST");
    expect(init.headers).toEqual({
      "content-type": "application/json",
      "user-agent": "Mozilla/5.0",
      "x-vercel-ip": "203.0.113.7",
      "x-va-server": "1",
    });
    expect(JSON.parse(init.body as string)).toEqual({
      o: "https://tokscale.ai/api/auth/github/callback",
      ts: expect.any(Number),
      r: "",
      en: "signed_up",
    });
  });

  it("swallows a failed request", async () => {
    process.env.VERCEL = "1";
    fetchMock.mockRejectedValue(new Error("analytics unavailable"));

    trackServerEvent("group_created", callbackRequest());

    await expect(runDeferred()).resolves.toBeUndefined();
  });

  it("does not throw when the event cannot be scheduled", () => {
    process.env.VERCEL = "1";
    mocks.after.mockImplementation(() => {
      throw new Error("outside a request scope");
    });

    expect(() => trackServerEvent("cli_authorized", callbackRequest())).not.toThrow();
  });
});
