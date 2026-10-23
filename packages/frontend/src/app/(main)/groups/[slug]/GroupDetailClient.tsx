"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { GroupManagement } from "@/components/leaderboard/GroupManagement";
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
import type { SortBy } from "@/lib/leaderboard/types";
import type { GroupPeriod } from "@/lib/reportingPeriods";
import { GroupReportingSettings } from "@/components/leaderboard/GroupReportingSettings";

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
  timeZone: string;
  weekStartsOn: number;
  memberCount: number;
  membership: { role: GroupRole } | null;
}

interface ManagedInvite {
  id: string;
  role: "admin" | "member";
  invitedUsername: string | null;
  status: string;
  maxUses: number;
  useCount: number;
  expiresAt: string;
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
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: 10px;

  @media (max-width: 720px) {
    grid-template-columns: 1fr;
  }
`;

const InviteField = styled.label`
  display: grid;
  gap: 6px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const InviteList = styled.ul`
  list-style: none;
  margin: 0;
  padding: 0;
  li { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px; padding: 12px 0; border-top: 1px solid var(--service-border); }
  span { overflow-wrap: anywhere; }
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
  group: initialGroup,
  currentUser,
  initialData,
}: GroupDetailClientProps) {
  const router = useRouter();
  const [group, setGroup] = useState(initialGroup);
  const [data, setData] = useState(initialData);
  const [period, setPeriod] = useState<GroupPeriod>(initialData.period);
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
  const [maxUses, setMaxUses] = useState("1");
  const [expiresInDays, setExpiresInDays] = useState("14");
  const [invites, setInvites] = useState<ManagedInvite[]>([]);
  const [inviteBusy, setInviteBusy] = useState(false);
  const [inviteListError, setInviteListError] = useState<string | null>(null);
  const [createdInviteId, setCreatedInviteId] = useState<string | null>(null);
  const didMountLeaderboard = useRef(false);
  const leaderboardRequestId = useRef(0);
  const inviteRequestId = useRef(0);

  const canInvite = isAdminRole(group.membership?.role);

  const loadInvites = useCallback(async (signal?: AbortSignal) => {
    const requestId = ++inviteRequestId.current;
    try {
      const response = await fetch(`/api/groups/${group.slug}/invite`, { cache: "no-store", signal });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error || "Failed to load invites");
      if (!signal?.aborted && requestId === inviteRequestId.current) { setInvites(payload.invites); setInviteListError(null); }
    } catch (err) {
      if (!signal?.aborted && requestId === inviteRequestId.current) setInviteListError(err instanceof Error ? err.message : "Failed to load invites");
    }
  }, [group.slug]);

  useEffect(() => {
    if (!canInvite) return;
    const controller = new AbortController();
    void loadInvites(controller.signal);
    return () => controller.abort();
  }, [canInvite, group.membership?.role, loadInvites]);

  async function revokeInvite(inviteId: string) {
    if (inviteBusy) return;
    setInviteBusy(true);
    setInviteError(null);
    try {
      const response = await fetch(`/api/groups/${group.slug}/invite`, {
        method: "DELETE", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ inviteId }),
      });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error || "Failed to revoke invite");
      if (createdInviteId === inviteId) { setInviteUrl(null); setCreatedInviteId(null); }
      await loadInvites();
    } catch (err) { setInviteError(err instanceof Error ? err.message : "Failed to revoke invite"); }
    finally { setInviteBusy(false); }
  }

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedSearch(search);
      setPage(1);
    }, 250);
    return () => clearTimeout(timer);
  }, [search]);

  const loadLeaderboard = useCallback((signal?: AbortSignal) => {
    const requestId = ++leaderboardRequestId.current;
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

    return fetch(`/api/groups/${group.slug}/leaderboard?${params}`, { signal, cache: "no-store" })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then((payload) => {
        if (requestId === leaderboardRequestId.current && !signal?.aborted) setData(payload);
      })
      .catch((err) => {
        if (requestId === leaderboardRequestId.current && err.name !== "AbortError") {
          setError(err.message || "Failed to load leaderboard");
        }
      })
      .finally(() => {
        if (requestId === leaderboardRequestId.current && !signal?.aborted) {
          setIsLoading(false);
        }
      });
  }, [debouncedSearch, group.slug, page, period, sortBy]);
  const currentLeaderboardLoader = useRef(loadLeaderboard);

  useEffect(() => {
    currentLeaderboardLoader.current = loadLeaderboard;
    if (!didMountLeaderboard.current) {
      didMountLeaderboard.current = true;
      return;
    }

    const abortController = new AbortController();
    loadLeaderboard(abortController.signal);
    return () => abortController.abort();
  }, [loadLeaderboard]);

  async function refreshGroup() {
    const response = await fetch(`/api/groups/${group.slug}`, { cache: "no-store" });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || "Failed to refresh group");
    setGroup(payload);
    await currentLeaderboardLoader.current();
  }

  async function createInvite() {
    if (inviteBusy) return;
    setInviteError(null);
    setInviteUrl(null);
    setCopied(false);
    setInviteBusy(true);

    try {
      const response = await fetch(`/api/groups/${group.slug}/invite`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          role: group.membership?.role === "owner" ? inviteRole : "member",
          invitedUsername: inviteUsername.trim() || null,
          maxUses: inviteUsername.trim() ? 1 : Number(maxUses),
          expiresInDays: Number(expiresInDays),
        }),
      });
      const payload = await response.json();

      if (!response.ok) {
        throw new Error(payload.error || "Failed to create invite");
      }

      const absoluteUrl = `${window.location.origin}${payload.joinUrl}`;
      setInviteUrl(absoluteUrl);
      setCreatedInviteId(payload.invite.id);
      setInviteUsername("");
      await loadInvites();
    } catch (err) {
      setInviteError(err instanceof Error ? err.message : "Failed to create invite");
    } finally { setInviteBusy(false); }
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
            <MetricValue aria-label={formatGroupMemberCount(group.memberCount)}>
              {group.memberCount.toLocaleString("en-US")}
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
            { value: "all" as GroupPeriod, label: "All time" },
            { value: "month" as GroupPeriod, label: "This month" },
            { value: "last-month" as GroupPeriod, label: "Last month" },
            { value: "week" as GroupPeriod, label: "This week" },
            { value: "last-week" as GroupPeriod, label: "Last week" },
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

      <Description aria-label="Group reporting period">
        {data.dateRange ? `${data.dateRange.start} – ${data.dateRange.end}` : "All submitted dates"}
        {` · ${data.timeZone} · Week starts ${["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][data.weekStartsOn]}`}
      </Description>

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
        <GroupReportingSettings
          slug={group.slug}
          timeZone={group.timeZone}
          weekStartsOn={group.weekStartsOn}
          onChanged={refreshGroup}
        />
      )}
      {canInvite && (
        <InvitePanel>
          <InviteHeading>
            <InviteTitle>Invite members</InviteTitle>
            <InviteDescription>
              Choose how many accounts can join. Each account can use a link once, even after leaving. Account-specific links allow one join.
            </InviteDescription>
          </InviteHeading>
          <InviteForm>
            <InviteField>GitHub username (optional)<Input
              type="text"
              name="invite-username"
              aria-label="Invitee GitHub username"
              value={inviteUsername}
              onChange={(event) => setInviteUsername(event.target.value)}
              placeholder="Any account"
            /></InviteField>
            <InviteField>Role<Select
              name="invite-role"
              aria-label="Invite role"
              value={group.membership?.role === "owner" ? inviteRole : "member"}
              onChange={(event) => setInviteRole(event.target.value as Exclude<GroupRole, "owner">)}
            >
              <option value="member">Member</option>
              {group.membership?.role === "owner" && <option value="admin">Admin</option>}
            </Select></InviteField>
            <InviteField>Maximum joins<Input type="number" min={1} max={1000} step={1}
              value={inviteUsername.trim() ? "1" : maxUses} disabled={!!inviteUsername.trim()}
              onChange={(event) => setMaxUses(event.target.value)} /></InviteField>
            <InviteField>Expires in days<Input type="number" min={1} max={30} step={1}
              value={expiresInDays} onChange={(event) => setExpiresInDays(event.target.value)} /></InviteField>
            <Button type="button" disabled={inviteBusy} onClick={createInvite}>{inviteBusy ? "Saving..." : "Create invite"}</Button>
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
          <InviteDescription>Save the link now; it cannot be retrieved later. Showing the latest 100 invites.</InviteDescription>
          <SecondaryButton type="button" onClick={() => void loadInvites()} disabled={inviteBusy}>Refresh invites</SecondaryButton>
          {inviteListError && <ErrorText role="alert">{inviteListError}</ErrorText>}
          <InviteList aria-label="Recent invites">
            {invites.map((invite) => {
              const expired = new Date(invite.expiresAt).getTime() <= Date.now();
              const active = invite.status === "pending" && !expired;
              return <li key={invite.id}>
                <span>{invite.invitedUsername ? `@${invite.invitedUsername}` : "Any account"} · {invite.role} · {invite.useCount}/{invite.maxUses} joined<br />
                  {invite.status === "pending" && expired ? "expired" : invite.status} · expires {new Date(invite.expiresAt).toLocaleString()}</span>
                {active && <SecondaryButton type="button" disabled={inviteBusy} onClick={() => void revokeInvite(invite.id)}>Revoke</SecondaryButton>}
              </li>;
            })}
          </InviteList>
        </InvitePanel>
      )}
      {canInvite && currentUser && group.membership && (
        <GroupManagement
          slug={group.slug}
          userId={currentUser.id}
          role={group.membership.role}
          onChanged={refreshGroup}
        />
      )}
    </>
  );
}
