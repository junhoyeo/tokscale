"use client";

import { useState, useEffect, useRef, useMemo, memo, useCallback } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { CopyIcon, CheckIcon, SearchIcon, XIcon } from "@/components/ui/Icons";
import { TabBar } from "@/components/TabBar";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { formatCurrency, formatNumber } from "@/lib/utils";
import { useSettings } from "@/lib/useSettings";
import { Switch } from "@/components/Switch";

const Section = styled.div`
  margin-bottom: 40px;
`;

const Title = styled.h1`
  font-size: 30px;
  font-weight: bold;
  margin-bottom: 8px;
  color: var(--color-fg-default);
`;

const Description = styled.p`
  margin-bottom: 24px;
  color: var(--color-fg-muted);
`;

const StatsGrid = styled.div`
  display: grid;
  grid-template-columns: 1fr;
  gap: 8px;
  
  @media (min-width: 480px) {
    grid-template-columns: repeat(2, 1fr);
    gap: 12px;
  }
  
  @media (min-width: 768px) {
    display: flex;
  }
`;

const StatCard = styled.div`
  flex: 1;
  border-radius: 12px;
  border: 1px solid var(--color-border-default);
  padding: 12px;
  background-color: var(--color-bg-default);
`;

const StatLabel = styled.p`
  font-size: 12px;
  color: var(--color-fg-muted);
`;

const StatValue = styled.p`
  font-size: 16px;
  font-weight: bold;
  color: var(--color-fg-default);
`;

const StatValuePrimary = styled(StatValue)`
  color: var(--color-primary);
`;

const TabSection = styled.div`
  margin-bottom: 24px;
`;

const TableContainer = styled.div`
  border-radius: 16px;
  border: 1px solid var(--color-border-default);
  overflow: hidden;
  background-color: var(--color-bg-default);
`;

const EmptyState = styled.div`
  padding: 32px;
  text-align: center;
`;

const EmptyMessage = styled.p`
  margin-bottom: 16px;
  color: var(--color-fg-muted);
`;

const EmptyHint = styled.p`
  font-size: 14px;
  color: var(--color-fg-subtle);
`;

const RetryButton = styled.button`
  margin-top: 16px;
  padding: 8px 16px;
  background-color: var(--color-primary);
  color: #fff;
  border: none;
  border-radius: 8px;
  cursor: pointer;
`;

const CodeSnippet = styled.code`
  padding-left: 8px;
  padding-right: 8px;
  padding-top: 4px;
  padding-bottom: 4px;
  border-radius: 4px;
  background-color: var(--color-bg-subtle);
`;

const TableWrapper = styled.div`
  overflow-x: auto;
`;

const Table = styled.table`
  width: 100%;
  min-width: 500px;
  
  @media (max-width: 560px) {
    min-width: unset;
  }
`;

const TableHead = styled.thead`
  border-bottom: 1px solid var(--color-border-default);
  background-color: var(--color-bg-elevated);
`;

const TableHeaderCell = styled.th`
  padding-left: 12px;
  padding-right: 12px;
  padding-top: 12px;
  padding-bottom: 12px;
  text-align: left;
  font-size: 12px;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--color-fg-muted);
  
  @media (max-width: 480px) {
    padding-left: 8px;
    padding-right: 8px;
    padding-top: 8px;
    padding-bottom: 8px;
  }
  
  @media (min-width: 640px) {
    padding-left: 24px;
    padding-right: 24px;
  }
  
  &.text-right {
    text-align: right;
  }
  
  &.hidden-mobile {
    display: none;
    
    @media (min-width: 768px) {
      display: table-cell;
    }
  }
  
  &.hidden-cost-mobile {
    @media (max-width: 560px) {
      display: none;
    }
  }
  
  &.w-24 {
    width: 96px;
  }
  
  &.rank-cell {
    width: 1%;
    white-space: nowrap;
    
    @media (max-width: 560px) {
      padding-left: 8px;
      padding-right: 4px;
    }
  }
`;

const TableBody = styled.tbody``;

