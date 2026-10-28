"use client";

import { useState, useEffect, useRef, useMemo, memo, useCallback } from "react";
import Link from "next/link";
import { useSearchParams, usePathname } from "next/navigation";
import styled from "styled-components";
import { DitherClipboard } from "dither-kit/DitherClipboard";
import { DitherPagination } from "dither-kit/DitherPagination";
import { DitherSearchInput } from "dither-kit/DitherSearchInput";
import { CheckIcon, CopyIcon } from "@/components/ui/Icons";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import {
  MobileRankingList,
  MobileRankingRow,
  PersonAvatar,
  RankMark,
  SegmentedControl,
  UsageBars,
} from "@/components/leaderboard/RankingUI";
import { getLeaderboardPeriodLabel } from "@/components/leaderboard/presentation";
import { formatCurrency, formatNumber } from "@/lib/utils";
import { useSettings } from "@/lib/useSettings";
import {
  resolveSortByParam,
  type LeaderboardSortBy,
} from "@/lib/leaderboard/constants";
import { parseCustomDateRange } from "@/lib/leaderboard/dateRange";
import type { LeaderboardData, LeaderboardUser, Period } from "@/lib/leaderboard/types";
import type { RecentUsage } from "@/lib/leaderboard/recentUsage";

/* App-screen list pattern: title row (ViewSelector) → underline tabs →
 * toolbar (scope left, search middle, count and sort right) → a table on the
 * page ground with no card around it, numbers right-aligned on tabular
 * figures, whole rows as links. */

const Section = styled.div`
  display: grid;
  gap: 16px;
`;

/* The summary strip: one row split by 1px rules, label above value. On a
 * phone the three cells stay one row (narrow columns, not a stacked list)
 * so the ranking still starts high on the screen. */
const Summary = styled.dl`
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
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
  border-left: 1px solid var(--service-border);

  &:first-child {
    padding-left: 0;
    border-left: 0;
  }

  &:last-child {
    padding-right: 0;
  }

  @media (min-width: 641px) {
    padding: 12px 28px;

    &:first-child {
      padding-left: 0;
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

/* "Your rank": the signed-in viewer's own line on a raised surface, one link
 * to their profile. */
const YourRankCard = styled(Link)`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 0;
  border-radius: var(--radius, 2px);
  background: var(--service-surface-muted);
  color: inherit;
  text-decoration: none;

  &:hover {
    background: var(--v2-row-hover);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }

  @media (max-width: 640px) {
    flex-wrap: wrap;
    gap: 8px 16px;
  }
`;

const YourRankIdentity = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 12px;
`;

const YourRankName = styled.p`
  margin: 0;
  overflow: hidden;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const YourRankUsername = styled.p`
  margin: 0;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const YourRankStats = styled.div`
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 32px;

  @media (max-width: 640px) {
    gap: 24px;
  }
`;

const YourRankStat = styled.div`
  text-align: right;
`;

const YourRankStatLabel = styled.p`
  margin: 0 0 4px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  white-space: nowrap;
`;

const YourRankStatValue = styled.p`
  margin: 0;
  color: var(--service-text);
  font-size: 1.25rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const ErrorBanner = styled.p`
  margin: 0;
  padding: 12px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

/* Toolbar: scope on the left (the date first), search in the middle, the
   count and sort on the right. */
const Toolbar = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px 24px;
  margin: 0 0 14px;
`;

const ToolbarLeft = styled.div`
  display: flex;
  flex: 0 0 auto;
  align-items: center;
`;

const ToolbarRight = styled.div`
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 24px;
`;

const SearchWrap = styled.div`
  flex: 1 1 220px;
  max-width: 340px;

  @media (max-width: 640px) {
    order: 2;
    max-width: none;
    width: 100%;
  }
