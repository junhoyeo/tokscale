"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { CheckIcon, CopyIcon, SearchIcon, XIcon } from "@/components/ui/Icons";
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
  margin: 32px 0 24px;
  display: grid;
  gap: 18px;
`;

const HeaderTop = styled.div`
  display: flex;
  justify-content: space-between;
  gap: 16px;
  align-items: flex-start;

  @media (max-width: 720px) {
    flex-direction: column;
  }
`;

const Identity = styled.div`
  display: flex;
  gap: 14px;
  align-items: center;
`;

const Avatar = styled.div<{ $image?: string | null }>`
  width: 58px;
  height: 58px;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  background:
    ${({ $image }) => $image ? `url(${$image}) center/cover` : "linear-gradient(135deg, #0073ff, #13a10e)"};
  flex: 0 0 auto;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--color-fg-default);
  font-size: 30px;
  font-weight: 700;
`;

const Meta = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 6px;
  color: var(--color-fg-muted);
  font-size: 13px;
`;

const Badge = styled.span`
  display: inline-flex;
  align-items: center;
  min-height: 24px;
  padding: 0 8px;
  border: 1px solid var(--color-border-default);
  border-radius: 999px;
  color: var(--color-fg-muted);
  background: var(--color-bg-subtle);
`;

const Description = styled.p`
  margin: 0;
  color: var(--color-fg-muted);
  line-height: 1.6;
`;

const Actions = styled.div`
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
`;

const Button = styled.button`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 38px;
  padding: 0 14px;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-default);
  color: var(--color-fg-default);
  font-weight: 600;
  cursor: pointer;

  &:disabled {
    opacity: 0.65;
    cursor: not-allowed;
  }
`;

const PrimaryLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 38px;
  padding: 0 14px;
  border-radius: 8px;
  border: 1px solid var(--color-primary);
  background: var(--color-primary);
  color: #fff;
  font-weight: 600;
  text-decoration: none;
`;

const StatsGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 10px;

  @media (max-width: 760px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
`;

const StatCard = styled.div`
  padding: 12px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-default);
`;

const StatLabel = styled.div`
  color: var(--color-fg-muted);
  font-size: 12px;
`;

const StatValue = styled.div`
  margin-top: 4px;
  color: var(--color-fg-default);
  font-weight: 700;
  font-size: 18px;
`;

const InvitePanel = styled.div`
  display: grid;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-default);
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
  min-height: 38px;
  padding: 0 12px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
  color: var(--color-fg-default);
  font: inherit;
`;

const Select = styled.select`
  min-height: 38px;
  padding: 0 12px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
  color: var(--color-fg-default);
  font: inherit;
`;

const LinkBox = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
  color: var(--color-fg-default);
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
  margin: 22px 0 14px;
`;

const Segmented = styled.div`
  display: inline-flex;
  padding: 4px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
`;

const SegmentButton = styled.button<{ $active: boolean }>`
  min-height: 32px;
  padding: 0 12px;
  border: 0;
  border-radius: 6px;
  background: ${({ $active }) => ($active ? "var(--color-bg-default)" : "transparent")};
  color: ${({ $active }) => ($active ? "var(--color-fg-default)" : "var(--color-fg-muted)")};
  font-weight: 600;
  cursor: pointer;
`;

const SearchWrapper = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 38px;
  padding: 0 10px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
`;

const SearchInput = styled.input`
  width: 180px;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--color-fg-default);
  font: inherit;
`;

const TableContainer = styled.div`
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  overflow: hidden;
  background: var(--color-bg-default);
`;

const TableWrapper = styled.div`
  overflow-x: auto;
`;

const Table = styled.table`
  width: 100%;
  min-width: 680px;
`;

const Th = styled.th`
  padding: 12px 16px;
  text-align: left;
  font-size: 12px;
  font-weight: 600;
  color: var(--color-fg-muted);
  background: var(--color-bg-elevated);
  border-bottom: 1px solid var(--color-border-default);

  &.right {
    text-align: right;
  }
`;

const Td = styled.td`
  padding: 12px 16px;
  border-bottom: 1px solid var(--color-border-default);
  color: var(--color-fg-default);

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
`;

const UserAvatar = styled.img`
  width: 34px;
  height: 34px;
  border-radius: 50%;
  object-fit: cover;
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.1);
`;

const Muted = styled.span`
  display: block;
  color: var(--color-fg-muted);
  font-size: 12px;
`;

const EmptyState = styled.div`
  padding: 32px;
  text-align: center;
  color: var(--color-fg-muted);
`;

const ErrorText = styled.p`
  margin: 0;
  color: var(--color-danger-fg, #f85149);
`;

function isAdminRole(role: GroupRole | undefined): boolean {
  return role === "owner" || role === "admin";
}

