/** @vitest-environment jsdom */

import React, { act } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import GroupDetailClient from "@/app/(main)/groups/[slug]/GroupDetailClient";
import { GroupManagement } from "@/components/leaderboard/GroupManagement";

vi.mock("next/link", () => ({
  default: ({ children, href, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) => (
    <a href={typeof href === "string" ? href : ""} {...props}>{children}</a>
  ),
}));

vi.mock("next/image", () => ({
  // The tests do not render remote avatars; keep the framework component inert.
  default: ({ alt = "", ...props }: React.ImgHTMLAttributes<HTMLImageElement>) => (
    // eslint-disable-next-line @next/next/no-img-element
    <img alt={alt} {...props} />
  ),
}));

vi.mock("nextjs-toploader/app", () => ({
  useRouter: () => ({ push: vi.fn() }),
}));

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const ownerGroup = {
  id: "group-1",
  name: "Team",
  slug: "team",
  description: null,
  avatarUrl: null,
  isPublic: false,
  timeZone: "UTC",
  weekStartsOn: 1,
  memberCount: 2,
  membership: { role: "owner" as const },
};

const initialData = {
  users: [],
  pagination: { page: 1, limit: 50, totalUsers: 0, totalPages: 0, hasNext: false, hasPrev: false },
  stats: { totalTokens: 10, totalCost: 1, activeUsers: 1, totalMembers: 2 },
  period: "all" as const,
  sortBy: "tokens" as const,
  dateRange: null,
  timeZone: "UTC",
  weekStartsOn: 1,
};

const owner = {
  id: "owner-1",
  username: "owner",
  displayName: "Owner",
  avatarUrl: null,
};

const initialMembers = [
  { userId: "owner-1", username: "owner", displayName: "Owner", avatarUrl: null, role: "owner" },
  { userId: "member-1", username: "member", displayName: "Member", avatarUrl: null, role: "member" },
  { userId: "member-2", username: "teammate", displayName: "Teammate", avatarUrl: null, role: "member" },
];

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value() { this.setAttribute("open", ""); },
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value() { this.removeAttribute("open"); this.dispatchEvent(new Event("close")); },
  });
  Object.defineProperty(HTMLElement.prototype, "hidePopover", {
    configurable: true,
    value: vi.fn(),
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function openMemberActions(username: string) {
  fireEvent.click(screen.getByRole("button", { name: `Manage @${username}` }));
  const actions = screen.getByRole("group", { name: `Actions for @${username}`, hidden: true });
  actions.removeAttribute("popover");
}

describe("group member management client flow", () => {
  it("submits transfer once and ignores an older leaderboard response", async () => {
    const firstLeaderboard = deferred<Response>();
    const refreshedLeaderboard = deferred<Response>();
    const transfer = deferred<Response>();
    let leaderboardCalls = 0;
    let memberCalls = 0;
    let transferCalls = 0;

    const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/invite") && (!init?.method || init.method === "GET")) return Promise.resolve(jsonResponse({ invites: [] }));
      if (url.endsWith("/api/groups/team/members") && (!init?.method || init.method === "GET")) {
        memberCalls += 1;
        return Promise.resolve(jsonResponse({ members: memberCalls === 1
          ? initialMembers
          : [
              { ...initialMembers[0], role: "admin" },
              { ...initialMembers[1], role: "owner" },
              initialMembers[2],
            ] }));
      }
      if (url.includes("/api/groups/team/leaderboard?")) {
        leaderboardCalls += 1;
        return leaderboardCalls === 1 ? firstLeaderboard.promise : refreshedLeaderboard.promise;
      }
      if (url.endsWith("/api/groups/team/transfer-ownership")) {
        transferCalls += 1;
        return transfer.promise;
      }
      if (url.endsWith("/api/groups/team")) {
        return Promise.resolve(jsonResponse({
          ...ownerGroup,
          membership: { role: "admin" },
        }));
      }
      throw new Error(`Unexpected fetch: ${url} ${init?.method || "GET"}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    render(<GroupDetailClient group={ownerGroup} currentUser={owner} initialData={initialData} />);
    await screen.findByRole("button", { name: "Manage @member" });

    fireEvent.click(screen.getByRole("button", { name: "This month" }));
    await waitFor(() => expect(leaderboardCalls).toBe(1));

    openMemberActions("member");
    fireEvent.click(screen.getByRole("button", { name: "Transfer ownership to @member" }));
    const confirm = await screen.findByRole("button", { name: "Confirm" });
    act(() => {
      confirm.click();
      confirm.click();
    });
    expect(transferCalls).toBe(1);

    await act(async () => { transfer.resolve(jsonResponse({ members: [] })); });
    await waitFor(() => expect(leaderboardCalls).toBe(2));

    await act(async () => {
      refreshedLeaderboard.resolve(jsonResponse({
        ...initialData,
        period: "month",
        stats: { ...initialData.stats, totalTokens: 222 },
      }));
    });
    await screen.findByText("222");

    let staleResponseRead = false;
    const staleResponse = jsonResponse(null);
    staleResponse.json = vi.fn(async () => {
      staleResponseRead = true;
      return {
        ...initialData,
        period: "month",
        stats: { ...initialData.stats, totalTokens: 111 },
      };
    });
    await act(async () => { firstLeaderboard.resolve(staleResponse); });
    await waitFor(() => expect(staleResponseRead).toBe(true));
    await act(async () => { await Promise.resolve(); });

    expect(screen.queryByText("111")).toBeNull();
    expect(screen.getByText("222")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Manage @teammate" })).toBeTruthy();
    const inviteRole = screen.getByRole("combobox", { name: "Invite role" });
    expect(inviteRole.querySelector('option[value="admin"]')).toBeNull();
    expect(screen.queryByRole("button", { name: /Transfer ownership to @/, hidden: true })).toBeNull();
  });

  it("keeps a failed removal confirmation open and allows retry", async () => {
    let deleteCalls = 0;
    let memberCalls = 0;
    const onChanged = vi.fn(async () => {});
    const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/invite") && (!init?.method || init.method === "GET")) return Promise.resolve(jsonResponse({ invites: [] }));
      if (url.includes("/api/groups/team/members?userId=") && init?.method === "DELETE") {
        deleteCalls += 1;
        return Promise.resolve(deleteCalls === 1
          ? jsonResponse({ error: "Temporary failure" }, 500)
          : jsonResponse({ success: true }));
      }
      if (url.endsWith("/api/groups/team/members") && (!init?.method || init.method === "GET")) {
        memberCalls += 1;
        return Promise.resolve(jsonResponse({
          members: memberCalls === 1 ? initialMembers : [initialMembers[0]],
        }));
      }
      throw new Error(`Unexpected fetch: ${url} ${init?.method || "GET"}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    render(
      <GroupManagement
        slug="team"
        userId="owner-1"
        role="owner"
        onChanged={onChanged}
      />
    );
    await screen.findByRole("button", { name: "Manage @member" });

    openMemberActions("member");
    fireEvent.click(screen.getByRole("button", { name: "Remove @member" }));
    fireEvent.click(await screen.findByRole("button", { name: "Confirm" }));

    expect(await screen.findAllByText("Temporary failure")).toHaveLength(2);
    expect(screen.getByRole("dialog")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    await waitFor(() => expect(deleteCalls).toBe(2));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(screen.queryByRole("button", { name: "Manage @member" })).toBeNull());
    expect(memberCalls).toBe(2);
    expect(onChanged).toHaveBeenCalledTimes(1);
  });

});


it("creates a reusable invite once, shows usage, and preserves the link on failed revocation", async () => {
  const creation = deferred<Response>();
  let invite: Record<string, unknown> | null = null;
  let posts = 0;
  let deletes = 0;
  vi.stubGlobal("fetch", vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith("/members")) return Promise.resolve(jsonResponse({ members: initialMembers }));
    if (url.endsWith("/invite")) {
      if (init?.method === "POST") {
        posts++;
        expect(JSON.parse(String(init.body))).toMatchObject({ maxUses: 5, expiresInDays: 7, invitedUsername: null });
        return creation.promise;
      }
      if (init?.method === "DELETE") {
        deletes++;
        expect(JSON.parse(String(init.body))).toEqual({ inviteId: "invite-1" });
        if (deletes === 1) return Promise.resolve(jsonResponse({ error: "Try again" }, 500));
        invite = { ...invite, status: "revoked" };
        return Promise.resolve(jsonResponse({ revoked: true }));
      }
      return Promise.resolve(jsonResponse({ invites: invite ? [invite] : [] }));
    }
    throw new Error(`Unexpected request ${url}`);
  }));
  render(<GroupDetailClient group={ownerGroup} currentUser={owner} initialData={initialData} />);
  await screen.findByRole("button", { name: "Manage @member" });
  fireEvent.change(screen.getByRole("spinbutton", { name: "Maximum joins" }), { target: { value: "5" } });
  fireEvent.change(screen.getByRole("spinbutton", { name: "Expires in days" }), { target: { value: "7" } });
  const createButton = screen.getByRole("button", { name: "Create invite" });
  fireEvent.click(createButton);
  fireEvent.click(createButton);
  expect(posts).toBe(1);
  invite = { id: "invite-1", role: "member", invitedUsername: null, maxUses: 5, useCount: 1, status: "pending", expiresAt: "2099-01-01T00:00:00Z" };
  await act(async () => creation.resolve(jsonResponse({ invite, joinUrl: "/groups/join/local-token" })));
  expect(await screen.findByText(/1\/5 joined/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Try again");
  expect(screen.getByRole("button", { name: "Copy invite link" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Revoke" })).toBeNull());
  expect(screen.queryByRole("button", { name: "Copy invite link" })).toBeNull();
  expect(screen.getByText(/revoked/)).toBeTruthy();
  fireEvent.change(screen.getByRole("textbox", { name: "Invitee GitHub username" }), { target: { value: "alice" } });
  const limit = screen.getByRole("spinbutton", { name: "Maximum joins" }) as HTMLInputElement;
  expect(limit.disabled).toBe(true);
  expect(limit.value).toBe("1");
});