`;

/* The range is stated again at the foot of the list; on a phone the toolbar
 * keeps the sort control only. */
const CountLine = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;

  @media (max-width: 640px) {
    display: none;
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

const DateRangeRow = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  margin: -2px 0 14px;
  flex-wrap: wrap;
`;

const DateInput = styled.input`
  min-height: 32px;
  padding: 0 10px;
  border: 1px solid var(--service-border-strong);
  border-radius: var(--radius, 2px);
  background: var(--service-surface);
  color: var(--service-text);
  font-size: 0.875rem;
  min-width: 140px;
  color-scheme: dark;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

const DateSeparator = styled.span`
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const DateApplyButton = styled.button`
  min-height: 32px;
  padding: 0 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: var(--radius, 2px);
  background: transparent;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;

  &:hover:not(:disabled) {
    background: var(--service-surface-muted);
  }

  &:disabled {
    opacity: 0.5;
    cursor: default;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
  }
`;

const LoadingNotice = styled.p`
  margin: 0 0 10px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

/* The table sits on the page ground, no card. */
const TableContainer = styled.div`
  border-bottom: 1px solid var(--service-border);
`;

const TableWrapper = styled.div`
  display: none;

  @media (min-width: 720px) {
    display: block;
  }
`;

/* Rows are row links (globals.css); the cells have padding, so the focus ring sits inside. */
const Table = styled.table`
  --row-focus-offset: -2px;

  width: 100%;