const TableRow = styled.tr`
  cursor: pointer;
  transition: all 0.2s;
  position: relative;
  
  &:hover {
    background-color: rgba(20, 26, 33, 0.6);
  }

  &[data-current-user="true"] {
    background: rgba(0, 115, 255, 0.05);
    box-shadow: inset 4px 0 0 #0073FF, inset 0 0 0 2px #0073FF;
    border-radius: 4px;
    
    &:hover {
      background-color: rgba(0, 115, 255, 0.12);
    }
  }
`;

const TableCell = styled.td`
  padding-left: 12px;
  padding-right: 12px;
  padding-top: 10px;
  padding-bottom: 10px;
  white-space: nowrap;
  vertical-align: middle;
  
  @media (max-width: 480px) {
    padding-left: 8px;
    padding-right: 8px;
    padding-top: 8px;
    padding-bottom: 8px;
  }
  
  @media (min-width: 640px) {
    padding-left: 24px;
    padding-right: 24px;
  }
  
  &.text-right {
    text-align: right;
  }
  
  &.hidden-mobile {
    display: none;
    
    @media (min-width: 768px) {
      display: table-cell;
    }
  }
  
  &.hidden-cost-mobile {
    @media (max-width: 560px) {
      display: none;
    }
  }
  
  &.w-24 {
    width: 96px;
  }
  
  &.rank-cell {
    width: 1%;
    white-space: nowrap;
    
    @media (max-width: 560px) {
      padding-left: 8px;
      padding-right: 4px;
    }
  }
`;

const RankBadge = styled.span`
  font-size: 16px;
  font-weight: bold;
  color: var(--color-fg-muted);
  
  &[data-rank="1"] { color: #EAB308; }
  &[data-rank="2"] { color: #9CA3AF; }
  &[data-rank="3"] { color: #D97706; }
  
  @media (max-width: 480px) {
    font-size: 14px;
  }
  
  @media (min-width: 640px) {
    font-size: 18px;
  }
`;

const UserContainer = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  
  @media (max-width: 480px) {
    gap: 6px;
    
    img {
      width: 32px !important;
      height: 32px !important;
    }
  }
  
  @media (min-width: 640px) {
    gap: 12px;
  }
`;

const UserInfo = styled.div`
  min-width: 0;
`;

const UserDisplayName = styled.p`
  font-weight: 500;
  font-size: 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 120px;
  color: var(--color-fg-default);
  
  @media (max-width: 480px) {
    max-width: 80px;
    font-size: 13px;
  }
  
  @media (min-width: 640px) {
    font-size: 16px;
    max-width: none;
  }
`;

const Username = styled.p`
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 120px;
  color: var(--color-fg-muted);
  
  @media (max-width: 480px) {
    max-width: 80px;
    font-size: 11px;
  }
  
  @media (min-width: 640px) {
    font-size: 14px;
    max-width: none;
  }
`;

const StatSpan = styled.span`
  font-weight: 500;
  font-size: 14px;
  color: var(--color-fg-default);
  
  @media (min-width: 640px) {
    font-size: 16px;
  }
`;

const TokenValue = styled.span`
  font-weight: 500;
  font-size: 14px;
  color: var(--color-primary);
  transition: color 0.12s ease;
  
  @media (max-width: 480px) {
    font-size: 13px;
  }
  
  @media (min-width: 640px) {
    font-size: 16px;
  }
  
  ${TableRow}:hover & {
    color: #0073FF;
  }
`;

const TokenValueFull = styled.span`
  display: none;
  
  @media (min-width: 768px) {
    display: inline;
  }
`;

const TokenValueAbbrev = styled.span`
  display: inline;
  
  @media (min-width: 768px) {
    display: none;
  }
`;

const CombinedValueContainer = styled.div`
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  
  @media (min-width: 561px) {
    display: block;
  }
`;

const CostValue = styled.span`
  font-weight: 400;
  font-size: 12px;
  color: var(--color-fg-muted);
  
  @media (min-width: 561px) {
    display: none;
  }
`;

const SubmitCount = styled.span`
  color: var(--color-fg-muted);
