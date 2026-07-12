"use client";

import { useState, useEffect, useRef, useMemo, memo, useCallback } from "react";
import Link from "next/link";
import { useRouter } from "nextjs-toploader/app";
import { useSearchParams, usePathname } from "next/navigation";
import styled from "styled-components";
import { CopyIcon, CheckIcon, SearchIcon, XIcon } from "@/components/ui/Icons";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import {
  MetricItem,
  MetricLabel,
  MetricStrip,
  MetricValue,
  MobileRankingList,
  MobileRankingRow,
  SegmentedControl,
} from "@/components/leaderboard/RankingUI";
import { getLeaderboardPeriodLabel } from "@/components/leaderboard/presentation";
import { formatCurrency, formatNumber, formatDuration } from "@/lib/utils";
import { useSettings } from "@/lib/useSettings";
import { isValidSortBy, type LeaderboardSortBy } from "@/lib/leaderboard/constants";
import { parseCustomDateRange } from "@/lib/leaderboard/dateRange";
import type { LeaderboardData, LeaderboardUser, Period } from "@/lib/leaderboard/types";

const Section = styled.div`
  display: grid;
  gap: 10px;
  margin-bottom: 24px;
`;

const ScopeLabel = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const TabSection = styled.div`
  width: 100%;
  max-width: 100%;
  margin-bottom: 14px;
  overflow: hidden;
`;

const TableContainer = styled.div`
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
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
  display: none;

  @media (min-width: 720px) {
    display: block;
  }
`;

const Table = styled.table`
  width: 100%;
`;

const TableHead = styled.thead`
  border-bottom: 1px solid var(--service-border);
`;

const TableHeaderCell = styled.th`
  padding-left: 12px;
  padding-right: 12px;
  padding-top: 12px;
  padding-bottom: 12px;
  text-align: left;
  font-size: 12px;
  font-weight: 500;
  color: var(--service-text-muted);
  white-space: nowrap;
  
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
  
  &.w-24 {
    width: 96px;
  }
  
  &.rank-cell {
    width: 1%;
    white-space: nowrap;
    
  }
`;

const TableBody = styled.tbody``;

const TableRow = styled.tr`
  cursor: pointer;
  position: relative;
  
  &:hover {
    background: var(--service-surface);
  }

  &:not(:last-child) td {
    border-bottom: 1px solid var(--service-border);
  }

  &[data-current-user="true"] {
    background: var(--service-accent-soft);
    box-shadow: inset 2px 0 0 var(--service-accent);
    
    &:hover {
      background: var(--service-accent-soft);
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
  
  @media (min-width: 640px) {
    padding-left: 18px;
    padding-right: 18px;
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
  
  &.w-24 {
    width: 96px;
  }
  
  &.rank-cell {
    width: 1%;
    white-space: nowrap;
    
  }
`;

const RankBadge = styled.span`
  color: var(--service-text-muted);
  font-size: 0.875rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  
  &[data-rank="1"] { color: #f4c95d; }
  &[data-rank="2"] { color: #c4ccda; }
  &[data-rank="3"] { color: #d99a68; }
`;

const UserContainer = styled(Link)`
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  color: inherit;
  text-decoration: none;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 3px;
  }
`;

const DesktopAvatar = styled.img`
  width: 36px;
  height: 36px;
  flex: 0 0 auto;
  border-radius: 50%;
  object-fit: cover;
  outline: 1px solid var(--service-border);
  outline-offset: -1px;
`;

const UserInfo = styled.div`
  min-width: 0;
`;

const UserDisplayName = styled.p`
  font-weight: 500;
  font-size: 0.875rem;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 120px;
  color: var(--service-text);
  
  @media (min-width: 640px) {
    font-size: 0.9375rem;
    max-width: none;
  }
`;

const Username = styled.p`
  font-size: 0.75rem;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 120px;
  color: var(--service-text-muted);
  
  @media (min-width: 640px) {
    font-size: 0.8125rem;
    max-width: none;
  }
`;

const StatSpan = styled.span`
  font-weight: 500;
  font-size: 0.875rem;
  color: var(--service-text);
  font-variant-numeric: tabular-nums;
  
  @media (min-width: 640px) {
    font-size: 0.9375rem;
  }
`;

const TokenValue = styled.span`
  font-weight: 500;
  font-size: 0.875rem;
  color: var(--service-accent-hover);
  font-variant-numeric: tabular-nums;
  
  @media (min-width: 640px) {
    font-size: 0.9375rem;
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
  padding-top: 24px;
  border-top: 1px solid var(--service-border);
`;

