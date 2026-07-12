"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { CheckIcon, CopyIcon, SearchIcon, XIcon } from "@/components/ui/Icons";
import {
  CompactBadge,
  GroupMark,
  MetricItem,
  MetricLabel,
  MetricStrip,
  MetricValue,
  MobileRankingList,
  MobileRankingRow,
  SecondaryActionLink,
  SecondaryButton,
  SegmentedControl,
} from "@/components/leaderboard/RankingUI";
import { formatGroupMemberCount } from "@/components/leaderboard/presentation";
import { formatCurrency, formatNumber } from "@/lib/utils";
import type { GroupLeaderboardData, GroupLeaderboardUser } from "@/lib/groups/getGroupLeaderboard";
import type { Period, SortBy } from "@/lib/leaderboard/types";

type GroupRole = "owner" | "admin" | "member";

interface SessionUser {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
}

interface GroupDetail {
  id: string;
  name: string;
  slug: string;
  description: string | null;
  avatarUrl: string | null;
  isPublic: boolean;
  memberCount: number;
  membership: { role: GroupRole } | null;
}

interface GroupDetailClientProps {
  group: GroupDetail;
  currentUser: SessionUser | null;
  initialData: GroupLeaderboardData;
}

const Header = styled.section`
  display: grid;
  gap: 16px;
  margin-bottom: 24px;
`;

const HeaderTop = styled.div`
  display: flex;
  justify-content: space-between;
  gap: 16px;
  align-items: flex-start;

  @media (max-width: 720px) {
    align-items: stretch;
    flex-direction: column;
  }
`;

const Identity = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 14px;
`;

const IdentityCopy = styled.div`
  min-width: 0;
`;

const Title = styled.h1`
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--service-text);
  font-size: 1.5rem;
  font-weight: 600;
  letter-spacing: -0.02em;
  text-wrap: balance;
`;

const Meta = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 6px;
`;

const Description = styled.p`
  margin: 0;
  max-width: 76ch;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Actions = styled.div`
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
`;

const Button = styled(SecondaryButton)`
  gap: 8px;
`;

const InvitePanel = styled.div`
  display: grid;
  gap: 12px;
  margin-top: 24px;
  padding-top: 20px;
  border-top: 1px solid var(--service-border);
`;

const InviteHeading = styled.div`
  display: grid;
  gap: 4px;
`;

const InviteTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
`;

const InviteDescription = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const InviteForm = styled.div`
  display: grid;
  grid-template-columns: minmax(180px, 1fr) 140px auto;
  gap: 10px;

  @media (max-width: 720px) {
    grid-template-columns: 1fr;
  }
`;

const Input = styled.input`
  min-height: 36px;
  padding: 0 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: var(--service-surface);
  color: var(--service-text);
  font: inherit;

  &:focus-visible {
    border-color: var(--service-focus);
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

const Select = styled.select`
  min-height: 36px;
  padding: 0 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: var(--service-surface);
  color: var(--service-text);
  font: inherit;

  &:focus-visible {
    border-color: var(--service-focus);
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

const LinkBox = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px;
  border: 1px solid var(--service-border);
  border-radius: 8px;
  background: var(--service-surface);
  color: var(--service-text);
  overflow: hidden;
`;

const LinkText = styled.code`
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const Toolbar = styled.div`
  display: flex;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin: 0 0 14px;
`;

const SearchWrapper = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 36px;
  padding: 0 10px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: var(--service-surface);
  color: var(--service-text-muted);

  &:focus-within {
    border-color: var(--service-focus);
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
  }
`;

const SearchInput = styled.input`
  width: 180px;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--service-text);
  font: inherit;

  @media (max-width: 640px) {
    width: 100%;
    min-width: 0;
    font-size: 1rem;
  }