`;

const PaginationContainer = styled.div`
  padding-left: 12px;
  padding-right: 12px;
  padding-top: 12px;
  padding-bottom: 12px;
  border-top: 1px solid var(--color-border-default);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  
  @media (min-width: 640px) {
    padding-left: 24px;
    padding-right: 24px;
    padding-top: 16px;
    padding-bottom: 16px;
    flex-direction: row;
  }
`;

const PaginationText = styled.p`
  font-size: 12px;
  text-align: center;
  color: var(--color-fg-muted);
  
  @media (min-width: 640px) {
    font-size: 14px;
    text-align: left;
  }
`;

const CTASection = styled.div`
  margin-top: 32px;
  padding: 24px;
  border-radius: 16px;
  border: 1px solid var(--color-border-default);
  background-color: var(--color-bg-default);
`;

const CTATitle = styled.h2`
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 12px;
  color: var(--color-fg-default);
`;

const CTADescription = styled.p`
  margin-bottom: 16px;
  color: var(--color-fg-muted);
`;

const CodeBlock = styled.div`
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-family: monospace;
  font-size: 14px;
`;

const CodeLine = styled.div`
  padding: 12px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  font-size: 16px;
  font-weight: 500;
  letter-spacing: -0.8px;
  background-color: var(--color-bg-subtle);

  * {
    font-family: "Inconsolata", monospace !important;
  }
`;

const CommandPrompt = styled.span`
  color: #4B6486;
  margin-right: 8px;
`;

const CommandPrefix = styled.span`
  color: #FFF;
  &::after {
    content: " ";
    white-space: pre;
  }
