"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";
import styled from "styled-components";
import {
  CompactBadge,
  GroupMark,
  PrimaryActionLink,
  SecondaryButton,
  SegmentedControl,
} from "@/components/leaderboard/RankingUI";
import { formatGroupMemberCount } from "@/components/leaderboard/presentation";

// Inlined view of the groups list that lives under the /leaderboard ?view=groups
// segmented control. The /groups/[slug], /groups/new, and /groups/join/[token]
// subpages still exist as standalone routes; this component just replaces the
// old /groups top-level listing page so groups is no longer a separate nav tab.

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

const Toolbar = styled.section`
  display: flex;
  justify-content: space-between;
  gap: 12px;
  align-items: center;
  margin-bottom: 16px;

  @media (max-width: 640px) {
    align-items: stretch;
    flex-direction: column-reverse;
  }
`;

const DirectoryStatus = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const FilterGroup = styled.div`
  display: grid;
  gap: 8px;
`;

const DirectoryAction = styled(PrimaryActionLink)`
  align-self: flex-start;
`;

const Grid = styled.ul`
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 340px), 1fr));
  gap: 12px;
  margin: 0;
  padding: 0;

  @media (min-width: 1240px) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
`;

const GridItem = styled.li`
  min-width: 0;
  list-style: none;
`;

const Card = styled(Link)`
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 12px;
  min-height: 100%;
  padding: 14px;
  border: 1px solid var(--service-border);
  border-radius: 10px;
  background: var(--service-surface);
  color: inherit;
  text-decoration: none;

  &:hover {
    border-color: var(--service-border-strong);
    background: var(--service-surface-muted);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const SkeletonGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 340px), 1fr));
  gap: 12px;
  pointer-events: none;
`;

const SkeletonCard = styled.div`
  min-height: 94px;
  border: 1px solid var(--service-border);
  border-radius: 10px;
  background: linear-gradient(
    90deg,
    var(--service-surface) 0%,
    var(--service-surface-muted) 50%,
    var(--service-surface) 100%
  );
  background-size: 200% 100%;
  animation: groups-skeleton-shimmer 1.6s ease-in-out infinite;

  @keyframes groups-skeleton-shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    animation: none;
  }
`;

const CardContent = styled.div`
  min-width: 0;
`;

const CardTitle = styled.h2`
  margin: 0;
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Meta = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 6px;
`;

const BodyText = styled.p`
  display: -webkit-box;
  margin: 10px 0 0;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.45;
  text-wrap: pretty;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;

  @media (max-width: 640px) {
    font-size: 1rem;
    line-height: 1.5;
  }
`;

const EmptyState = styled.div`
  padding: 28px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.875rem;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const ErrorText = styled.p`
  margin: 0 0 12px;
  color: #ff8c85;
`;

const LoadMoreButton = styled(SecondaryButton)`
  margin-top: 16px;
`;

function GroupCard({ group }: { group: GroupCardData }) {
  return (
    <GridItem>
      <Card href={`/groups/${group.slug}`}>
        <GroupMark name={group.name} avatarUrl={group.avatarUrl} />
        <CardContent>
          <CardTitle>{group.name}</CardTitle>
          <Meta>
            <CompactBadge>{group.isPublic ? "Public" : "Private"}</CompactBadge>
            <CompactBadge>{formatGroupMemberCount(group.memberCount)}</CompactBadge>
            {group.role && <CompactBadge>{group.role}</CompactBadge>}
          </Meta>
          {group.description && <BodyText>{group.description}</BodyText>}
        </CardContent>
      </Card>
    </GridItem>
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

  // Send unauthenticated users back to /leaderboard?view=groups so they land
  // on the groups view after sign-in (was /groups before the consolidation).
  const signInHref = "/api/auth/github?returnTo=/leaderboard?view=groups";
  const directoryCount = activePagination.total;

  return (
    <>
      <Toolbar>
        <FilterGroup>
          <SegmentedControl
            label="Group filter"
            value={activeTab}
            options={[
              { value: "public", label: "Public" },
              { value: "mine", label: "My groups", disabled: !currentUser },
            ]}
            onChange={handleTabChange}
          />
          <DirectoryStatus role="status" aria-live="polite">
            {activeTab === "mine"
              ? `${directoryCount.toLocaleString("en-US")} ${directoryCount === 1 ? "group" : "groups"}`
              : `${directoryCount.toLocaleString("en-US")} public ${directoryCount === 1 ? "group" : "groups"}`}
          </DirectoryStatus>
        </FilterGroup>
        {currentUser ? (
          <DirectoryAction href="/groups/new">New group</DirectoryAction>
        ) : (
          <DirectoryAction href={signInHref}>Sign in</DirectoryAction>
        )}
      </Toolbar>

      {error && <ErrorText role="alert">{error}</ErrorText>}
      {isLoading && groups.length === 0 ? (
        <SkeletonGrid aria-busy="true" aria-live="polite" aria-label="Loading groups">
          {Array.from({ length: 6 }).map((_, i) => (
            <SkeletonCard key={i} />
          ))}
        </SkeletonGrid>
      ) : (
        <>
          {groups.length === 0 ? (
            <EmptyState>
              {activeTab === "mine" ? "You are not in any groups yet." : "No public groups yet."}
            </EmptyState>
          ) : (
            <>
              <Grid role="list">
                {groups.map((group) => (
                  <GroupCard key={group.id} group={group} />
                ))}
              </Grid>
              {activePagination.hasNext ? (
                <LoadMoreButton
                  type="button"
                  onClick={handleLoadMore}
                  disabled={isLoading}
                >
                  {isLoading ? "Loading..." : "Load more"}
                </LoadMoreButton>
              ) : null}
            </>
          )}
        </>
      )}
    </>
  );
}
