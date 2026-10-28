"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";
import styled from "styled-components";
import {
  GroupMark,
  SecondaryButton,
  SegmentedControl,
} from "@/components/leaderboard/RankingUI";
import { formatGroupMemberCount } from "@/components/leaderboard/presentation";

// The groups directory under the /leaderboard ?view=groups view. The
// /groups/[slug], /groups/new, and /groups/join/[token] subpages are separate
// routes; this list is the directory, one row per group on the page ground
// (no cards), the whole row a link to the group.

type GroupRole = "owner" | "admin" | "member";

interface SessionUser {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
}

export interface GroupCardData {
  id: string;
  name: string;
  slug: string;
  description: string | null;
  avatarUrl: string | null;
  isPublic: boolean;
  memberCount: number;
  role?: GroupRole;
}

interface GroupPagination {
  page: number;
  limit: number;
  total: number;
  totalPages: number;
  hasNext: boolean;
  hasPrev: boolean;
}

interface GroupsBrowserProps {
  currentUser: SessionUser | null;
  initialPublicGroups: GroupCardData[];
  initialMyGroups: GroupCardData[];
  initialPublicPagination: GroupPagination;
  initialMyPagination: GroupPagination;
}

type ActiveTab = "public" | "mine";

const Toolbar = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px 24px;
  margin-bottom: 14px;
`;

const ToolbarLeft = styled.div`
  display: flex;
  flex: 0 0 auto;
  align-items: center;
`;

const DirectoryStatus = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const Table = styled.ul`
  margin: 0;
  padding: 0;
  list-style: none;
  border-bottom: 1px solid var(--service-border);
`;

const Row = styled.li`
  position: relative;
  border-top: 1px solid var(--service-border);
`;

/* The row is one link: the name anchors it and its ::after covers the row. */
const RowLink = styled(Link)`
  display: grid;
  grid-template-columns: 42px minmax(0, 1fr) 96px 104px 88px;
  column-gap: 16px;
  align-items: center;
  padding: 12px 0;
  color: inherit;
  text-decoration: none;

  &::after {
    position: absolute;
    inset: 0;
    content: "";
  }

  &:focus-visible {
    outline: none;
  }

  &:focus-visible::after {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }

  @media (max-width: 720px) {
    grid-template-columns: 42px minmax(0, 1fr) auto;
    row-gap: 2px;
  }
`;

const RowBody = styled.div`
  min-width: 0;
`;

const RowName = styled.p`
  margin: 0;
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const RowDescription = styled.p`
  margin: 2px 0 0;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.45;
  text-overflow: ellipsis;
  white-space: nowrap;

  /* On a phone the second line carries the fields, not the prose. */
  @media (max-width: 720px) {
    display: none;
  }
`;

const RowCell = styled.span`
  color: var(--service-text-muted);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  text-align: right;
  white-space: nowrap;

  &.visibility {
    text-align: left;
  }

  @media (max-width: 720px) {
    display: none;
  }
`;

const RowMobileMeta = styled.p`
  display: none;
  margin: 2px 0 0;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 720px) {
    display: block;
  }
`;

const HeaderRow = styled.div`
  display: grid;
  grid-template-columns: 42px minmax(0, 1fr) 96px 104px 88px;
  column-gap: 16px;
  padding-bottom: 8px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  span:nth-child(n + 3) {
    text-align: right;
  }

  /* Visibility is a word column; its values read from the left. */
  span:nth-child(3) {
    text-align: left;
  }

  @media (max-width: 720px) {
    display: none;
  }
`;

const SkeletonRow = styled.li`
  height: 67px;
  border-top: 1px solid var(--service-border);
  background: var(--service-surface-muted);
  opacity: 0.5;
`;

const EmptyState = styled.p`
  margin: 0;
  padding: 32px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const ErrorText = styled.p`
  margin: 0 0 12px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const LoadMoreRow = styled.div`
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 12px 0;
`;