const CTATitle = styled.h2`
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 12px;
  color: var(--service-text);
`;

const CTADescription = styled.p`
  margin-bottom: 16px;
  color: var(--service-text-muted);
`;

const CodeBlock = styled.div`
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 8px;
  font-family: monospace;
  font-size: 14px;
`;

const CodeLine = styled.div`
  width: 100%;
  min-width: 0;
  padding: 12px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  font-size: 16px;
  font-weight: 500;
  letter-spacing: -0.8px;
  border: 1px solid var(--service-border);
  background: var(--service-surface);
  overflow: hidden;

  * {
    font-family: "Inconsolata", monospace !important;
  }
`;

const CommandPrompt = styled.span`
  flex: 0 0 auto;
  color: #4B6486;
  margin-right: 8px;
`;

const CommandPrefix = styled.span`
  flex: 0 0 auto;
  color: #FFF;
  &::after {
    content: " ";
    white-space: pre;
  }
`;

const CommandName = styled.span`
  flex: 0 0 auto;
  background: linear-gradient(90deg, #0CF 0%, #0073FF 100%);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
`;

const CommandArg = styled.span`
  min-width: 0;
  flex: 0 1 auto;
  overflow: hidden;
  color: #FFF;
  text-overflow: ellipsis;
  white-space: nowrap;
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
  border: 1px solid var(--service-accent);
  background: var(--service-accent-soft);
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

const CurrentUserAvatar = styled.img`
  width: 48px;
  height: 48px;
  flex: 0 0 auto;
  border-radius: 50%;
  object-fit: cover;
  outline: 1px solid var(--service-border);
  outline-offset: -1px;
`;

const CurrentUserDetails = styled.div`
  min-width: 0;
  flex: 1;
`;

const CurrentUserName = styled.p`
  font-weight: 600;
  font-size: 16px;
  color: var(--service-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const CurrentUserUsername = styled.p`
  font-size: 14px;
  color: var(--service-text-muted);
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
  color: var(--service-text-muted);
  margin-bottom: 4px;
`;

const CurrentUserStatValue = styled.p`
  font-size: 16px;
  font-weight: 600;
  color: var(--service-accent-hover);
`;

const ErrorBanner = styled.div`
  margin-bottom: 24px;
  padding: 12px 16px;
  border-radius: 8px;
  border: 1px solid rgba(248, 81, 73, 0.55);
  background: rgba(248, 81, 73, 0.1);
  color: #ff8c85;
  font-size: 14px;
  display: flex;
  align-items: center;
  gap: 8px;
`;

const SortLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const SearchSortRow = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;

  @media (max-width: 560px) {
    flex-direction: column;
    align-items: stretch;
  }
`;

const SearchInputWrapper = styled.div`
  position: relative;
  flex: 1;
  max-width: 360px;

  @media (max-width: 560px) {
    max-width: none;
  }
`;

const SearchInputIcon = styled.span`
  position: absolute;
  left: 12px;
  top: 50%;
  transform: translateY(-50%);
  color: var(--service-text-muted);
  pointer-events: none;
  display: flex;
  align-items: center;
`;

const SearchInput = styled.input`
  width: 100%;
  min-height: 36px;
  padding: 0 36px;
  border-radius: 8px;
  border: 1px solid var(--service-border-strong);
  background: var(--service-surface);
  color: var(--service-text);
  font-size: 0.875rem;
  outline: none;

  &::placeholder {
    color: var(--service-text-muted);
  }

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
  color: var(--service-text-muted);
  cursor: pointer;
  border-radius: 4px;
  &:hover {
    color: var(--service-text);
  }
`;

const SortToggleInner = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 0 0 auto;

  @media (max-width: 560px) {
    justify-content: space-between;
  }
`;

const DateRangeRow = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 14px;
  flex-wrap: wrap;
`;

const DateInput = styled.input`
  min-height: 36px;
  padding: 0 10px;
  border-radius: 8px;
  border: 1px solid var(--service-border-strong);
  background: var(--service-surface);
  color: var(--service-text);
  font-size: 0.875rem;
  outline: none;
  min-width: 140px;

  &:focus-visible {
    border-color: var(--service-focus);
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  &::-webkit-calendar-picker-indicator {
    filter: invert(0.7);
    cursor: pointer;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

const DateSeparator = styled.span`
  font-size: 14px;
  color: var(--service-text-muted);
`;

const DateApplyButton = styled.button`
  min-height: 36px;
  padding: 0 12px;
  border-radius: 8px;
  border: 1px solid var(--service-accent);
  background: var(--service-accent);
  color: #fff;
  font-size: 0.875rem;
  font-weight: 500;

  &:hover {
    border-color: var(--service-accent-hover);
    background: var(--service-accent-hover);
  }

  &:disabled {
    opacity: 0.4;
    cursor: default;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
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

interface LeaderboardClientProps {
  initialData: LeaderboardData;
  currentUser: { id: string; username: string; displayName: string | null; avatarUrl: string | null } | null;
  initialSortBy: 'tokens' | 'cost' | 'time';
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
  showTime: boolean;
  onRowClick: (username: string) => void;
}

const LeaderboardRow = memo(function LeaderboardRow({
  user,
  isCurrentUser,
  showTime,
  onRowClick,
}: LeaderboardRowProps) {
  const formattedTokens = useMemo(() => user.totalTokens.toLocaleString('en-US'), [user.totalTokens]);
  const formattedCost = useMemo(() => user.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 }), [user.totalCost]);
  
  return (
    <TableRow
      onClick={() => onRowClick(user.username)}
      data-current-user={isCurrentUser}
    >
      <TableCell className="rank-cell">
        <RankBadge data-rank={user.rank <= 3 ? user.rank : undefined}>
          #{user.rank}
        </RankBadge>
      </TableCell>
      <TableCell>
        <UserContainer
          href={`/u/${user.username}`}
          onClick={(event) => event.stopPropagation()}
          aria-current={isCurrentUser ? "true" : undefined}
        >
          <DesktopAvatar
            src={user.avatarUrl || `https://github.com/${user.username}.png`}
            alt=""
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
      {showTime && (
        <TableCell className="text-right hidden-mobile w-24">
          <StatSpan>{formatDuration(user.totalActiveTimeMs)}</StatSpan>
        </TableCell>
      )}
    </TableRow>
  );
});

