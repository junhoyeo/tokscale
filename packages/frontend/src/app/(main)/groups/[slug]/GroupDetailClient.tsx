"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { DitherField } from "dither-kit/DitherField";
import { DitherInput } from "dither-kit/DitherInput";
import { DitherSearchInput } from "dither-kit/DitherSearchInput";
import { GroupManagement } from "@/components/leaderboard/GroupManagement";
import { CheckIcon, CopyIcon } from "@/components/ui/Icons";
import { Select } from "@/components/ui/Select";
import {
  CompactBadge,
  GroupMark,
  MobileRankingList,
  MobileRankingRow,
  PersonAvatar,
  PrimaryButton,
  RankMark,
  SecondaryActionLink,
  SecondaryButton,
  SegmentedControl,
  ShareBar,
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

/* Title row: the group's mark and name, its one sentence, then the
 * secondary actions on the right. The primary action of the screen (for a
 * manager) is Create invite, in the invite section. */
const Header = styled.section`
  display: grid;
  gap: 16px;
  margin-bottom: 24px;
`;

/* On a phone the row flattens (display: contents) so the reading order is
 * name, description, then the secondary actions — not the actions between
 * the name and its sentence. */
const HeaderTop = styled.div`
  display: flex;
  justify-content: space-between;
  gap: 16px;
  align-items: flex-start;

  @media (max-width: 720px) {
    display: contents;
  }
`;

const Identity = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 16px;

  @media (max-width: 720px) {
    order: 1;
  }
`;

const IdentityCopy = styled.div`
  min-width: 0;
`;

const Title = styled.h1`
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--service-text);
  font-size: 1.75rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
  text-wrap: balance;

  @media (max-width: 640px) {
    font-size: 1.5rem;
  }
`;

const Meta = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
`;

const Description = styled.p`
  order: 2;
  margin: 0;
  max-width: 76ch;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
  text-wrap: pretty;
`;

const Actions = styled.div`
  display: flex;
  flex: 0 0 auto;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;

  @media (max-width: 720px) {
    order: 3;
  }
`;

/* The summary strip: one row split by 1px rules, label above value. On a
 * phone the four cells read as a 2×2 divided block. */
const Summary = styled.dl`
  order: 4;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  margin: 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);

  @media (min-width: 641px) {
    display: flex;
    max-width: max-content;
  }
`;

const SummaryItem = styled.div`
  min-width: 0;
  padding: 10px 12px;
  border-top: 1px solid var(--service-border);
  border-left: 1px solid var(--service-border);

  &:nth-child(1),
  &:nth-child(2) {
    border-top: 0;
  }

  &:nth-child(odd) {
    padding-left: 0;
    border-left: 0;
  }

  @media (min-width: 641px) {
    padding: 12px 28px;
    border-top: 0;

    &:first-child {
      padding-left: 0;
      border-left: 0;
    }

    &:not(:first-child) {
      border-left: 1px solid var(--service-border);
    }

    &:last-child {
      padding-right: 0;
    }
  }
`;

const SummaryLabel = styled.dt`
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  white-space: nowrap;
  text-overflow: ellipsis;
`;

const SummaryValue = styled.dd`
  margin: 4px 0 0;
  overflow: hidden;
  color: var(--service-text);
  font-size: 1.125rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  text-overflow: ellipsis;

  @media (min-width: 641px) {
    font-size: 1.25rem;
  }
`;

const Toolbar = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px 24px;
  margin: 0 0 14px;
`;

const ToolbarRight = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px 24px;
`;

const SearchWrap = styled.div`
  width: 240px;

  @media (max-width: 640px) {
    width: 100%;
  }
`;

const SortGroup = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
`;

const SortLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const PeriodLine = styled.p`
  margin: 0 0 10px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
`;

const TableContainer = styled.div`
  border-bottom: 1px solid var(--service-border);
`;

const TableWrapper = styled.div`
  display: none;

  @media (min-width: 720px) {
    display: block;
  }