`;

const ClearSearchButton = styled.button`
  display: inline-grid;
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  place-items: center;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--service-text-muted);

  &:hover {
    color: var(--service-text);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const TableContainer = styled.div`
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
`;

const TableWrapper = styled.div`
  display: none;

  @media (min-width: 720px) {
    display: block;
  }
`;

const Table = styled.table`
  width: 100%;
`;

const Th = styled.th`
  padding: 12px 16px;
  text-align: left;
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--service-text-muted);
  border-bottom: 1px solid var(--service-border);
  white-space: nowrap;

  &.right {
    text-align: right;
  }
`;

const Td = styled.td`
  padding: 12px 16px;
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;

  &.right {
    text-align: right;
  }
`;

const UserCell = styled(Link)`
  display: inline-flex;
  align-items: center;
  gap: 10px;
  color: inherit;
  text-decoration: none;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 3px;
  }
`;

const UserAvatar = styled.img`
  width: 34px;
  height: 34px;
  border-radius: 50%;
  object-fit: cover;
  outline: 1px solid var(--service-border);
  outline-offset: -1px;
`;

const Muted = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

const DesktopRow = styled.tr<{ $current: boolean }>`
  background: ${({ $current }) => $current ? "var(--service-accent-soft)" : "transparent"};
  box-shadow: ${({ $current }) => $current ? "inset 2px 0 0 var(--service-accent)" : "none"};
`;

const Pagination = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 0;
`;

const PaginationStatus = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const PaginationActions = styled.div`
  display: flex;
  gap: 8px;
`;

const EmptyState = styled.div`
  padding: 32px;
  text-align: center;
  color: var(--service-text-muted);

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const ErrorText = styled.p`
  margin: 0;
  color: #ff8c85;
`;

function isAdminRole(role: GroupRole | undefined): boolean {
  return role === "owner" || role === "admin";
}

function roleLabel(role: string): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function GroupRow({
  user,
  isCurrentUser,
}: {
  user: GroupLeaderboardUser;
  isCurrentUser: boolean;
}) {
  return (
    <DesktopRow $current={isCurrentUser}>
      <Td>#{user.rank}</Td>
      <Td>
        <UserCell
          href={`/u/${user.username}`}
          aria-current={isCurrentUser ? "true" : undefined}
        >
          <UserAvatar src={user.avatarUrl || `https://github.com/${user.username}.png`} alt="" />
          <span>
            {user.displayName || user.username}
            <Muted>@{user.username}</Muted>
          </span>
        </UserCell>
      </Td>
      <Td>{roleLabel(user.role)}</Td>
      <Td className="right">{formatCurrency(user.totalCost)}</Td>
      <Td className="right">{formatNumber(user.totalTokens)}</Td>
    </DesktopRow>
  );
}

function GroupMobileRow({
  user,
  isCurrentUser,
  sortBy,
}: {
  user: GroupLeaderboardUser;
  isCurrentUser: boolean;
  sortBy: SortBy;
}) {
  const primary = sortBy === "cost"
    ? { label: "Cost", value: formatCurrency(user.totalCost) }
    : { label: "Tokens", value: formatNumber(user.totalTokens) };
  const secondary = [
    roleLabel(user.role),
    sortBy === "cost"
      ? `${formatNumber(user.totalTokens)} tokens`
      : formatCurrency(user.totalCost),
  ].filter(Boolean).join(" · ");

  return (
    <MobileRankingRow
      rank={user.rank}
      href={`/u/${user.username}`}
      avatarUrl={user.avatarUrl}
      username={user.username}
      displayName={user.displayName || user.username}
      primaryLabel={primary.label}
      primaryValue={primary.value}
      meta={secondary}
      isCurrentUser={isCurrentUser}
    />
  );
}

export default function GroupDetailClient({
  group,
  currentUser,
  initialData,
}: GroupDetailClientProps) {
  const router = useRouter();
  const [data, setData] = useState(initialData);
  const [period, setPeriod] = useState<Period>(initialData.period);
  const [sortBy, setSortBy] = useState<SortBy>(initialData.sortBy);
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [inviteRole, setInviteRole] = useState<Exclude<GroupRole, "owner">>("member");
  const [inviteUsername, setInviteUsername] = useState("");
  const [inviteUrl, setInviteUrl] = useState<string | null>(null);
  const [inviteError, setInviteError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const didMountLeaderboard = useRef(false);

  const canInvite = isAdminRole(group.membership?.role);

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedSearch(search);
      setPage(1);
    }, 250);
    return () => clearTimeout(timer);
  }, [search]);

  const loadLeaderboard = useCallback((signal?: AbortSignal) => {
    const params = new URLSearchParams({
      period,
      sortBy,
      page: String(page),
      limit: "50",
    });
    if (debouncedSearch) {
      params.set("search", debouncedSearch);
    }

    setIsLoading(true);
    setError(null);

    fetch(`/api/groups/${group.slug}/leaderboard?${params}`, { signal })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then((payload) => {
        setData(payload);
      })
      .catch((err) => {
        if (err.name !== "AbortError") {
          setError(err.message || "Failed to load leaderboard");
        }
      })
      .finally(() => {
        if (!signal?.aborted) {
          setIsLoading(false);
        }
      });
  }, [debouncedSearch, group.slug, page, period, sortBy]);

  useEffect(() => {
    if (!didMountLeaderboard.current) {
      didMountLeaderboard.current = true;
      return;
    }

    const abortController = new AbortController();
    loadLeaderboard(abortController.signal);
    return () => abortController.abort();
  }, [loadLeaderboard]);

  async function createInvite() {
    setInviteError(null);
    setInviteUrl(null);

    try {
      const response = await fetch(`/api/groups/${group.slug}/invite`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          role: inviteRole,
          invitedUsername: inviteUsername.trim() || null,
        }),
      });
      const payload = await response.json();

      if (!response.ok) {
        throw new Error(payload.error || "Failed to create invite");
      }

      const absoluteUrl = `${window.location.origin}${payload.joinUrl}`;
      setInviteUrl(absoluteUrl);
      setInviteUsername("");
    } catch (err) {
      setInviteError(err instanceof Error ? err.message : "Failed to create invite");
    }
  }

  async function copyInvite() {
    if (!inviteUrl) return;

    try {
      setInviteError(null);
      await navigator.clipboard.writeText(inviteUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
      setInviteError("Could not copy invite link.");
    }
  }

  async function leaveGroup() {
    const response = await fetch(`/api/groups/${group.slug}/leave`, { method: "POST" });
    if (response.ok) {
      router.push("/leaderboard?view=groups");
    }
  }

  const sortedUsers = useMemo(() => data.users || [], [data.users]);

  return (
    <>
      <Header>
        <HeaderTop>
          <Identity>
            <GroupMark name={group.name} avatarUrl={group.avatarUrl} size="detail" />
            <IdentityCopy>
              <Title>{group.name}</Title>
              <Meta>
                <CompactBadge>{group.isPublic ? "Public" : "Private"}</CompactBadge>
                {group.membership && <CompactBadge>{roleLabel(group.membership.role)}</CompactBadge>}
              </Meta>
            </IdentityCopy>
          </Identity>
          <Actions>
            <SecondaryActionLink href="/leaderboard?view=groups">All groups</SecondaryActionLink>
            {group.membership && group.membership.role !== "owner" && (
              <Button type="button" onClick={leaveGroup}>Leave group</Button>
            )}
          </Actions>
        </HeaderTop>
        {group.description && <Description>{group.description}</Description>}

        <MetricStrip>
          <MetricItem>
            <MetricLabel>Active users</MetricLabel>
            <MetricValue>{data.stats.activeUsers.toLocaleString("en-US")}</MetricValue>
          </MetricItem>
          <MetricItem>
            <MetricLabel>Members</MetricLabel>
            <MetricValue aria-label={formatGroupMemberCount(data.stats.totalMembers || group.memberCount)}>
              {(data.stats.totalMembers || group.memberCount).toLocaleString("en-US")}
            </MetricValue>
          </MetricItem>
          <MetricItem>
            <MetricLabel>Tokens</MetricLabel>
            <MetricValue $accent>{formatNumber(data.stats.totalTokens)}</MetricValue>
          </MetricItem>
          <MetricItem>
            <MetricLabel>Cost</MetricLabel>
            <MetricValue>{formatCurrency(data.stats.totalCost)}</MetricValue>
          </MetricItem>
        </MetricStrip>
      </Header>

      <Toolbar>
        <SegmentedControl
          label="Group leaderboard period"
          value={period}
          options={[
            { value: "all" as Period, label: "All time" },
            { value: "month" as Period, label: "This month" },
            { value: "week" as Period, label: "This week" },
          ]}
          onChange={(value) => {
            setPeriod(value);
            setPage(1);
          }}
        />

        <Actions>
          <SearchWrapper>
            <SearchIcon size={16} />
            <SearchInput
              type="text"
              name="group-member-search"
              aria-label="Search group members"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search members"
            />
            {search && (
              <ClearSearchButton type="button" onClick={() => setSearch("")} aria-label="Clear search">
                <XIcon size={16} />
              </ClearSearchButton>
            )}
          </SearchWrapper>
          <SegmentedControl
            label="Group leaderboard sort"
            value={sortBy}
            options={[
              { value: "tokens" as SortBy, label: "Tokens" },
              { value: "cost" as SortBy, label: "Cost" },
            ]}
            onChange={(value) => {
              setSortBy(value);
              setPage(1);
            }}
          />
        </Actions>
      </Toolbar>

      <TableContainer>
        {error ? (
          <EmptyState role="alert">{error}</EmptyState>
        ) : isLoading ? (
          <EmptyState role="status">Loading leaderboard...</EmptyState>
        ) : sortedUsers.length === 0 ? (
          <EmptyState>No submitted usage for this group yet.</EmptyState>
        ) : (
          <>
            <TableWrapper>
              <Table>
                <thead>
                  <tr>
                    <Th>Rank</Th>
                    <Th>User</Th>
                    <Th>Role</Th>
                    <Th className="right">Cost</Th>
                    <Th className="right">Tokens</Th>
                  </tr>
                </thead>
                <tbody>
                  {sortedUsers.map((user) => (
                    <GroupRow
                      key={user.userId}
                      user={user}
                      isCurrentUser={currentUser?.username === user.username}
                    />
                  ))}
                </tbody>
              </Table>
            </TableWrapper>
            <MobileRankingList role="list" aria-label="Group leaderboard rankings">
              {sortedUsers.map((user) => (
                <GroupMobileRow
                  key={user.userId}
                  user={user}
                  isCurrentUser={currentUser?.username === user.username}
                  sortBy={sortBy}
                />
              ))}
            </MobileRankingList>
            {data.pagination.totalPages > 1 && (
              <Pagination>
                <PaginationStatus>
                  Showing {(data.pagination.page - 1) * data.pagination.limit + 1}–{Math.min(data.pagination.page * data.pagination.limit, data.pagination.totalUsers)} of {data.pagination.totalUsers.toLocaleString("en-US")}
                </PaginationStatus>
                <PaginationActions>
                  <SecondaryButton
                    type="button"
                    disabled={!data.pagination.hasPrev}
                    onClick={() => setPage(Math.max(1, data.pagination.page - 1))}
                  >
                    Previous
                  </SecondaryButton>
                  <SecondaryButton
                    type="button"
                    disabled={!data.pagination.hasNext}
                    onClick={() => setPage(data.pagination.page + 1)}
                  >
                    Next
                  </SecondaryButton>
                </PaginationActions>
              </Pagination>
            )}
          </>
        )}
      </TableContainer>

      {canInvite && (
        <InvitePanel>
          <InviteHeading>
            <InviteTitle>Invite members</InviteTitle>
            <InviteDescription>
              Create a scoped link, optionally restricted to one GitHub username.
            </InviteDescription>
          </InviteHeading>
          <InviteForm>
            <Input
              type="text"
              name="invite-username"
              aria-label="Invitee GitHub username"
              value={inviteUsername}
              onChange={(event) => setInviteUsername(event.target.value)}
              placeholder="GitHub username (optional)"
            />
            <Select
              name="invite-role"
              aria-label="Invite role"
              value={inviteRole}
              onChange={(event) => setInviteRole(event.target.value as Exclude<GroupRole, "owner">)}
            >
              <option value="member">Member</option>
              <option value="admin">Admin</option>
            </Select>
            <Button type="button" onClick={createInvite}>Create invite</Button>
          </InviteForm>
          {inviteError && <ErrorText role="alert">{inviteError}</ErrorText>}
          {inviteUrl && (
            <LinkBox>
              <LinkText>{inviteUrl}</LinkText>
              <Button type="button" onClick={copyInvite} aria-label="Copy invite link">
                {copied ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
                {copied ? "Copied" : "Copy"}
              </Button>
            </LinkBox>
          )}
        </InvitePanel>
      )}
    </>
  );
}