function LeaderboardMobileRow({
  user,
  isCurrentUser,
  sortBy,
}: {
  user: LeaderboardUser;
  isCurrentUser: boolean;
  sortBy: LeaderboardSortBy;
}) {
  const primary = sortBy === "cost"
    ? { label: "Cost", value: formatCurrency(user.totalCost) }
    : sortBy === "time"
      ? { label: "Active time", value: formatDuration(user.totalActiveTimeMs) }
      : { label: "Tokens", value: formatNumber(user.totalTokens) };
  const secondary = [
    sortBy !== "tokens" ? `${formatNumber(user.totalTokens)} tokens` : null,
    sortBy !== "cost" ? formatCurrency(user.totalCost) : null,
    sortBy !== "time" ? formatDuration(user.totalActiveTimeMs) : null,
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

const VALID_PERIODS: Period[] = ["all", "month", "last-month", "week", "custom"];

function parsePeriodParam(value: string | null): Period | null {
  if (!value) return null;
  return VALID_PERIODS.includes(value as Period) ? (value as Period) : null;
}

export default function LeaderboardClient({ initialData, currentUser, initialSortBy, initialUserRank }: LeaderboardClientProps) {
  const router = useRouter();
  const searchParams = useSearchParams();
  const pathname = usePathname();

  const urlPeriod = parsePeriodParam(searchParams.get("period"));
  const urlPage = searchParams.get("page") ? Math.max(1, Number(searchParams.get("page")) || 1) : null;
  const sortByParam = searchParams.get("sortBy");
  const urlSortBy = isValidSortBy(sortByParam) ? sortByParam : null;
  const urlFrom = searchParams.get("from") || "";
  const urlTo = searchParams.get("to") || "";
  const urlSearch = searchParams.get("search")?.trim() || "";
  const initialCustomDateRange = parseCustomDateRange(urlPeriod === "custom" ? urlFrom : null, urlPeriod === "custom" ? urlTo : null);

  const [data, setData] = useState<LeaderboardData>(initialData);
  const [error, setError] = useState<string | null>(null);
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);
  // Server/client divergence note: when ?period=custom&from=BAD&to=BAD is
  // requested, the server falls back to period="all" (see page.tsx) while the
  // client keeps period="custom" from the URL. This is intentionally safe
  // because the client will not fire a fetch until the user applies a valid
  // date range (isCustomWithoutDates guard), so no mismatched data is shown.
  const [period, setPeriod] = useState<Period>(initialData.period);
  const [page, setPage] = useState(urlPage || initialData.pagination.page);
  const [currentUserRank, setCurrentUserRank] = useState<LeaderboardUser | null>(initialUserRank);
  const [currentUserRankError, setCurrentUserRankError] = useState(false);
  const [searchQuery, setSearchQuery] = useState(urlSearch);
  const [debouncedSearch, setDebouncedSearch] = useState(urlSearch);
  const [retryToken, setRetryToken] = useState(0);
  const [customFrom, setCustomFrom] = useState(initialCustomDateRange?.from || "");
  const [customTo, setCustomTo] = useState(initialCustomDateRange?.to || "");
  const [appliedFrom, setAppliedFrom] = useState(initialCustomDateRange?.from || "");
  const [appliedTo, setAppliedTo] = useState(initialCustomDateRange?.to || "");
  const [resolvedRequest, setResolvedRequest] = useState({
    period: initialData.period,
    page: initialData.pagination.page,
    sortBy: initialSortBy,
    search: urlSearch,
    retryToken: 0,
    customFrom: initialCustomDateRange?.from || "",
    customTo: initialCustomDateRange?.to || "",
  });

  const { leaderboardSortBy, setLeaderboardSort, mounted } = useSettings();

  // Precedence for the active sort column:
  //   1. URL `?sortBy=` on first paint wins (preserves shareable links), but
  //   2. the moment the user clicks a SortOption, their choice takes over and
  //      stays sticky from the persisted setting (`leaderboardSortBy`).
  // `urlSortOverride` is cleared on user clicks; do not remove that state reset
  // when refactoring or the URL param will silently override every click.
  const [urlSortOverride, setUrlSortOverride] = useState<LeaderboardSortBy | null>(urlSortBy);
  const effectiveSortBy = urlSortOverride
    ? urlSortOverride
    : (mounted ? leaderboardSortBy : initialSortBy);
  const requestedPage = data.pagination.totalPages > 0
    ? Math.min(page, data.pagination.totalPages)
    : page;
  const isCustomWithoutDates = period === "custom" && (!appliedFrom || !appliedTo);
  const isLoading = !isCustomWithoutDates && (
    period !== resolvedRequest.period
    || requestedPage !== resolvedRequest.page
    || effectiveSortBy !== resolvedRequest.sortBy
    || debouncedSearch !== resolvedRequest.search
    || retryToken !== resolvedRequest.retryToken
    || (period === "custom" && (appliedFrom !== resolvedRequest.customFrom || appliedTo !== resolvedRequest.customTo))
  );

  const isFirstRankFetch = useRef(true);
  const isFirstUrlSync = useRef(true);

  useEffect(() => {
    if (isFirstUrlSync.current) {
      isFirstUrlSync.current = false;
      return;
    }
    const params = new URLSearchParams();
    // Preserve ?view= when it's present (e.g. view=users navigated explicitly)
    const currentView = searchParams.get("view");
    if (currentView) params.set("view", currentView);
    if (period !== "all") params.set("period", period);
    if (requestedPage > 1) params.set("page", String(requestedPage));
    if (effectiveSortBy !== "tokens") params.set("sortBy", effectiveSortBy);
    if (period === "custom" && appliedFrom) params.set("from", appliedFrom);
    if (period === "custom" && appliedTo) params.set("to", appliedTo);
    if (debouncedSearch) params.set("search", debouncedSearch);
    const qs = params.toString();
    const url = qs ? `${pathname}?${qs}` : pathname;
    window.history.replaceState(null, "", url);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [period, requestedPage, effectiveSortBy, appliedFrom, appliedTo, pathname, debouncedSearch]);

  // Debounce search input so URL/search sync updates after typing stops.
  const isSearchMounted = useRef(false);
  useEffect(() => {
    if (!isSearchMounted.current) {
      isSearchMounted.current = true;
      return;
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

    const customParams = period === "custom" ? `&from=${appliedFrom}&to=${appliedTo}` : "";
    fetch(`/api/leaderboard/user/${currentUser.username}?period=${period}&sortBy=${effectiveSortBy}${customParams}`, {
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
  }, [currentUser, period, effectiveSortBy, appliedFrom, appliedTo]);

  const fetchData = useCallback((
    targetPeriod: Period,
    targetPage: number,
    targetSortBy: LeaderboardSortBy,
    targetSearch: string,
    targetRetryToken: number,
    signal?: AbortSignal,
    targetCustomFrom?: string,
    targetCustomTo?: string,
  ) => {
    const searchParam = targetSearch ? `&search=${encodeURIComponent(targetSearch)}` : "";
    const customParams = targetPeriod === "custom" && targetCustomFrom && targetCustomTo
      ? `&from=${targetCustomFrom}&to=${targetCustomTo}`
      : "";
    fetch(`/api/leaderboard?period=${targetPeriod}&page=${targetPage}&limit=50&sortBy=${targetSortBy}${searchParam}${customParams}`, { signal })
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
          customFrom: targetCustomFrom || "",
          customTo: targetCustomTo || "",
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
            customFrom: targetCustomFrom || "",
            customTo: targetCustomTo || "",
          });
        }
      });
  }, []);

  useEffect(() => {
    if (!isLoading) {
      return;
    }

    if (period === "custom" && (!appliedFrom || !appliedTo)) {
      return;
    }

    const abortController = new AbortController();
    fetchData(period, requestedPage, effectiveSortBy, debouncedSearch, retryToken, abortController.signal, appliedFrom, appliedTo);
    return () => abortController.abort();
  }, [appliedFrom, appliedTo, debouncedSearch, effectiveSortBy, fetchData, isLoading, period, requestedPage, retryToken]);

  const sortedUsers = data.users || [];
  const showTime = true;

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
        <ScopeLabel>
          {getLeaderboardPeriodLabel(period, appliedFrom, appliedTo)} aggregate
        </ScopeLabel>

        <MetricStrip>
          <MetricItem>
            <MetricLabel>Ranked users</MetricLabel>
            <MetricValue>{data.stats.uniqueUsers.toLocaleString("en-US")}</MetricValue>
          </MetricItem>
          <MetricItem>
            <MetricLabel>Tokens</MetricLabel>
            <MetricValue
              $accent
              aria-label={`Tokens ${data.stats.totalTokens.toLocaleString("en-US")}`}
              title={data.stats.totalTokens.toLocaleString("en-US")}
            >
              <HoverTooltip data-tooltip={data.stats.totalTokens.toLocaleString('en-US')}>
                {formatNumber(data.stats.totalTokens)}
              </HoverTooltip>
            </MetricValue>
          </MetricItem>
          <MetricItem>
            <MetricLabel>Cost</MetricLabel>
            <MetricValue
              aria-label={`Cost ${data.stats.totalCost.toLocaleString("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2 })}`}
              title={data.stats.totalCost.toLocaleString("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2 })}
            >
              <HoverTooltip data-tooltip={data.stats.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 })}>
                {formatCurrency(data.stats.totalCost)}
              </HoverTooltip>
            </MetricValue>
          </MetricItem>
          {data.stats.totalActiveTimeMs !== null && (
            <MetricItem>
              <MetricLabel>Active time</MetricLabel>
              <MetricValue>{formatDuration(data.stats.totalActiveTimeMs)}</MetricValue>
            </MetricItem>
          )}
        </MetricStrip>
      </Section>

      {currentUser && currentUserRankError && (
        <ErrorBanner>
          <span>Unable to load your ranking. Please refresh the page.</span>
        </ErrorBanner>
      )}

      {currentUser && currentUserRank && (
        <CurrentUserCard>
          <CurrentUserInfo>
            <CurrentUserAvatar
              src={currentUser.avatarUrl || `https://github.com/${currentUser.username}.png`}
              alt=""
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
        <SegmentedControl
          label="Leaderboard period"
          options={[
            { value: "all" as Period, label: "All time" },
            { value: "last-month" as Period, label: "Last month" },
            { value: "month" as Period, label: "This month" },
            { value: "week" as Period, label: "This week" },
            { value: "custom" as Period, label: "Custom" },
          ]}
          value={period}
          onChange={(value) => {
            setPeriod(value);
            setPage(1);
            if (value !== "custom") {
              setAppliedFrom("");
              setAppliedTo("");
              setCustomFrom("");
              setCustomTo("");
            }
          }}
        />
      </TabSection>

      {period === "custom" && (
        <DateRangeRow>
          <DateInput
            type="date"
            name="leaderboard-from"
            aria-label="Leaderboard start date"
            value={customFrom}
            onChange={(e) => setCustomFrom(e.target.value)}
            max={customTo || undefined}
          />
          <DateSeparator>~</DateSeparator>
          <DateInput
            type="date"
            name="leaderboard-to"
            aria-label="Leaderboard end date"
            value={customTo}
            onChange={(e) => setCustomTo(e.target.value)}
            min={customFrom || undefined}
          />
          <DateApplyButton
            type="button"
            disabled={!parseCustomDateRange(customFrom, customTo)}
            onClick={() => {
              const parsed = parseCustomDateRange(customFrom, customTo);
              if (!parsed) {
                return;
              }
              setAppliedFrom(parsed.from);
              setAppliedTo(parsed.to);
              setPage(1);
            }}
          >
            Apply
          </DateApplyButton>
        </DateRangeRow>
      )}

      <SearchSortRow>
        <SearchInputWrapper>
          <SearchInputIcon>
            <SearchIcon size={16} />
          </SearchInputIcon>
          <SearchInput
            type="text"
            name="leaderboard-search"
            aria-label="Search leaderboard users"
            placeholder="Search users..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <ClearSearchButton type="button" onClick={() => setSearchQuery("")} aria-label="Clear search">
              <XIcon size={16} />
            </ClearSearchButton>
          )}
        </SearchInputWrapper>
        <SortToggleInner>
          <SortLabel>Sort</SortLabel>
          <SegmentedControl
            label="Leaderboard sort"
            value={effectiveSortBy}
            options={[
              { value: "tokens", label: "Tokens" },
              { value: "cost", label: "Cost" },
              { value: "time", label: "Time" },
            ]}
            onChange={(value) => {
              setUrlSortOverride(null);
              setLeaderboardSort(value);
            }}
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
            <RetryButton type="button" onClick={() => setRetryToken((prev) => prev + 1)}>
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
                      {showTime && (
                        <TableHeaderCell className="text-right hidden-mobile w-24">Time</TableHeaderCell>
                      )}
                    </tr>
                  </TableHead>
                  <TableBody>
                    {sortedUsers.map((user) => (
                      <LeaderboardRow
                        key={user.userId}
                        user={user}
                        isCurrentUser={!!(currentUser && user.username === currentUser.username)}
                        showTime={showTime}
                        onRowClick={handleRowClick}
                      />
                    ))}
                  </TableBody>
                </Table>
              </TableWrapper>

              <MobileRankingList role="list" aria-label="Leaderboard rankings">
                {sortedUsers.map((user) => (
                  <LeaderboardMobileRow
                    key={user.userId}
                    user={user}
                    isCurrentUser={!!(currentUser && user.username === currentUser.username)}
                    sortBy={effectiveSortBy}
                  />
                ))}
              </MobileRankingList>

              {data.pagination.totalPages > 1 && (
                <PaginationContainer>
                  <PaginationText>
                    Showing {(data.pagination.page - 1) * data.pagination.limit + 1}-
                    {Math.min(data.pagination.page * data.pagination.limit, data.pagination.totalUsers)} of{" "}
                    {data.pagination.totalUsers}
                  </PaginationText>
                  <PaginationNav>
                    <PageButton
                      type="button"
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
                            <PageButton type="button" key={p} $active={p === current} onClick={() => setPage(p)}>
                              {p}
                            </PageButton>
                          );
                          last = p;
                        }
                        return pages;
                      })()}
                    </PaginationPages>
                    <PageButton
                      type="button"
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
          {mounted && typeof window !== "undefined" && window.location.hostname !== "tokscale.ai" && (
            <CodeLine>
              <CommandPrompt>$</CommandPrompt>
              <CommandPrefix>export</CommandPrefix>
              <CommandName>TOKSCALE_API_URL</CommandName>
              <CommandArg>={`${window.location.origin}`}</CommandArg>
              <CopyIconButton
                type="button"
                onClick={() => handleCopyCommand(`export TOKSCALE_API_URL=${window.location.origin}`)}
                className={copiedCommand === `export TOKSCALE_API_URL=${window.location.origin}` ? "copied" : ""}
                aria-label="Copy command"
              >
                {copiedCommand === `export TOKSCALE_API_URL=${window.location.origin}` ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
              </CopyIconButton>
            </CodeLine>
          )}
          <CodeLine>
            <CommandPrompt>$</CommandPrompt>
            <CommandPrefix>bunx</CommandPrefix>
            <CommandName>tokscale</CommandName>
            <CommandArg>login</CommandArg>
            <CopyIconButton
              type="button"
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
              type="button"
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