const LoadMoreCount = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
`;

const LoadMoreButton = styled(SecondaryButton)`
  margin: 0;
`;

function roleLabel(role: GroupRole): string {
  return role === "owner" ? "Owner" : role === "admin" ? "Admin" : "Member";
}

function GroupRow({ group }: { group: GroupCardData }) {
  return (
    <Row>
      <RowLink href={`/groups/${group.slug}`}>
        <GroupMark name={group.name} avatarUrl={group.avatarUrl} />
        <RowBody>
          <RowName>{group.name}</RowName>
          <RowDescription>{group.description || "No description."}</RowDescription>
          <RowMobileMeta>
            {`${group.isPublic ? "Public" : "Private"} · ${formatGroupMemberCount(group.memberCount)}`}
            {group.role ? ` · ${roleLabel(group.role)}` : ""}
          </RowMobileMeta>
        </RowBody>
        <RowCell className="visibility">{group.isPublic ? "Public" : "Private"}</RowCell>
        <RowCell>{group.memberCount.toLocaleString("en-US")}</RowCell>
        <RowCell>{group.role ? roleLabel(group.role) : "—"}</RowCell>
      </RowLink>
    </Row>
  );
}

export default function GroupsBrowser({
  currentUser,
  initialPublicGroups,
  initialMyGroups,
  initialPublicPagination,
  initialMyPagination,
}: GroupsBrowserProps) {
  const [activeTab, setActiveTab] = useState<ActiveTab>(currentUser ? "mine" : "public");
  const [publicGroups, setPublicGroups] = useState(initialPublicGroups);
  const [myGroups, setMyGroups] = useState(initialMyGroups);
  const [publicPagination, setPublicPagination] = useState(initialPublicPagination);
  const [myPagination, setMyPagination] = useState(initialMyPagination);
  const [loadingState, setLoadingState] = useState<Record<ActiveTab, boolean>>({
    public: false,
    mine: false,
  });
  const [error, setError] = useState<string | null>(null);
  // Tracks the in-flight fetch per tab so we can:
  //   1. Cancel a same-tab duplicate (rapid Load More clicks) without
  //      stomping a request from the other tab.
  //   2. Always clear the loading state on completion or abort — the
  //      previous implementation skipped the `setTabLoading(tab, false)`
  //      reset when the request was aborted, which left the tab stuck
  //      with loading=true if the abort happened mid-flight.
  const inflightByTab = useRef<Map<ActiveTab, AbortController>>(new Map());

  const setTabLoading = useCallback((tab: ActiveTab, isLoading: boolean) => {
    setLoadingState((current) => ({ ...current, [tab]: isLoading }));
  }, []);

  const loadGroups = useCallback((tab: ActiveTab, append = false) => {
    // Abort any in-flight request for THIS tab (a fresh Load More
    // supersedes the previous one). Requests for the other tab keep
    // running so a tab switch does not lose work.
    inflightByTab.current.get(tab)?.abort();
    const controller = new AbortController();
    inflightByTab.current.set(tab, controller);

    const page =
      append && tab === "mine"
        ? myPagination.page + 1
        : append
          ? publicPagination.page + 1
          : 1;
    const params = new URLSearchParams({
      page: String(page),
      limit: "20",
    });

    if (tab === "mine") {
      params.set("my", "true");
    }

    const url = `/api/groups?${params.toString()}`;
    setTabLoading(tab, true);
    setError(null);

    fetch(url, { signal: controller.signal })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then((payload) => {
        if (!Array.isArray(payload.groups)) {
          throw new Error("Invalid response");
        }

        const nextPagination = payload.pagination;

        if (tab === "mine") {
          setMyGroups((prev) => (append ? [...prev, ...payload.groups] : payload.groups));
          if (nextPagination) {
            setMyPagination(nextPagination);
          }
        } else {
          setPublicGroups((prev) =>
            append ? [...prev, ...payload.groups] : payload.groups,
          );
          if (nextPagination) {
            setPublicPagination(nextPagination);
          }
        }
      })
      .catch((err) => {
        if (err.name !== "AbortError") {
          setError(err.message || "Failed to load groups");
        }
      })
      .finally(() => {
        // Only clear loading if this controller is still the active one
        // for this tab. If a newer request superseded it, leave the
        // newer loading state alone (it owns the spinner now).
        if (inflightByTab.current.get(tab) === controller) {
          setTabLoading(tab, false);
          inflightByTab.current.delete(tab);
        }
      });
  }, [myPagination.page, publicPagination.page, setTabLoading]);

  // Abort every in-flight request on unmount.
  useEffect(() => {
    const inflight = inflightByTab.current;
    return () => {
      for (const controller of inflight.values()) {
        controller.abort();
      }
      inflight.clear();
    };
  }, []);

  const groups = activeTab === "mine" ? myGroups : publicGroups;
  const activePagination = activeTab === "mine" ? myPagination : publicPagination;
  const isLoading = loadingState[activeTab];
  const handleTabChange = (tab: ActiveTab) => {
    if (activeTab === tab) {
      return;
    }

    setActiveTab(tab);

    // Skip the network fetch when the tab's SSR data is still on page 1 and
    // already has rows — no stale data to refresh.
    const currentGroups = tab === "mine" ? myGroups : publicGroups;
    const currentPagination = tab === "mine" ? myPagination : publicPagination;
    if (currentPagination.page === 1 && currentGroups.length > 0) {
      return;
    }

    loadGroups(tab);
  };

  const handleLoadMore = () => {
    loadGroups(activeTab, true);
  };

  const directoryCount = activePagination.total;

  return (
    <>
      <Toolbar>
        <ToolbarLeft>
          <SegmentedControl
            label="Group filter"
            value={activeTab}
            options={[
              { value: "public", label: "Public" },
              { value: "mine", label: "My groups", disabled: !currentUser },
            ]}
            onChange={handleTabChange}
          />
        </ToolbarLeft>
        <DirectoryStatus role="status" aria-live="polite">
          {activeTab === "mine"
            ? `${directoryCount.toLocaleString("en-US")} ${directoryCount === 1 ? "group" : "groups"} of yours`
            : `${directoryCount.toLocaleString("en-US")} public ${directoryCount === 1 ? "group" : "groups"}`}
        </DirectoryStatus>
      </Toolbar>

      {error && <ErrorText role="alert">{error}</ErrorText>}

      {isLoading && groups.length === 0 ? (
        <Table aria-busy="true" aria-live="polite" aria-label="Loading groups">
          {Array.from({ length: 5 }).map((_, i) => (
            <SkeletonRow key={i} />
          ))}
        </Table>
      ) : groups.length === 0 ? (
        <EmptyState>
          {activeTab === "mine"
            ? "You are not in any groups yet."
            : "No public groups yet."}
        </EmptyState>
      ) : (
        <>
          <HeaderRow aria-hidden="true">
            <span />
            <span>Group</span>
            <span>Visibility</span>
            <span>Members</span>
            <span>Your role</span>
          </HeaderRow>
          <Table role="list">
            {groups.map((group) => (
              <GroupRow key={group.id} group={group} />
            ))}
          </Table>
          <LoadMoreRow>
            {activePagination.hasNext ? (
              <LoadMoreButton
                type="button"
                onClick={handleLoadMore}
                disabled={isLoading}
              >
                {isLoading ? "Loading…" : "Load more groups"}
              </LoadMoreButton>
            ) : null}
            <LoadMoreCount>
              {groups.length.toLocaleString("en-US")} of {directoryCount.toLocaleString("en-US")} shown
            </LoadMoreCount>
          </LoadMoreRow>
        </>
      )}
    </>
  );
}