function roleLabel(role: string): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function GroupRow({
  user,
  showSubmissionCount,
}: {
  user: GroupLeaderboardUser;
  showSubmissionCount: boolean;
}) {
  return (
    <tr>
      <Td>#{user.rank}</Td>
      <Td>
        <UserCell href={`/u/${user.username}`}>
          <UserAvatar src={user.avatarUrl || `https://github.com/${user.username}.png`} alt={user.username} />
          <span>
            {user.displayName || user.username}
            <Muted>@{user.username}</Muted>
          </span>
        </UserCell>
      </Td>
      <Td>{roleLabel(user.role)}</Td>
      <Td className="right">{formatCurrency(user.totalCost)}</Td>
      <Td className="right">{formatNumber(user.totalTokens)}</Td>
      {showSubmissionCount && <Td className="right">{user.submissionCount ?? "-"}</Td>}
    </tr>
  );
}

export default function GroupDetailClient({
  group,
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
  const showSubmissionCount = period === "all";

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
      router.push("/groups");
    }
  }

  const sortedUsers = useMemo(() => data.users || [], [data.users]);

  return (
    <>
      <Header>
        <HeaderTop>
          <Identity>
            <Avatar $image={group.avatarUrl} />
            <div>
              <Title>{group.name}</Title>
              <Meta>
                <Badge>{group.isPublic ? "Public" : "Private"}</Badge>
                <Badge>{group.memberCount} members</Badge>
                {group.membership && <Badge>{roleLabel(group.membership.role)}</Badge>}
              </Meta>
            </div>
          </Identity>
          <Actions>
            <PrimaryLink href="/groups">All groups</PrimaryLink>
            {group.membership && group.membership.role !== "owner" && (
              <Button onClick={leaveGroup}>Leave</Button>
            )}
          </Actions>
        </HeaderTop>
        {group.description && <Description>{group.description}</Description>}

        <StatsGrid>
          <StatCard>
            <StatLabel>Active users</StatLabel>
            <StatValue>{data.stats.activeUsers}</StatValue>
          </StatCard>
          <StatCard>
            <StatLabel>Members</StatLabel>
            <StatValue>{data.stats.totalMembers || group.memberCount}</StatValue>
          </StatCard>
          <StatCard>
            <StatLabel>Total tokens</StatLabel>
            <StatValue>{formatNumber(data.stats.totalTokens)}</StatValue>
          </StatCard>
          <StatCard>
            <StatLabel>Total cost</StatLabel>
            <StatValue>{formatCurrency(data.stats.totalCost)}</StatValue>
          </StatCard>
        </StatsGrid>

        {canInvite && (
          <InvitePanel>
            <InviteForm>
              <Input
                value={inviteUsername}
                onChange={(event) => setInviteUsername(event.target.value)}
                placeholder="GitHub username (optional)"
              />
              <Select
                value={inviteRole}
                onChange={(event) => setInviteRole(event.target.value as Exclude<GroupRole, "owner">)}
              >
                <option value="member">Member</option>
                <option value="admin">Admin</option>
              </Select>
              <Button onClick={createInvite}>Create invite</Button>
            </InviteForm>
            {inviteError && <ErrorText>{inviteError}</ErrorText>}
            {inviteUrl && (
              <LinkBox>
                <LinkText>{inviteUrl}</LinkText>
                <Button onClick={copyInvite} aria-label="Copy invite link">
                  {copied ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
                  {copied ? "Copied" : "Copy"}
                </Button>
              </LinkBox>
            )}
          </InvitePanel>
        )}
      </Header>

      <Toolbar>
        <Segmented aria-label="Period">
          {(["all", "month", "week"] as Period[]).map((value) => (
            <SegmentButton
              key={value}
              $active={period === value}
              onClick={() => {
                setPeriod(value);
                setPage(1);
              }}
            >
              {value === "all" ? "All time" : value === "month" ? "Month" : "Week"}
            </SegmentButton>
          ))}
        </Segmented>

        <Actions>
          <SearchWrapper>
            <SearchIcon size={16} />
            <SearchInput
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search members"
            />
            {search && (
              <Button onClick={() => setSearch("")} aria-label="Clear search">
                <XIcon size={16} />
              </Button>
            )}
          </SearchWrapper>
          <Segmented aria-label="Sort">
            {(["tokens", "cost"] as SortBy[]).map((value) => (
              <SegmentButton
                key={value}
                $active={sortBy === value}
                onClick={() => {
                  setSortBy(value);
                  setPage(1);
                }}
              >
                {value === "tokens" ? "Tokens" : "Cost"}
              </SegmentButton>
            ))}
          </Segmented>
        </Actions>
      </Toolbar>

      <TableContainer>
        {error ? (
          <EmptyState>{error}</EmptyState>
        ) : isLoading ? (
          <EmptyState>Loading leaderboard...</EmptyState>
        ) : sortedUsers.length === 0 ? (
          <EmptyState>No submitted usage for this group yet.</EmptyState>
        ) : (
          <TableWrapper>
            <Table>
              <thead>
                <tr>
                  <Th>Rank</Th>
                  <Th>User</Th>
                  <Th>Role</Th>
                  <Th className="right">Cost</Th>
                  <Th className="right">Tokens</Th>
                  {showSubmissionCount && <Th className="right">Submits</Th>}
                </tr>
              </thead>
              <tbody>
                {sortedUsers.map((user) => (
                  <GroupRow key={user.userId} user={user} showSubmissionCount={showSubmissionCount} />
                ))}
              </tbody>
            </Table>
          </TableWrapper>
        )}
      </TableContainer>
    </>
  );
}