`;

const Table = styled.table`
  --row-focus-offset: -2px;

  width: 100%;
`;

const TableHead = styled.thead`
  border-bottom: 1px solid var(--service-border-strong);
`;

const TableHeaderCell = styled.th`
  padding: 10px 16px;
  text-align: left;
  font-size: 0.8125rem;
  font-weight: 400;
  color: var(--service-text-muted);
  white-space: nowrap;

  &:first-child {
    padding-left: 0;
  }

  &:last-child {
    padding-right: 0;
  }

  &.right {
    text-align: right;
  }

  &.rank-cell {
    width: 72px;
  }

  &.role-cell {
    width: 104px;
  }

  &.share-cell {
    width: 170px;
  }

  &.cost-cell {
    width: 108px;
  }

  &.tokens-cell {
    width: 120px;
  }

  @media (min-width: 1100px) {
    &.role-cell {
      width: 120px;
    }

    &.share-cell {
      width: 280px;
    }

    &.cost-cell {
      width: 140px;
    }

    &.tokens-cell {
      width: 150px;
    }
  }
`;

const TableRow = styled.tr<{ $current: boolean }>`
  &:not(:last-child) td {
    border-bottom: 1px solid var(--service-border);
  }

  background: ${({ $current }) => $current ? "var(--service-surface-muted)" : "transparent"};

  /* Your own row keeps its tone under the row-link hover. */
  &[data-row-link]:hover {
    background: ${({ $current }) => $current ? "var(--service-surface-muted)" : "var(--v2-row-hover)"};
  }
`;

const TableCell = styled.td`
  padding: 10px 16px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;

  &:first-child {
    padding-left: 0;
  }

  &:last-child {
    padding-right: 0;
  }

  &.right {
    text-align: right;
  }
`;

const RankCell = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
`;

/* The row's primary link: the whole row opens the profile (row links, globals.css). */
const UserCell = styled(Link)`
  display: inline-flex;
  align-items: center;
  gap: 12px;
  color: inherit;
  text-decoration: none;
`;

const Muted = styled.span`
  display: block;
  margin-top: 2px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const ShareCell = styled.span`
  display: inline-flex;
  min-width: 0;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
`;

const ShareBarBox = styled.span`
  flex: 1 1 0;
  min-width: 0;
  max-width: 180px;
`;

const ShareValue = styled.span`
  min-width: 44px;
  color: var(--service-text);
  font-variant-numeric: tabular-nums;
  text-align: right;
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
  font-variant-numeric: tabular-nums;
`;

const PaginationActions = styled.div`
  display: flex;
  align-items: center;
  gap: 16px;
`;

const EmptyState = styled.p`
  margin: 0;
  padding: 32px 0;
  border-top: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

/* A refresh keeps the rows on screen; only the notice changes. */
const LoadingLine = styled.p`
  margin: 0;
  padding: 8px 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const ErrorText = styled.p`
  margin: 8px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

/* Admin sections use the settings row: a label column (title + one sentence)
 * beside its panel, separated by a full-width rule. */
const AdminSection = styled.section`
  display: grid;
  gap: 16px;
  margin-top: 32px;
  padding-top: 24px;
  border-top: 1px solid var(--service-border);

  @media (min-width: 900px) {
    grid-template-columns: 320px minmax(0, 1fr);
    gap: 40px;
  }
`;

const SectionLabel = styled.div`
  min-width: 0;
`;

const SectionTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 500;
`;

const SectionNote = styled.p`
  margin: 6px 0 0;
  max-width: 46ch;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
`;

const Panel = styled.div`
  display: grid;
  min-width: 0;
  align-content: start;
  gap: 16px;
