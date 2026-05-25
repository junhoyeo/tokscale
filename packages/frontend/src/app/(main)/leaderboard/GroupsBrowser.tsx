"use client";

import Link from "next/link";
import { useCallback, useState } from "react";
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

interface GroupsBrowserProps {
  currentUser: SessionUser | null;
  initialPublicGroups: GroupCardData[];
  initialMyGroups: GroupCardData[];
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
}: GroupsBrowserProps) {
  const [activeTab, setActiveTab] = useState<ActiveTab>(currentUser ? "mine" : "public");
  const [publicGroups, setPublicGroups] = useState(initialPublicGroups);
  const [myGroups, setMyGroups] = useState(initialMyGroups);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadGroups = useCallback((tab: ActiveTab, signal?: AbortSignal) => {
    const url = tab === "mine" ? "/api/groups?my=true" : "/api/groups";
    setIsLoading(true);
    setError(null);

    fetch(url, { signal })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then((payload) => {
        if (!Array.isArray(payload.groups)) {
          throw new Error("Invalid response");
        }
        if (tab === "mine") {
          setMyGroups(payload.groups);
        } else {
          setPublicGroups(payload.groups);
        }
      })
      .catch((err) => {
        if (err.name !== "AbortError") {
          setError(err.message || "Failed to load groups");
        }
      })
      .finally(() => {
        if (!signal?.aborted) {
          setIsLoading(false);
        }
      });
  }, []);

  const groups = activeTab === "mine" ? myGroups : publicGroups;
  const handleTabChange = (tab: ActiveTab) => {
    setActiveTab(tab);
    loadGroups(tab);
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
      {isLoading ? (
        <SkeletonGrid aria-busy="true" aria-live="polite" aria-label="Loading groups">
          {Array.from({ length: 6 }).map((_, i) => (
            <SkeletonCard key={i} />
          ))}
        </SkeletonGrid>
      ) : groups.length === 0 ? (
        <EmptyState>
          {activeTab === "mine"
            ? "You are not in any groups yet."
            : "No public groups yet."}
        </EmptyState>
      ) : (
        <Grid>
          {groups.map((group) => (
            <GroupCard key={group.id} group={group} />
          ))}
        </Grid>
      )}
    </>
  );
}