`;

const CommandName = styled.span`
  background: linear-gradient(90deg, #0CF 0%, #0073FF 100%);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
`;

const CommandArg = styled.span`
  color: #FFF;
  &::before {
    content: " ";
    white-space: pre;
  }
`;

const CopyIconButton = styled.button`
  display: flex;
  align-items: center;
  justify-content: center;
  margin-left: auto;
  padding: 6px;
  border: none;
  background: transparent;
  color: #4B6486;
  cursor: pointer;
  border-radius: 4px;
  transition: all 150ms;
  flex-shrink: 0;

  &:hover {
    color: #FFF;
    background: rgba(255, 255, 255, 0.1);
  }

  &.copied {
    color: #3FB950;
  }
`;

const CurrentUserCard = styled.div`
  margin-bottom: 24px;
  padding: 16px;
  border-radius: 12px;
  border: 2px solid #0073FF;
  background: rgba(0, 115, 255, 0.05);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;

  @media (max-width: 640px) {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }
`;

const CurrentUserInfo = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
  min-width: 0;
`;

const CurrentUserDetails = styled.div`
  min-width: 0;
  flex: 1;
`;

const CurrentUserName = styled.p`
  font-weight: 600;
  font-size: 16px;
  color: var(--color-fg-default);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const CurrentUserUsername = styled.p`
  font-size: 14px;
  color: var(--color-fg-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const CurrentUserStats = styled.div`
  display: flex;
  gap: 24px;
  align-items: center;

  @media (max-width: 640px) {
    justify-content: space-between;
  }
`;

const CurrentUserStat = styled.div`
  text-align: right;

  @media (max-width: 640px) {
    text-align: left;
  }
`;

const CurrentUserStatLabel = styled.p`
  font-size: 12px;
  color: var(--color-fg-muted);
  margin-bottom: 4px;
`;

const CurrentUserStatValue = styled.p`
  font-size: 16px;
  font-weight: 600;
  color: #0073FF;
`;

const ErrorBanner = styled.div`
  margin-bottom: 24px;
  padding: 12px 16px;
  border-radius: 8px;
  border: 1px solid #F85149;
  background: rgba(248, 81, 73, 0.1);
  color: #F85149;
  font-size: 14px;
  display: flex;
  align-items: center;
  gap: 8px;
`;

const SortLabel = styled.span`
  font-size: 12px;
  color: var(--color-fg-muted);
  font-weight: 500;
`;

const SearchSortRow = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 16px;

  @media (max-width: 560px) {
    flex-direction: column;
    align-items: stretch;
  }
`;

const SearchInputWrapper = styled.div`
  position: relative;
  flex: 1;
  max-width: 320px;

  @media (max-width: 560px) {
    max-width: none;
  }
`;

const SearchInputIcon = styled.span`
  position: absolute;
  left: 12px;
  top: 50%;
  transform: translateY(-50%);
  color: var(--color-fg-muted);
  pointer-events: none;
  display: flex;
  align-items: center;
`;

const SearchInput = styled.input`
  width: 100%;
  padding: 8px 36px 8px 36px;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  background-color: var(--color-bg-subtle);
  color: var(--color-fg-default);
  font-size: 14px;
  outline: none;
  transition: border-color 0.15s ease;

  &::placeholder {
    color: var(--color-fg-muted);
  }

  &:focus {
    border-color: #0073FF;
    box-shadow: 0 0 0 3px rgba(0, 115, 255, 0.15);
  }
`;

const ClearSearchButton = styled.button`
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px;
  border: none;
  background: transparent;
  color: var(--color-fg-muted);
  cursor: pointer;
  border-radius: 4px;
  transition: color 0.15s ease;

  &:hover {
    color: var(--color-fg-default);
  }
`;

const SortToggleInner = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
`;

const HoverTooltip = styled.span`
  position: relative;
  cursor: default;

  &::after {
    content: attr(data-tooltip);
    position: absolute;
    bottom: calc(100% + 8px);
    left: 50%;
    transform: translateX(-50%);
    background-color: #111B2C;
    color: #e5e5e5;
    border-radius: 8px;
    padding: 8px 12px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0;
    white-space: nowrap;
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.4), 0 0 0 1px rgba(255, 255, 255, 0.06);
    z-index: 1000;
    pointer-events: none;
    opacity: 0;
    transition: opacity 0.15s ease;
  }

  &:hover::after {
    opacity: 1;
  }
`;

const PaginationNav = styled.nav`
  display: flex;
  align-items: center;
  gap: 4px;
`;

const PageButton = styled.button<{ $active?: boolean }>`
  min-width: 32px;
  height: 32px;
  padding: 0 8px;
  border-radius: 6px;
  border: 1px solid ${({ $active }) => $active ? '#0073FF' : 'var(--color-border-default)'};
  background: ${({ $active }) => $active ? '#0073FF' : 'transparent'};
  color: ${({ $active }) => $active ? '#fff' : 'var(--color-fg-muted)'};
  font-size: 13px;
  cursor: pointer;
  transition: all 150ms;
  display: flex;
  align-items: center;
  justify-content: center;

  &:hover:not(:disabled) {
    border-color: #0073FF;
    color: ${({ $active }) => $active ? '#fff' : 'var(--color-fg-default)'};
  }

  &:disabled {
    opacity: 0.4;
    cursor: default;
  }
`;

const PageEllipsis = styled.span`
  min-width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-fg-muted);
  font-size: 13px;
`;

const PaginationPages = styled.div`
  display: none;
  gap: 4px;

  @media (min-width: 768px) {
    display: flex;
  }
`;

export type Period = "all" | "month" | "week";

export interface LeaderboardUser {
  rank: number;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number;
  totalCost: number;
  submissionCount: number | null;
  lastSubmission: string;
}

export interface LeaderboardData {
  users: LeaderboardUser[];
  pagination: {
    page: number;
    limit: number;
    totalUsers: number;
    totalPages: number;
    hasNext: boolean;
    hasPrev: boolean;
  };
  stats: {
    totalTokens: number;
    totalCost: number;
    totalSubmissions: number | null;
    uniqueUsers: number;
  };
  period: Period;
  sortBy?: 'tokens' | 'cost';
}

interface LeaderboardClientProps {
  initialData: LeaderboardData;
  currentUser: { id: string; username: string; displayName: string | null; avatarUrl: string | null } | null;
  initialSortBy: 'tokens' | 'cost';
  initialUserRank: LeaderboardUser | null;
}

function isValidLeaderboardData(data: unknown): data is LeaderboardData {
  return (
    typeof data === "object" &&
    data !== null &&
    "users" in data &&
    "pagination" in data &&
    "stats" in data &&
    Array.isArray((data as LeaderboardData).users)
  );
}

interface LeaderboardRowProps {
  user: LeaderboardUser;
  isCurrentUser: boolean;
  isLastRow: boolean;
  showSubmissionCount: boolean;
  onRowClick: (username: string) => void;
}

const LeaderboardRow = memo(function LeaderboardRow({
  user,
  isCurrentUser,
  isLastRow,
  showSubmissionCount,
  onRowClick,
}: LeaderboardRowProps) {
  const formattedTokens = useMemo(() => user.totalTokens.toLocaleString('en-US'), [user.totalTokens]);
  const formattedCost = useMemo(() => user.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 }), [user.totalCost]);
  
  return (
    <TableRow
      onClick={() => onRowClick(user.username)}
      data-current-user={isCurrentUser}
      style={isLastRow ? undefined : { borderBottom: "1px solid var(--color-border-default)" }}
    >
      <TableCell className="rank-cell">
        <RankBadge data-rank={user.rank <= 3 ? user.rank : undefined}>
          #{user.rank}
        </RankBadge>
      </TableCell>
      <TableCell>
        <UserContainer>
          <img
            src={user.avatarUrl || `https://github.com/${user.username}.png`}
            alt={user.username}
            width={40}
            height={40}
            style={{ borderRadius: "50%", objectFit: "cover", flexShrink: 0, boxShadow: "0 0 0 1px rgba(255, 255, 255, 0.1)" }}
          />
          <UserInfo>
            <UserDisplayName>
              {user.displayName || user.username}
            </UserDisplayName>
            <Username>
              @{user.username}
            </Username>
          </UserInfo>
        </UserContainer>
      </TableCell>
      <TableCell className="text-right hidden-cost-mobile">
        <StatSpan title={formattedCost}>
          {formatCurrency(user.totalCost)}
        </StatSpan>
      </TableCell>
      <TableCell className="text-right">
        <CombinedValueContainer>
          <TokenValue title={formattedTokens}>
            <TokenValueFull>{formattedTokens}</TokenValueFull>
            <TokenValueAbbrev>{formatNumber(user.totalTokens)}</TokenValueAbbrev>
          </TokenValue>
          <CostValue title={formattedCost}>
            {formatCurrency(user.totalCost)}
          </CostValue>
        </CombinedValueContainer>
      </TableCell>
      {showSubmissionCount && (
        <TableCell className="text-right hidden-mobile w-24">
          <SubmitCount>{user.submissionCount ?? "—"}</SubmitCount>
        </TableCell>
      )}
    </TableRow>
  );
});