`;

const FieldGrid = styled.div`
  display: grid;
  gap: 16px;

  @media (min-width: 560px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  /* Field labels are 14px sentence case, not the kit's 12px caption. */
  label {
    font-size: 0.875rem;
    font-weight: 500;
  }
`;

const InviteActions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;
`;

const LinkBox = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid var(--service-border);
  border-radius: var(--radius, 2px);
  background: var(--service-surface-muted);
  overflow: hidden;
`;

const LinkText = styled.code`
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const InviteList = styled.ul`
  margin: 0;
  padding: 0;
  list-style: none;
  border-top: 1px solid var(--service-border);
`;

const InviteRow = styled.li`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 0;
  border-bottom: 1px solid var(--service-border);
`;

const InviteRowText = styled.span`
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
`;

const InviteRowStrong = styled.strong`
  color: var(--service-text);
  font-weight: 500;
`;

function isAdminRole(role: GroupRole | undefined): boolean {
  return role === "owner" || role === "admin";
}

function roleLabel(role: string): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function formatShare(share: number | null): string {
  if (share === null) return "—";
  if (share <= 0) return "0%";
  const percent = share * 100;
  if (percent < 0.1) return "<0.1%";
  return `${percent < 10 ? percent.toFixed(1) : Math.round(percent)}%`;
}

function GroupRow({
  user,
  share,
  maxShare,
  isCurrentUser,
}: {
  user: GroupLeaderboardUser;
  share: number | null;
  maxShare: number;
  isCurrentUser: boolean;
}) {
  return (
    <TableRow $current={isCurrentUser} data-row-link="" data-current-user={isCurrentUser}>
      <TableCell className="rank-cell right">
        <RankCell>
          {user.rank <= 3 && <RankMark level={user.rank as 1 | 2 | 3} />}
          {user.rank}
        </RankCell>
      </TableCell>
      <TableCell>
        <UserCell
          href={`/u/${user.username}`}
          data-row-primary=""
          aria-current={isCurrentUser ? "true" : undefined}
        >
          <PersonAvatar
            src={user.avatarUrl || `https://github.com/${user.username}.png`}
            name={user.displayName || user.username}
            size={34}
          />
          <span>
            {user.displayName || user.username}
            <Muted>@{user.username}</Muted>
          </span>
        </UserCell>
      </TableCell>
      <TableCell>{roleLabel(user.role)}</TableCell>
      <TableCell className="right">
        <ShareCell>
          <ShareBarBox>
            <ShareBar
              value={maxShare > 0 ? (share ?? 0) / maxShare : 0}
              label={`${user.username}: ${formatShare(share)} of the group's tokens`}
            />
          </ShareBarBox>
          <ShareValue>{formatShare(share)}</ShareValue>
        </ShareCell>
      </TableCell>
      <TableCell className="right">{formatCurrency(user.totalCost)}</TableCell>
      <TableCell className="right">{formatNumber(user.totalTokens)}</TableCell>
    </TableRow>
  );
}

function GroupMobileRow({
  user,
  share,
  maxShare,
  isCurrentUser,
  sortBy,
}: {
  user: GroupLeaderboardUser;
  share: number | null;
  maxShare: number;
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
      share={maxShare > 0 && share != null ? share / maxShare : null}
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
  const totalTokens = data.stats.totalTokens;
  const shareFor = (user: GroupLeaderboardUser) => (
    totalTokens > 0 ? user.totalTokens / totalTokens : null
  );
  // The bar is the row's share of the largest share on the screen, so the
  // ranking reads at a glance; the figure states the absolute share.
  const maxShare = sortedUsers.reduce((max, user) => (
    totalTokens > 0 ? Math.max(max, user.totalTokens / totalTokens) : max
  ), 0);
  const rangeStart = (data.pagination.page - 1) * data.pagination.limit + 1;
  const rangeEnd = Math.min(data.pagination.page * data.pagination.limit, data.pagination.totalUsers);

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
                <CompactBadge>{formatGroupMemberCount(group.memberCount)}</CompactBadge>
              </Meta>
            </IdentityCopy>
          </Identity>
          <Actions>
            <SecondaryActionLink href="/leaderboard?view=groups">All groups</SecondaryActionLink>
            {group.membership && group.membership.role !== "owner" && (
              <SecondaryButton type="button" onClick={leaveGroup}>Leave group</SecondaryButton>
            )}
          </Actions>
        </HeaderTop>
        {group.description && <Description>{group.description}</Description>}

        <Summary>
          <SummaryItem>
            <SummaryLabel>Active users</SummaryLabel>
            <SummaryValue>{data.stats.activeUsers.toLocaleString("en-US")}</SummaryValue>
          </SummaryItem>
          <SummaryItem>
            <SummaryLabel>Members</SummaryLabel>
            <SummaryValue aria-label={formatGroupMemberCount(group.memberCount)}>
              {group.memberCount.toLocaleString("en-US")}
            </SummaryValue>
          </SummaryItem>
          <SummaryItem>
            <SummaryLabel>Tokens</SummaryLabel>
            <SummaryValue>{formatNumber(data.stats.totalTokens)}</SummaryValue>
          </SummaryItem>
          <SummaryItem>
            <SummaryLabel>Est. cost</SummaryLabel>
            <SummaryValue>{formatCurrency(data.stats.totalCost)}</SummaryValue>
          </SummaryItem>
        </Summary>
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

        <ToolbarRight>
          <SearchWrap>
            <DitherSearchInput
              value={search}
              onChange={setSearch}
              placeholder="Search members"
              aria-label="Search group members"
              name="group-member-search"
            />
          </SearchWrap>
          <SortGroup>
            <SortLabel>Sort</SortLabel>
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
          </SortGroup>
        </ToolbarRight>
      </Toolbar>

      <PeriodLine>
        {data.dateRange ? `${data.dateRange.start} – ${data.dateRange.end}` : "All submitted dates"}
        {` · ${data.timeZone} · Week starts ${["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][data.weekStartsOn]}`}
      </PeriodLine>

      <TableContainer>
        {error ? (
          <EmptyState role="alert">{error}</EmptyState>
        ) : sortedUsers.length === 0 && isLoading ? (
          <EmptyState role="status">Loading leaderboard…</EmptyState>
        ) : sortedUsers.length === 0 ? (
          <EmptyState>
            {debouncedSearch
              ? `No members match “${debouncedSearch}”.`
              : "No submitted usage for this group yet."}
          </EmptyState>
        ) : (
          <>
            {isLoading && <LoadingLine role="status" aria-live="polite">Updating leaderboard…</LoadingLine>}
            <TableWrapper>
              <Table>
                <TableHead>
                  <tr>
                    <TableHeaderCell scope="col" className="rank-cell right">Rank</TableHeaderCell>
                    <TableHeaderCell scope="col">Member</TableHeaderCell>
                    <TableHeaderCell scope="col" className="role-cell">Role</TableHeaderCell>
                    <TableHeaderCell scope="col" className="share-cell right">Share</TableHeaderCell>
                    <TableHeaderCell scope="col" className="cost-cell right">Est. cost</TableHeaderCell>
                    <TableHeaderCell scope="col" className="tokens-cell right">Tokens</TableHeaderCell>
                  </tr>
                </TableHead>
                <tbody>
                  {sortedUsers.map((user) => (
                    <GroupRow
                      key={user.userId}
                      user={user}
                      share={shareFor(user)}
                      maxShare={maxShare}
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
                  share={shareFor(user)}
                  maxShare={maxShare}
                  isCurrentUser={currentUser?.username === user.username}
                  sortBy={sortBy}
                />
              ))}
            </MobileRankingList>
            {data.pagination.totalPages > 1 && (
              <Pagination>
                <PaginationStatus>
                  Showing {rangeStart.toLocaleString("en-US")}–{rangeEnd.toLocaleString("en-US")} of {data.pagination.totalUsers.toLocaleString("en-US")}
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
        <AdminSection aria-label="Invite members">
          <SectionLabel>
            <SectionTitle>Invite members</SectionTitle>
            <SectionNote>
              Each account can use a link once, even after leaving. Account-specific links allow one join.
            </SectionNote>
          </SectionLabel>
          <Panel>
            <FieldGrid>
              <DitherField label="GitHub username (optional)" description="Any account may join when left empty.">
                <DitherInput
                  value={inviteUsername}
                  onChange={setInviteUsername}
                  name="invite-username"
                  aria-label="Invitee GitHub username"
                  placeholder="Any account"
                />
              </DitherField>
              <DitherField label="Role">
                <Select
                  name="invite-role"
                  aria-label="Invite role"
                  value={group.membership?.role === "owner" ? inviteRole : "member"}
                  options={[
                    { value: "member", label: "Member" },
                    ...(group.membership?.role === "owner" ? [{ value: "admin", label: "Admin" }] : []),
                  ]}
                  onChange={(next) => setInviteRole(next as Exclude<GroupRole, "owner">)}
                />
              </DitherField>
              <DitherField label="Maximum joins">
                <DitherInput
                  type="number"
                  value={inviteUsername.trim() ? "1" : maxUses}
                  onChange={setMaxUses}
                  disabled={!!inviteUsername.trim()}
                  name="invite-max-uses"
                />
              </DitherField>
              <DitherField label="Expires in days">
                <DitherInput
                  type="number"
                  value={expiresInDays}
                  onChange={setExpiresInDays}
                  name="invite-expires"
                />
              </DitherField>
            </FieldGrid>
            <InviteActions>
              <PrimaryButton type="button" disabled={inviteBusy} onClick={createInvite}>
                {inviteBusy ? "Saving…" : "Create invite"}
              </PrimaryButton>
              <SectionNote>Save the link when it appears; it cannot be retrieved later.</SectionNote>
            </InviteActions>
            {inviteError && <ErrorText role="alert">{inviteError}</ErrorText>}
            {inviteUrl && (
              <LinkBox>
                <LinkText>{inviteUrl}</LinkText>
                <SecondaryButton type="button" onClick={copyInvite} aria-label="Copy invite link">
                  {copied ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
                  {copied ? "Copied" : "Copy"}
                </SecondaryButton>
              </LinkBox>
            )}
            <InviteActions>
              <SecondaryButton type="button" onClick={() => void loadInvites()} disabled={inviteBusy}>
                Refresh invites
              </SecondaryButton>
              <SectionNote>Showing the latest 100 invites.</SectionNote>
            </InviteActions>
            {inviteListError && <ErrorText role="alert">{inviteListError}</ErrorText>}
            <InviteList aria-label="Recent invites">
              {invites.length === 0 && (
                <InviteRow>
                  <InviteRowText>No invites yet. Create one to share this group.</InviteRowText>
                </InviteRow>
              )}
              {invites.map((invite) => {
                const expired = new Date(invite.expiresAt).getTime() <= Date.now();
                const active = invite.status === "pending" && !expired;
                return (
                  <InviteRow key={invite.id}>
                    <InviteRowText>
                      <InviteRowStrong>
                        {invite.invitedUsername ? `@${invite.invitedUsername}` : "Any account"}
                      </InviteRowStrong>
                      {` · ${invite.role} · ${invite.useCount}/${invite.maxUses} joined`}
                      <br />
                      {invite.status === "pending" && expired ? "expired" : invite.status} · expires {new Date(invite.expiresAt).toLocaleString()}
                    </InviteRowText>
                    {active && (
                      <SecondaryButton type="button" disabled={inviteBusy} onClick={() => void revokeInvite(invite.id)}>
                        Revoke
                      </SecondaryButton>
                    )}
                  </InviteRow>
                );
              })}
            </InviteList>
          </Panel>
        </AdminSection>
      )}

      {canInvite && (
        <GroupReportingSettings
          slug={group.slug}
          timeZone={group.timeZone}
          weekStartsOn={group.weekStartsOn}
          onChanged={refreshGroup}
        />
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