`;

const TableHead = styled.thead`
  /* The one heavier line in the table. */
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

  &.text-right {
    text-align: right;
  }

  &.rank-cell {
    width: 72px;
  }

  /* Columns that hold numbers get the width their figure needs at that
   * breakpoint; the identity column takes the rest. */
  &.recent-cell {
    width: 180px;
  }

  &.cost-cell {
    width: 104px;
  }

  &.tokens-cell {
    width: 120px;
  }

  @media (min-width: 1100px) {
    &.recent-cell {
      width: 300px;
    }

    &.cost-cell {
      width: 110px;
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

  &[data-current-user="true"] {
    background: var(--service-surface-muted);
  }

  /* Your own row keeps its tone under the row-link hover. */
  &[data-row-link]:hover {
    background: ${({ $current }) => $current ? "var(--service-surface-muted)" : "var(--v2-row-hover)"};
  }
`;

const TableCell = styled.td`
  padding: 10px 16px;
  white-space: nowrap;
  vertical-align: middle;

  &:first-child {
    padding-left: 0;
  }

  &:last-child {
    padding-right: 0;
  }

  &.text-right {
    text-align: right;
  }
`;

/* Rank as data (a plain number on tabular figures); the top three carry the
 * small dithered mark, denser the higher the rank. */
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
const UserContainer = styled(Link)`
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
  color: inherit;
  text-decoration: none;
`;

const UserInfo = styled.div`
  min-width: 0;
`;

const UserDisplayName = styled.p`
  overflow: hidden;
  font-weight: 500;
  font-size: 0.9375rem;
  text-overflow: ellipsis;
  color: var(--service-text);
  margin: 0;
`;

const Username = styled.p`
  overflow: hidden;
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  color: var(--service-text-muted);
  margin: 2px 0 0;
`;

const YouTag = styled.span`
  margin-left: 6px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 400;
`;

/* Past 30 days: thirty dithered day columns, then the window's total, so the
 * figure column stays aligned and every row's bars start on the same x. */
const RecentCell = styled.span`
  display: flex;
  min-width: 0;
  align-items: flex-end;
  justify-content: flex-end;
  gap: 12px;
`;

const RecentBarsBox = styled.span`
  flex: 1 1 0;
  min-width: 0;
  max-width: 200px;
`;

const RecentValue = styled.span`
  min-width: 52px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  text-align: right;
`;

const StatSpan = styled.span`
  font-weight: 500;
  font-size: 0.875rem;
  color: var(--service-text);
  font-variant-numeric: tabular-nums;
`;

const TokenValue = styled.span`
  font-weight: 500;
  font-size: 0.9375rem;
  color: var(--service-text);
  font-variant-numeric: tabular-nums;
`;

const TokenValueFull = styled.span`
  display: none;

  @media (min-width: 900px) {
    display: inline;
  }
`;

const TokenValueAbbrev = styled.span`
  display: inline;

  @media (min-width: 900px) {
    display: none;
  }
`;

const EmptyState = styled.div`
  padding: 40px 16px;
  border-top: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.875rem;
  text-align: center;
`;

const RetryButton = styled.button`
  margin: 0 0 0 12px;
  padding: 0 2px;
  border: 0;
  background: transparent;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
  text-decoration: underline;
  text-underline-offset: 4px;
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const CodeSnippet = styled.code`
  padding: 1px 6px;
  border-radius: var(--radius, 2px);
  background: var(--service-surface-muted);
  color: var(--service-text);
`;

const PaginationBar = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 0;

  @media (max-width: 640px) {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
`;

const PaginationText = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;

  @media (min-width: 641px) {
    font-size: 0.875rem;
  }
`;

/* The kit's pagination, stripped of its boxes: numbers in tabular figures,
 * the current page marked by the accent line (the same selection mark as
 * every other control on the screen). */
const Pager = styled(DitherPagination)`
  gap: 2px;

  button,
  span {
    height: 32px;
    min-width: 32px;
    border: 0;
    background: transparent;
    font-size: 0.875rem;
  }

  button {
    position: relative;
    color: var(--service-text-muted);
  }

  button[aria-current="page"] {
    color: var(--service-text);

    &::after {
      position: absolute;
      right: 6px;
      bottom: 4px;
      left: 6px;
      height: 2px;
      background: var(--service-accent);
      content: "";
    }
  }

  button:not([disabled]):hover {
    color: var(--service-text);
  }

  button:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }
`;

const CTASection = styled.section`
  margin-top: 40px;
`;

const CTATitle = styled.h2`
  margin: 0;
  font-size: 1.125rem;
  font-weight: 500;
  letter-spacing: -0.01em;
  color: var(--service-text);
`;

const CTADescription = styled.p`
  margin: 6px 0 16px;
  max-width: 68ch;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
`;

const CodeBlock = styled.div`
  display: grid;
  min-width: 0;
  max-width: 720px;
`;

const CodeLine = styled.pre`
  display: flex;
  min-width: 0;
  align-items: center;
  margin: 0;
  padding: 10px 12px;
  border-bottom: 1px solid var(--service-border);
  font-size: 0.8125rem;

  &:first-child {
    border-top: 1px solid var(--service-border);
  }
`;

const Code = styled.code`
  min-width: 0;
  flex: 1 1 auto;
  overflow-x: auto;
  white-space: pre;
`;

const CommandPrompt = styled.span`
  flex: 0 0 auto;
  margin-right: 10px;
  color: var(--service-text-muted);
`;

const CopyCommandButton = styled.button`
  display: grid;
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  place-items: center;
  margin-left: 12px;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--service-text-muted);
  cursor: pointer;

  &:hover {
    color: var(--service-text);
  }

  &.copied {
    color: var(--positive, #44a282);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }

  @media (pointer: coarse) {
    width: 44px;
    height: 44px;
  }
`;

interface LeaderboardClientProps {
  initialData: LeaderboardData;
  /** Past-30-day daily tokens for the users on the first page, read on the server. */
  initialRecentUsage: RecentUsage | null;
  currentUser: { id: string; username: string; displayName: string | null; avatarUrl: string | null } | null;
  initialSortBy: LeaderboardSortBy;
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

function describeRecent(values: number[], start: string): string {
  const total = values.reduce((sum, value) => sum + value, 0);
  if (total <= 0) return "No submitted usage in the past 30 days";
  const active = values.filter((value) => value > 0).length;
  const busiest = values.indexOf(Math.max(...values));
  const busiestDate = new Date(Date.parse(`${start}T00:00:00Z`) + busiest * 86_400_000);
  const busiestLabel = busiestDate.toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
  return `${total.toLocaleString("en-US")} tokens in the past 30 days, active ${active} of 30 days, busiest ${busiestLabel} (${formatNumber(values[busiest])})`;
}

interface LeaderboardRowProps {
  user: LeaderboardUser;
  /** Daily tokens over the past 30 days, oldest first; undefined while loading. */
  recent: number[] | undefined;
  recentStart: string;
  isCurrentUser: boolean;
}

const LeaderboardRow = memo(function LeaderboardRow({
  user,
  recent,
  recentStart,
  isCurrentUser,
}: LeaderboardRowProps) {
  const formattedTokens = useMemo(() => user.totalTokens.toLocaleString('en-US'), [user.totalTokens]);
  const formattedCost = useMemo(() => user.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 }), [user.totalCost]);
  const recentTotal = recent ? recent.reduce((sum, value) => sum + value, 0) : 0;

  return (
    <TableRow data-row-link="" data-current-user={isCurrentUser} $current={isCurrentUser}>
      <TableCell className="rank-cell text-right">
        <RankCell>
          {user.rank <= 3 && <RankMark level={user.rank as 1 | 2 | 3} />}
          {user.rank}
        </RankCell>
      </TableCell>
      <TableCell>
        <UserContainer
          href={`/u/${user.username}`}
          data-row-primary=""
          aria-current={isCurrentUser ? "true" : undefined}
        >
          <PersonAvatar
            src={user.avatarUrl || `https://github.com/${user.username}.png`}
            name={user.displayName || user.username}
            size={36}
          />
          <UserInfo>
            <UserDisplayName>
              {user.displayName || user.username}
              {isCurrentUser && <YouTag>You</YouTag>}
            </UserDisplayName>
            <Username>@{user.username}</Username>
          </UserInfo>
        </UserContainer>
      </TableCell>
      <TableCell className="text-right">
        {recent ? (
          <InstantTooltip as={RecentCell} content={describeRecent(recent, recentStart)}>
            <RecentBarsBox>
              <UsageBars values={recent} label={`${user.username}: ${describeRecent(recent, recentStart)}`} />
            </RecentBarsBox>
            <RecentValue>{recentTotal > 0 ? formatNumber(recentTotal) : "0"}</RecentValue>
          </InstantTooltip>
        ) : (
          <RecentValue aria-label="Loading the past 30 days">—</RecentValue>
        )}
      </TableCell>
      <TableCell className="text-right">
        <InstantTooltip as={StatSpan} content={formattedCost}>
          {formatCurrency(user.totalCost)}
        </InstantTooltip>
      </TableCell>
      <TableCell className="text-right">
        <InstantTooltip as={TokenValue} content={formattedTokens}>
          <TokenValueFull>{formattedTokens}</TokenValueFull>
          <TokenValueAbbrev>{formatNumber(user.totalTokens)}</TokenValueAbbrev>
        </InstantTooltip>
      </TableCell>
    </TableRow>
  );
});

function LeaderboardMobileRow({
  user,
  recent,
  isCurrentUser,
  sortBy,
}: {
  user: LeaderboardUser;
  recent: number[] | undefined;
  isCurrentUser: boolean;
  sortBy: LeaderboardSortBy;
}) {
  const primary = sortBy === "cost"
    ? { label: "Est. cost", value: formatCurrency(user.totalCost) }
    : { label: "Tokens", value: formatNumber(user.totalTokens) };
  const secondary = sortBy === "cost"
    ? `${formatNumber(user.totalTokens)} tokens`
    : `${formatCurrency(user.totalCost)} est. cost`;

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
      recent={recent ?? null}
    />
  );
}

const VALID_PERIODS: Period[] = ["all", "month", "last-month", "week", "custom"];

function parsePeriodParam(value: string | null): Period | null {
  if (!value) return null;
  return VALID_PERIODS.includes(value as Period) ? (value as Period) : null;
}

export default function LeaderboardClient({ initialData, initialRecentUsage, currentUser, initialSortBy }: LeaderboardClientProps) {
  const searchParams = useSearchParams();
  const pathname = usePathname();

  const urlPeriod = parsePeriodParam(searchParams.get("period"));
  const urlPage = searchParams.get("page") ? Math.max(1, Number(searchParams.get("page")) || 1) : null;
  const sortByParam = searchParams.get("sortBy");
  const urlSortBy = resolveSortByParam(sortByParam);
  const urlFrom = searchParams.get("from") || "";
  const urlTo = searchParams.get("to") || "";
  const urlSearch = searchParams.get("search")?.trim() || "";
  const initialCustomDateRange = parseCustomDateRange(urlPeriod === "custom" ? urlFrom : null, urlPeriod === "custom" ? urlTo : null);

  const [data, setData] = useState<LeaderboardData>(initialData);
  const [error, setError] = useState<string | null>(null);
  // Server/client divergence note: when ?period=custom&from=BAD&to=BAD is
  // requested, the server falls back to period="all" (see page.tsx) while the
  // client keeps period="custom" from the URL. This is intentionally safe
  // because the client will not fire a fetch until the user applies a valid
  // date range (isCustomWithoutDates guard), so no mismatched data is shown.
  const [period, setPeriod] = useState<Period>(initialData.period);
  const [page, setPage] = useState(urlPage || initialData.pagination.page);
  const [currentUserRank, setCurrentUserRank] = useState<LeaderboardUser | null>(null);
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

  // Past 30 days for the users on screen. The first page arrives from the
  // server; a new page, period, sort or search fetches the ids it lacks.
  const [recentUsage, setRecentUsage] = useState<RecentUsage | null>(initialRecentUsage);
  const visibleIds = sortedUsers.map((user) => user.userId).join(",");
  useEffect(() => {
    if (!visibleIds) return;
    const ids = visibleIds.split(",");
    if (recentUsage && ids.every((id) => id in recentUsage.byUser)) return;
    const controller = new AbortController();
    fetch(`/api/leaderboard/recent?users=${encodeURIComponent(visibleIds)}`, { signal: controller.signal })
      .then((response) => (response.ok ? (response.json() as Promise<RecentUsage>) : null))
      .then((usage) => {
        if (!usage) return;
        setRecentUsage((previous) =>
          previous && previous.end === usage.end
            ? { ...usage, byUser: { ...previous.byUser, ...usage.byUser } }
            : usage,
        );
      })
      .catch(() => {
        // The column keeps its dash; the table itself is unaffected.
      });
    return () => controller.abort();
  }, [visibleIds, recentUsage]);

  const periodLabel = getLeaderboardPeriodLabel(period, appliedFrom, appliedTo);
  const rangeStart = (data.pagination.page - 1) * data.pagination.limit + 1;
  const rangeEnd = Math.min(data.pagination.page * data.pagination.limit, data.pagination.totalUsers);

  const commands = useMemo(() => {
    const list: { command: string; describe: string }[] = [];
    if (mounted && typeof window !== "undefined" && window.location.hostname !== "tokscale.ai") {
      list.push({
        command: `export TOKSCALE_API_URL=${window.location.origin}`,
        describe: "the API URL export",
      });
    }
    list.push({ command: "bunx tokscale login", describe: "tokscale login" });
    list.push({ command: "bunx tokscale submit", describe: "tokscale submit" });
    return list;
  }, [mounted]);

  return (
    <Section>
      <Summary>
        <SummaryItem>
          <SummaryLabel>Ranked profiles</SummaryLabel>
          <SummaryValue>{data.stats.uniqueUsers.toLocaleString("en-US")}</SummaryValue>
        </SummaryItem>
        <SummaryItem>
          <SummaryLabel>Tokens</SummaryLabel>
          <SummaryValue aria-label={`Tokens ${data.stats.totalTokens.toLocaleString("en-US")}`}>
            <InstantTooltip content={data.stats.totalTokens.toLocaleString("en-US")} describe={false}>
              {formatNumber(data.stats.totalTokens)}
            </InstantTooltip>
          </SummaryValue>
        </SummaryItem>
        <SummaryItem>
          <SummaryLabel>Est. cost</SummaryLabel>
          <SummaryValue
            aria-label={`Estimated cost ${data.stats.totalCost.toLocaleString("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2 })}`}
          >
            <InstantTooltip
              content={data.stats.totalCost.toLocaleString("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2 })}
              describe={false}
            >
              {formatCurrency(data.stats.totalCost)}
            </InstantTooltip>
          </SummaryValue>
        </SummaryItem>
      </Summary>

      {currentUser && currentUserRankError && (
        <ErrorBanner role="status">
          Unable to load your ranking. Please refresh the page.
        </ErrorBanner>
      )}

      {currentUser && currentUserRank && (
        <YourRankCard href={`/u/${currentUser.username}`}>
          <YourRankIdentity>
            <PersonAvatar
              src={currentUser.avatarUrl || `https://github.com/${currentUser.username}.png`}
              name={currentUser.displayName || currentUser.username}
              size={44}
            />
            <div>
              <YourRankName>
                {currentUser.displayName || currentUser.username}
              </YourRankName>
              <YourRankUsername>@{currentUser.username}</YourRankUsername>
            </div>
          </YourRankIdentity>
          <YourRankStats>
            <YourRankStat>
              <YourRankStatLabel>Your rank</YourRankStatLabel>
              <YourRankStatValue>#{currentUserRank.rank}</YourRankStatValue>
            </YourRankStat>
            <YourRankStat>
              <YourRankStatLabel>Tokens</YourRankStatLabel>
              <YourRankStatValue>
                <InstantTooltip content={currentUserRank.totalTokens.toLocaleString("en-US")}>
                  {formatNumber(currentUserRank.totalTokens)}
                </InstantTooltip>
              </YourRankStatValue>
            </YourRankStat>
            <YourRankStat>
              <YourRankStatLabel>Est. cost</YourRankStatLabel>
              <YourRankStatValue>
                <InstantTooltip
                  content={currentUserRank.totalCost.toLocaleString("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2 })}
                >
                  {formatCurrency(currentUserRank.totalCost)}
                </InstantTooltip>
              </YourRankStatValue>
            </YourRankStat>
          </YourRankStats>
        </YourRankCard>
      )}

      <div>
        <Toolbar>
          <ToolbarLeft>
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
          </ToolbarLeft>
          <SearchWrap>
            <DitherSearchInput
              value={searchQuery}
              onChange={setSearchQuery}
              placeholder="Search users"
              aria-label="Search leaderboard users"
              name="leaderboard-search"
            />
          </SearchWrap>
          <ToolbarRight>
            <CountLine>
              {debouncedSearch
                ? `${data.pagination.totalUsers.toLocaleString("en-US")} ${data.pagination.totalUsers === 1 ? "result" : "results"} for “${debouncedSearch}”`
                : `${period === "custom" ? `${periodLabel} · ` : ""}${rangeStart.toLocaleString("en-US")}–${rangeEnd.toLocaleString("en-US")} of ${data.pagination.totalUsers.toLocaleString("en-US")}`}
            </CountLine>
            <SortGroup>
              <SortLabel>Sort</SortLabel>
              <SegmentedControl
                label="Leaderboard sort"
                value={effectiveSortBy}
                options={[
                  { value: "tokens", label: "Tokens" },
                  { value: "cost", label: "Est. cost" },
                ]}
                onChange={(value) => {
                  setUrlSortOverride(null);
                  setLeaderboardSort(value);
                }}
              />
            </SortGroup>
          </ToolbarRight>
        </Toolbar>

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
            <DateSeparator>to</DateSeparator>
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
      </div>

      {isLoading && (
        <LoadingNotice role="status" aria-live="polite">
          Updating leaderboard…
        </LoadingNotice>
      )}

      {error ? (
        <TableContainer>
          <EmptyState role="alert">
            Failed to load the leaderboard.
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
                  No users found for &ldquo;{debouncedSearch}&rdquo;. Try a different search.
                </>
              ) : (
                <>
                  No submissions yet. Be the first: run <CodeSnippet>tokscale login &amp;&amp; tokscale submit</CodeSnippet>
                </>
              )}
            </EmptyState>
          ) : (
            <>
              <TableWrapper>
                <Table>
                  <TableHead>
                    <tr>
                      <TableHeaderCell scope="col" className="rank-cell text-right">Rank</TableHeaderCell>
                      <TableHeaderCell scope="col">User</TableHeaderCell>
                      <TableHeaderCell scope="col" className="recent-cell text-right">Past 30 days</TableHeaderCell>
                      <TableHeaderCell scope="col" className="cost-cell text-right">Est. cost</TableHeaderCell>
                      <TableHeaderCell scope="col" className="tokens-cell text-right">Tokens</TableHeaderCell>
                    </tr>
                  </TableHead>
                  <tbody>
                    {sortedUsers.map((user) => (
                      <LeaderboardRow
                        key={user.userId}
                        user={user}
                        recent={recentUsage?.byUser[user.userId]}
                        recentStart={recentUsage?.start ?? ""}
                        isCurrentUser={!!(currentUser && user.username === currentUser.username)}
                      />
                    ))}
                  </tbody>
                </Table>
              </TableWrapper>

              <MobileRankingList role="list" aria-label="Leaderboard rankings">
                {sortedUsers.map((user) => (
                  <LeaderboardMobileRow
                    key={user.userId}
                    user={user}
                    recent={recentUsage?.byUser[user.userId]}
                    isCurrentUser={!!(currentUser && user.username === currentUser.username)}
                    sortBy={effectiveSortBy}
                  />
                ))}
              </MobileRankingList>

              {data.pagination.totalPages > 1 && (
                <PaginationBar>
                  <PaginationText>
                    Showing {rangeStart.toLocaleString("en-US")}–{rangeEnd.toLocaleString("en-US")} of{" "}
                    {data.pagination.totalUsers.toLocaleString("en-US")}
                  </PaginationText>
                  <Pager
                    page={data.pagination.page}
                    total={data.pagination.totalPages}
                    siblings={1}
                    onPageChange={setPage}
                  />
                </PaginationBar>
              )}
            </>
          )}
        </TableContainer>
      )}

      <CTASection id="submit">
        <CTATitle>Submit your usage</CTATitle>
        <CTADescription>
          Install the Tokscale CLI and submit your usage data to appear on the leaderboard.
        </CTADescription>
        <CodeBlock>
          {commands.map(({ command, describe }) => (
            <CodeLine key={command}>
              <CommandPrompt aria-hidden="true">$</CommandPrompt>
              <DitherClipboard value={command}>
                {({ copied, copy }) => (
                  <>
                    <Code>{command}</Code>
                    <CopyCommandButton
                      type="button"
                      onClick={() => void copy()}
                      className={copied ? "copied" : ""}
                      aria-label={copied ? `Copied ${describe}` : `Copy ${describe}`}
                    >
                      {copied ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
                    </CopyCommandButton>
                  </>
                )}
              </DitherClipboard>
            </CodeLine>
          ))}
        </CodeBlock>
      </CTASection>
    </Section>
  );
}