export default function LeaderboardClient({ initialData, currentUser, initialSortBy, initialUserRank }: LeaderboardClientProps) {
  const router = useRouter();
  const [data, setData] = useState<LeaderboardData>(initialData);
  const [error, setError] = useState<string | null>(null);
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);
  const [period, setPeriod] = useState<Period>(initialData.period);
  const [page, setPage] = useState(initialData.pagination.page);
  const [currentUserRank, setCurrentUserRank] = useState<LeaderboardUser | null>(initialUserRank);
  const [currentUserRankError, setCurrentUserRankError] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [retryToken, setRetryToken] = useState(0);
  const [resolvedRequest, setResolvedRequest] = useState({
    period: initialData.period,
    page: initialData.pagination.page,
    sortBy: initialSortBy,
    search: "",
    retryToken: 0,
  });

  const { leaderboardSortBy, setLeaderboardSort, mounted } = useSettings();

  const effectiveSortBy = mounted ? leaderboardSortBy : initialSortBy;
  const requestedPage = data.pagination.totalPages > 0
    ? Math.min(page, data.pagination.totalPages)
    : page;
  const isLoading =
    period !== resolvedRequest.period
    || requestedPage !== resolvedRequest.page
    || effectiveSortBy !== resolvedRequest.sortBy
    || debouncedSearch !== resolvedRequest.search
    || retryToken !== resolvedRequest.retryToken;

  const isFirstRankFetch = useRef(true);

  // Debounce search input — skip setPage(1) on initial mount with empty query
  const isSearchMounted = useRef(false);
  useEffect(() => {
    if (!isSearchMounted.current) {
      isSearchMounted.current = true;
      if (!searchQuery) return;
    }
    const timer = setTimeout(() => {
      setDebouncedSearch(searchQuery);
      setPage(1);
    }, 300);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  useEffect(() => {
    if (!currentUser) {
      return;
    }

    if (isFirstRankFetch.current) {
      isFirstRankFetch.current = false;
      return;
    }

    const abortController = new AbortController();

    fetch(`/api/leaderboard/user/${currentUser.username}?period=${period}&sortBy=${effectiveSortBy}`, {
      signal: abortController.signal,
    })
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.json();
      })
      .then((userData) => {
        setCurrentUserRank(userData);
        setCurrentUserRankError(false);
      })
      .catch((err) => {
        if (err.name !== "AbortError") {
          setCurrentUserRank(null);
          setCurrentUserRankError(true);
        }
      });

    return () => abortController.abort();
  }, [currentUser, period, effectiveSortBy]);

  const fetchData = useCallback((
    targetPeriod: Period,
    targetPage: number,
    targetSortBy: "tokens" | "cost",
    targetSearch: string,
    targetRetryToken: number,
    signal?: AbortSignal,
  ) => {
    const searchParam = targetSearch ? `&search=${encodeURIComponent(targetSearch)}` : "";
    fetch(`/api/leaderboard?period=${targetPeriod}&page=${targetPage}&limit=50&sortBy=${targetSortBy}${searchParam}`, { signal })
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.json();
      })
      .then((result) => {
        if (!isValidLeaderboardData(result)) {
          throw new Error("Invalid response format");
        }
        setData(result);
        setError(null);
        setResolvedRequest({
          period: targetPeriod,
          page: result.pagination.page,
          sortBy: targetSortBy,
          search: targetSearch,
          retryToken: targetRetryToken,
        });
      })
      .catch((err) => {
        if (err.name !== "AbortError") {
          setError(err.message || "Failed to load");
          setResolvedRequest({
            period: targetPeriod,
            page: targetPage,
            sortBy: targetSortBy,
            search: targetSearch,
            retryToken: targetRetryToken,
          });
        }
      });
  }, []);

  useEffect(() => {
    if (!isLoading) {
      return;
    }

    const abortController = new AbortController();
    fetchData(period, requestedPage, effectiveSortBy, debouncedSearch, retryToken, abortController.signal);
    return () => abortController.abort();
  }, [debouncedSearch, effectiveSortBy, fetchData, isLoading, period, requestedPage, retryToken]);

  const sortedUsers = data.users || [];
  const showSubmissionCount = period === "all";

  const handleCopyCommand = (command: string) => {
    navigator.clipboard.writeText(command);
    setCopiedCommand(command);
    setTimeout(() => setCopiedCommand(null), 2000);
  };

  const handleRowClick = useCallback((username: string) => {
    router.push(`/u/${username}`);
  }, [router]);

  return (
    <>
      <Section>
        <Title>Leaderboard</Title>
        <Description>See who&apos;s using the most tokens</Description>

        <StatsGrid>
          <StatCard>
            <StatLabel>Users</StatLabel>
            <StatValue>{data.stats.uniqueUsers}</StatValue>
          </StatCard>
          <StatCard>
            <StatLabel>Total Tokens</StatLabel>
            <StatValuePrimary>
              <HoverTooltip data-tooltip={data.stats.totalTokens.toLocaleString('en-US')}>
                {formatNumber(data.stats.totalTokens)}
              </HoverTooltip>
            </StatValuePrimary>
          </StatCard>
          <StatCard>
            <StatLabel>Total Cost</StatLabel>
            <StatValue>
              <HoverTooltip data-tooltip={data.stats.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 })}>
                {formatCurrency(data.stats.totalCost)}
              </HoverTooltip>
            </StatValue>
          </StatCard>
        </StatsGrid>
      </Section>

      {currentUser && currentUserRankError && (
        <ErrorBanner>
          <span>⚠️</span>
          <span>Unable to load your ranking. Please refresh the page.</span>
        </ErrorBanner>
      )}

      {currentUser && currentUserRank && (
        <CurrentUserCard>
          <CurrentUserInfo>
            <img
              src={currentUser.avatarUrl || `https://github.com/${currentUser.username}.png`}
              alt={currentUser.username}
              width={48}
              height={48}
              style={{ borderRadius: "50%", objectFit: "cover", flexShrink: 0, boxShadow: "0 0 0 1px rgba(255, 255, 255, 0.1)" }}
            />
            <CurrentUserDetails>
              <CurrentUserName>
                {currentUser.displayName || currentUser.username}
              </CurrentUserName>
              <CurrentUserUsername>
                @{currentUser.username}
              </CurrentUserUsername>
            </CurrentUserDetails>
          </CurrentUserInfo>
          <CurrentUserStats>
            <CurrentUserStat>
              <CurrentUserStatLabel>Your Rank</CurrentUserStatLabel>
              <CurrentUserStatValue>#{currentUserRank.rank}</CurrentUserStatValue>
            </CurrentUserStat>
            <CurrentUserStat>
              <CurrentUserStatLabel>Tokens</CurrentUserStatLabel>
              <CurrentUserStatValue>
                <HoverTooltip data-tooltip={currentUserRank.totalTokens.toLocaleString('en-US')}>
                  {formatNumber(currentUserRank.totalTokens)}
                </HoverTooltip>
              </CurrentUserStatValue>
            </CurrentUserStat>
            <CurrentUserStat>
              <CurrentUserStatLabel>Cost</CurrentUserStatLabel>
              <CurrentUserStatValue>
                <HoverTooltip data-tooltip={currentUserRank.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 })}>
                  {formatCurrency(currentUserRank.totalCost)}
                </HoverTooltip>
              </CurrentUserStatValue>
            </CurrentUserStat>
          </CurrentUserStats>
        </CurrentUserCard>
      )}

      <TabSection>
        <TabBar
          tabs={[
            { id: "all" as Period, label: "All Time" },
            { id: "month" as Period, label: "This Month" },
            { id: "week" as Period, label: "This Week" },
          ]}
          activeTab={period}
          onTabChange={(tab) => {
            setPeriod(tab);
            setPage(1);
            if (tab !== "all") {
              setSearchQuery("");
              setDebouncedSearch("");
            }
          }}
        />
      </TabSection>

      <SearchSortRow>
        {period === "all" && (
          <SearchInputWrapper>
            <SearchInputIcon>
              <SearchIcon size={16} />
            </SearchInputIcon>
            <SearchInput
              type="text"
              placeholder="Search users..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
            {searchQuery && (
              <ClearSearchButton onClick={() => setSearchQuery("")} aria-label="Clear search">
                <XIcon size={16} />
              </ClearSearchButton>
            )}
          </SearchInputWrapper>
        )}
        <SortToggleInner>
          <SortLabel>Sort by:</SortLabel>
          <Switch
            checked={effectiveSortBy === 'cost'}
            onChange={(checked) => setLeaderboardSort(checked ? 'cost' : 'tokens')}
            leftLabel="Tokens"
            rightLabel="Cost"
          />
        </SortToggleInner>
      </SearchSortRow>

      {isLoading ? (
        <LeaderboardSkeleton />
      ) : error ? (
        <TableContainer>
          <EmptyState>
            <EmptyMessage>Failed to load leaderboard</EmptyMessage>
            <EmptyHint>{error}</EmptyHint>
            <RetryButton onClick={() => setRetryToken((prev) => prev + 1)}>
              Retry
            </RetryButton>
          </EmptyState>
        </TableContainer>
      ) : (
        <TableContainer>
          {data.users.length === 0 ? (
            <EmptyState>
              {debouncedSearch ? (
                <>
                  <EmptyMessage>No users found for &ldquo;{debouncedSearch}&rdquo;</EmptyMessage>
                  <EmptyHint>Try a different search term</EmptyHint>
                </>
              ) : (
                <>
                  <EmptyMessage>No submissions yet. Be the first!</EmptyMessage>
                  <EmptyHint>
                    Run <CodeSnippet>tokscale login && tokscale submit</CodeSnippet>
                  </EmptyHint>
                </>
              )}
            </EmptyState>
          ) : (
            <>
              <TableWrapper>
                <Table>
                  <TableHead>
                    <tr>
                      <TableHeaderCell className="rank-cell">Rank</TableHeaderCell>
                      <TableHeaderCell>User</TableHeaderCell>
                      <TableHeaderCell className="text-right hidden-cost-mobile">Cost</TableHeaderCell>
                      <TableHeaderCell className="text-right">Tokens</TableHeaderCell>
                      {showSubmissionCount && (
                        <TableHeaderCell className="text-right hidden-mobile w-24">Submits</TableHeaderCell>
                      )}
                    </tr>
                  </TableHead>
                  <TableBody>
                    {sortedUsers.map((user, index) => (
                      <LeaderboardRow
                        key={user.userId}
                        user={user}
                        isCurrentUser={!!(currentUser && user.username === currentUser.username)}
                        isLastRow={index === sortedUsers.length - 1}
                        showSubmissionCount={showSubmissionCount}
                        onRowClick={handleRowClick}
                      />
                    ))}
                  </TableBody>
                </Table>
              </TableWrapper>

              {data.pagination.totalPages > 1 && (
                <PaginationContainer>
                  <PaginationText>
                    Showing {(data.pagination.page - 1) * data.pagination.limit + 1}-
                    {Math.min(data.pagination.page * data.pagination.limit, data.pagination.totalUsers)} of{" "}
                    {data.pagination.totalUsers}
                  </PaginationText>
                  <PaginationNav>
                    <PageButton
                      disabled={data.pagination.page <= 1}
                      onClick={() => setPage(data.pagination.page - 1)}
                      aria-label="Previous page"
                    >
                      ←
                    </PageButton>
                    <PaginationPages>
                      {(() => {
                        const pages: React.ReactNode[] = [];
                        const total = data.pagination.totalPages;
                        const current = data.pagination.page;
                        const delta = 2;
                        const visible = new Set<number>();
                        visible.add(1);
                        visible.add(total);
                        for (let i = Math.max(2, current - delta); i <= Math.min(total - 1, current + delta); i++) {
                          visible.add(i);
                        }

                        const sorted = Array.from(visible).sort((a, b) => a - b);
                        let last = 0;
                        for (const p of sorted) {
                          if (last && p - last > 1) {
                            pages.push(<PageEllipsis key={`e${p}`}>…</PageEllipsis>);
                          }
                          pages.push(
                            <PageButton key={p} $active={p === current} onClick={() => setPage(p)}>
                              {p}
                            </PageButton>
                          );
                          last = p;
                        }
                        return pages;
                      })()}
                    </PaginationPages>
                    <PageButton
                      disabled={data.pagination.page >= data.pagination.totalPages}
                      onClick={() => setPage(data.pagination.page + 1)}
                      aria-label="Next page"
                    >
                      →
                    </PageButton>
                  </PaginationNav>
                </PaginationContainer>
              )}
            </>
          )}
        </TableContainer>
      )}

      <CTASection>
        <CTATitle>Join the Leaderboard</CTATitle>
        <CTADescription>Install Tokscale CLI and submit your usage data:</CTADescription>
        <CodeBlock>
          <CodeLine>
            <CommandPrompt>$</CommandPrompt>
            <CommandPrefix>bunx</CommandPrefix>
            <CommandName>tokscale</CommandName>
            <CommandArg>login</CommandArg>
            <CopyIconButton
              onClick={() => handleCopyCommand("bunx tokscale login")}
              className={copiedCommand === "bunx tokscale login" ? "copied" : ""}
              aria-label="Copy command"
            >
              {copiedCommand === "bunx tokscale login" ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
            </CopyIconButton>
          </CodeLine>
          <CodeLine>
            <CommandPrompt>$</CommandPrompt>
            <CommandPrefix>bunx</CommandPrefix>
            <CommandName>tokscale</CommandName>
            <CommandArg>submit</CommandArg>
            <CopyIconButton
              onClick={() => handleCopyCommand("bunx tokscale submit")}
              className={copiedCommand === "bunx tokscale submit" ? "copied" : ""}
              aria-label="Copy command"
            >
              {copiedCommand === "bunx tokscale submit" ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
            </CopyIconButton>
          </CodeLine>
        </CodeBlock>
      </CTASection>
    </>
  );
}
