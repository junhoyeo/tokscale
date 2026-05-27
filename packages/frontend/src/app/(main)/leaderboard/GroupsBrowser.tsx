"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";
import styled from "styled-components";

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

const Header = styled.section`
  margin: 16px 0 20px;
  display: flex;
  justify-content: space-between;
  gap: 16px;
  align-items: flex-start;

  @media (max-width: 640px) {
    flex-direction: column;
  }
`;

const Description = styled.p`
  margin: 0;
  max-width: 680px;
  color: var(--color-fg-muted);
  line-height: 1.6;
`;

const PrimaryLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 40px;
  padding: 0 16px;
  border-radius: 8px;
  border: 1px solid var(--color-primary);
  background: var(--color-primary);
  color: #fff;
  font-size: 14px;
  font-weight: 600;
  text-decoration: none;
  white-space: nowrap;
`;

const Tabs = styled.div`
  display: inline-flex;
  padding: 4px;
  margin-bottom: 16px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
`;

const TabButton = styled.button<{ $active: boolean }>`
  min-height: 32px;
  padding: 0 14px;
  border: 0;
  border-radius: 6px;
  background: ${({ $active }) => ($active ? "var(--color-bg-default)" : "transparent")};
  color: ${({ $active }) => ($active ? "var(--color-fg-default)" : "var(--color-fg-muted)")};
  font-weight: 600;
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--color-primary);
    outline-offset: 2px;
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
`;

const Grid = styled.div`
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
`;

const Card = styled(Link)`
  display: flex;
  flex-direction: column;
  min-height: 150px;
  padding: 16px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-default);
  color: inherit;
  text-decoration: none;
  transition: border-color 0.15s, background 0.15s;

  &:hover {
    border-color: var(--color-primary);
    background: var(--color-bg-subtle);
  }

  &:focus-visible {
    outline: 2px solid var(--color-primary);
    outline-offset: 2px;
    border-color: var(--color-primary);
  }
`;

const SkeletonGrid = styled(Grid)`
  pointer-events: none;
`;

const SkeletonCard = styled.div`
  min-height: 150px;
  padding: 16px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: linear-gradient(
    90deg,
    var(--color-bg-default) 0%,
    var(--color-bg-subtle) 50%,
    var(--color-bg-default) 100%
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

const CardTop = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
`;

const Avatar = styled.div<{ $image?: string | null }>`
  width: 42px;
  height: 42px;
  border-radius: 8px;
  flex: 0 0 auto;
  border: 1px solid var(--color-border-default);
  background: ${({ $image }) =>
    $image ? `url(${$image}) center/cover` : "linear-gradient(135deg, #0073ff, #13a10e)"};
`;

const CardTitle = styled.h2`
  margin: 0;
  font-size: 16px;
  color: var(--color-fg-default);
`;

const Meta = styled.div`
  margin-top: 3px;
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  color: var(--color-fg-muted);
  font-size: 12px;
`;

const BodyText = styled.p`
  margin: 0;
  color: var(--color-fg-muted);
  line-height: 1.5;
  font-size: 14px;
`;

const EmptyState = styled.div`
  padding: 28px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-default);
  color: var(--color-fg-muted);
`;

const ErrorText = styled.p`
  color: var(--color-danger-fg, #f85149);
`;

const LoadMoreButton = styled.button`
  margin-top: 16px;
  min-height: 36px;
  padding: 0 16px;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-default);
  color: var(--color-fg-default);
  cursor: pointer;

  &:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
`;

function GroupCard({ group }: { group: GroupCardData }) {
  return (
    <Card href={`/groups/${group.slug}`}>
      <CardTop>
        <Avatar $image={group.avatarUrl} />
        <div>
          <CardTitle>{group.name}</CardTitle>
          <Meta>
            <span>{group.isPublic ? "Public" : "Private"}</span>
            <span>{group.memberCount} members</span>
            {group.role && <span>{group.role}</span>}
          </Meta>
        </div>
      </CardTop>
      <BodyText>{group.description || "A scoped Tokscale leaderboard."}</BodyText>
    </Card>
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

  return (
    <>
      <Header>
        <Description>
          Create private or public leaderboards for teams, friends, and workspaces without changing
          the global Tokscale rankings.
        </Description>
        {currentUser ? (
          <PrimaryLink href="/groups/new">New group</PrimaryLink>
        ) : (
          <PrimaryLink href={signInHref}>Sign in</PrimaryLink>
        )}
      </Header>

      <Tabs aria-label="Group filters">
        <TabButton $active={activeTab === "public"} onClick={() => handleTabChange("public")}>
          Public
        </TabButton>
        <TabButton
          $active={activeTab === "mine"}
          onClick={() => handleTabChange("mine")}
          disabled={!currentUser}
        >
          My groups
        </TabButton>
      </Tabs>

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
              <Grid>
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
