"use client";

import { useState, useEffect, useRef } from "react";
import { useRouter } from "next/navigation";
import styled from "styled-components";
import { Pagination, Avatar } from "@primer/react";
import { TabBar } from "@/components/TabBar";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { formatCurrency, formatNumber } from "@/lib/utils";

const Section = styled.div`
  margin-bottom: 40px;
`;

const Title = styled.h1`
  font-size: 30px;
  font-weight: bold;
  margin-bottom: 8px;
`;

const Description = styled.p`
  margin-bottom: 24px;
`;

const StatsGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 12px;
  
  @media (min-width: 768px) {
    display: flex;
  }
`;

const StatCard = styled.div`
  flex: 1;
  border-radius: 12px;
  border: 1px solid;
  padding: 12px;
`;

const StatLabel = styled.p`
  font-size: 12px;
`;

const StatValue = styled.p`
  font-size: 16px;
  font-weight: bold;
`;

const TabSection = styled.div`
  margin-bottom: 24px;
`;

const TableContainer = styled.div`
  border-radius: 16px;
  border: 1px solid;
  overflow: hidden;
`;

const EmptyState = styled.div`
  padding: 32px;
  text-align: center;
`;

const EmptyMessage = styled.p`
  margin-bottom: 16px;
`;

const EmptyHint = styled.p`
  font-size: 14px;
`;

const CodeSnippet = styled.code`
  padding-left: 8px;
  padding-right: 8px;
  padding-top: 4px;
  padding-bottom: 4px;
  border-radius: 4px;
`;

const TableWrapper = styled.div`
  overflow-x: auto;
`;

const Table = styled.table`
  width: 100%;
  min-width: 500px;
`;

const TableHead = styled.thead`
  border-bottom: 1px solid;
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
`;

const TableBody = styled.tbody``;

const TableRow = styled.tr`
  cursor: pointer;
  transition: background-color 0.2s;
  
  &:hover {
    background-color: rgba(20, 26, 33, 0.6);
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
`;

const RankBadge = styled.span`
  font-size: 16px;
  font-weight: bold;
  
  @media (min-width: 640px) {
    font-size: 18px;
  }
`;

const UserContainer = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  
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
  
  @media (min-width: 640px) {
    font-size: 14px;
    max-width: none;
  }
`;

const StatSpan = styled.span`
  font-weight: 500;
  font-size: 14px;
  
  @media (min-width: 640px) {
    font-size: 16px;
  }
`;

const TokenValue = styled.span`
  font-weight: 500;
  font-size: 14px;
  color: var(--color-primary);
  transition: color 0.12s ease;
  
  @media (min-width: 640px) {
    font-size: 16px;
  }
  
  ${TableRow}:hover & {
    color: #0073FF;
  }
`;

const PaginationContainer = styled.div`
  padding-left: 12px;
  padding-right: 12px;
  padding-top: 12px;
  padding-bottom: 12px;
  border-top: 1px solid;
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
  
  @media (min-width: 640px) {
    font-size: 14px;
    text-align: left;
  }
`;

const CTASection = styled.div`
  margin-top: 32px;
  padding: 24px;
  border-radius: 16px;
  border: 1px solid;
`;

const CTATitle = styled.h2`
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 12px;
`;

const CTADescription = styled.p`
  margin-bottom: 16px;
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
  font-family: "Inconsolata", monospace;
  font-size: 16px;
  font-weight: 500;
  letter-spacing: -0.8px;
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

export type Period = "all" | "month" | "week";

export interface LeaderboardUser {
  rank: number;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number;
  totalCost: number;
  submissionCount: number;
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
    totalSubmissions: number;
    uniqueUsers: number;
  };
  period: Period;
}

interface LeaderboardClientProps {
  initialData: LeaderboardData;
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

export default function LeaderboardClient({ initialData }: LeaderboardClientProps) {
  const router = useRouter();
  const [data, setData] = useState<LeaderboardData>(initialData);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [period, setPeriod] = useState<Period>(initialData.period);
  const [page, setPage] = useState(initialData.pagination.page);

  const isFirstMount = useRef(true);

  const fetchData = (targetPeriod: Period, targetPage: number, signal?: AbortSignal) => {
    setIsLoading(true);
    setError(null);
    fetch(`/api/leaderboard?period=${targetPeriod}&page=${targetPage}&limit=50`, { signal })
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.json();
      })
      .then((result) => {
        if (!isValidLeaderboardData(result)) {
          throw new Error("Invalid response format");
        }
        setData(result);
        setIsLoading(false);
      })
      .catch((err) => {
        if (err.name !== "AbortError") {
          setError(err.message || "Failed to load");
          setIsLoading(false);
        }
      });
  };

  useEffect(() => {
    if (isFirstMount.current) {
      isFirstMount.current = false;
      return;
    }

    const abortController = new AbortController();
    fetchData(period, page, abortController.signal);
    return () => abortController.abort();
  }, [period, page]);

  useEffect(() => {
    if (data.pagination.totalPages > 0 && page > data.pagination.totalPages) {
      setPage(data.pagination.totalPages);
    }
  }, [data.pagination.totalPages, page]);

  return (
    <>
      <Section>
        <Title style={{ color: "var(--color-fg-default)" }}>
          Leaderboard
        </Title>
        <Description style={{ color: "var(--color-fg-muted)" }}>
          See who&apos;s using the most tokens
        </Description>

        <StatsGrid>
          <StatCard
            style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
          >
            <StatLabel style={{ color: "var(--color-fg-muted)" }}>
              Users
            </StatLabel>
            <StatValue style={{ color: "var(--color-fg-default)" }}>
              {data.stats.uniqueUsers}
            </StatValue>
          </StatCard>
          <StatCard
            style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
          >
            <StatLabel style={{ color: "var(--color-fg-muted)" }}>
              Total Tokens
            </StatLabel>
            <StatValue
              style={{ color: "var(--color-primary)", textDecoration: "none" }}
              title={data.stats.totalTokens.toLocaleString('en-US')}
            >
              {formatNumber(data.stats.totalTokens)}
            </StatValue>
          </StatCard>
          <StatCard
            style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
          >
            <StatLabel style={{ color: "var(--color-fg-muted)" }}>
              Total Cost
            </StatLabel>
            <StatValue
              style={{ color: "var(--color-fg-default)", textDecoration: "none" }}
              title={data.stats.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 })}
            >
              {formatCurrency(data.stats.totalCost)}
            </StatValue>
          </StatCard>
        </StatsGrid>
      </Section>

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
          }}
        />
      </TabSection>

      {isLoading ? (
        <LeaderboardSkeleton />
      ) : error ? (
        <TableContainer
          style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
        >
          <EmptyState>
            <EmptyMessage style={{ color: "var(--color-fg-muted)" }}>
              Failed to load leaderboard
            </EmptyMessage>
            <EmptyHint style={{ color: "var(--color-fg-subtle)" }}>
              {error}
            </EmptyHint>
            <button
              onClick={() => fetchData(period, page)}
              style={{
                marginTop: 16,
                padding: "8px 16px",
                backgroundColor: "var(--color-primary)",
                color: "#fff",
                border: "none",
                borderRadius: 8,
                cursor: "pointer",
              }}
            >
              Retry
            </button>
          </EmptyState>
        </TableContainer>
      ) : (
        <TableContainer
          style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
        >
          {data.users.length === 0 ? (
            <EmptyState>
              <EmptyMessage style={{ color: "var(--color-fg-muted)" }}>
                No submissions yet. Be the first!
              </EmptyMessage>
              <EmptyHint style={{ color: "var(--color-fg-subtle)" }}>
                Run{" "}
                <CodeSnippet
                  style={{ backgroundColor: "var(--color-bg-subtle)" }}
                >
                  tokscale login && tokscale submit
                </CodeSnippet>
              </EmptyHint>
            </EmptyState>
          ) : (
            <>
              <TableWrapper>
                <Table>
                  <TableHead
                    style={{ backgroundColor: "var(--color-bg-elevated)", borderColor: "var(--color-border-default)" }}
                  >
                    <tr>
                      <TableHeaderCell
                        style={{ color: "var(--color-fg-muted)" }}
                      >
                        Rank
                      </TableHeaderCell>
                      <TableHeaderCell
                        style={{ color: "var(--color-fg-muted)" }}
                      >
                        User
                      </TableHeaderCell>
                      <TableHeaderCell
                        className="text-right"
                        style={{ color: "var(--color-fg-muted)" }}
                      >
                        Cost
                      </TableHeaderCell>
                      <TableHeaderCell
                        className="text-right"
                        style={{ color: "var(--color-fg-muted)" }}
                      >
                        Tokens
                      </TableHeaderCell>
                      <TableHeaderCell
                        className="text-right hidden-mobile w-24"
                        style={{ color: "var(--color-fg-muted)" }}
                      >
                        Submits
                      </TableHeaderCell>
                    </tr>
                  </TableHead>
                  <TableBody>
                    {data.users.map((user, index) => (
                      <TableRow
                        key={user.userId}
                        onClick={() => router.push(`/u/${user.username}`)}
                        style={{
                          borderBottom: index < data.users.length - 1 ? "1px solid var(--color-border-default)" : "none",
                        }}
                      >
                        <TableCell>
                          <RankBadge
                            style={{
                              color:
                                user.rank === 1
                                  ? "#EAB308"
                                  : user.rank === 2
                                  ? "#9CA3AF"
                                  : user.rank === 3
                                  ? "#D97706"
                                  : "var(--color-fg-muted)",
                            }}
                          >
                            #{user.rank}
                          </RankBadge>
                        </TableCell>
                        <TableCell>
                          <UserContainer>
                            <Avatar
                              src={user.avatarUrl || `https://github.com/${user.username}.png`}
                              alt={user.username}
                              size={40}
                            />
                            <UserInfo>
                              <UserDisplayName
                                style={{ color: "var(--color-fg-default)" }}
                              >
                                {user.displayName || user.username}
                              </UserDisplayName>
                              <Username
                                style={{ color: "var(--color-fg-muted)" }}
                              >
                                @{user.username}
                              </Username>
                            </UserInfo>
                          </UserContainer>
                        </TableCell>
                        <TableCell className="text-right">
                          <StatSpan
                            style={{ color: "var(--color-fg-default)", textDecoration: "none" }}
                            title={user.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 })}
                          >
                            {formatCurrency(user.totalCost)}
                          </StatSpan>
                        </TableCell>
                        <TableCell className="text-right">
                          <TokenValue
                            title={user.totalTokens.toLocaleString('en-US')}
                          >
                            {user.totalTokens.toLocaleString('en-US')}
                          </TokenValue>
                        </TableCell>
                        <TableCell className="text-right hidden-mobile w-24">
                          <span style={{ color: "var(--color-fg-muted)" }}>{user.submissionCount}</span>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableWrapper>

              {data.pagination.totalPages > 1 && (
                <PaginationContainer
                  style={{ borderColor: "var(--color-border-default)" }}
                >
                  <PaginationText style={{ color: "var(--color-fg-muted)" }}>
                    Showing {(data.pagination.page - 1) * data.pagination.limit + 1}-
                    {Math.min(data.pagination.page * data.pagination.limit, data.pagination.totalUsers)} of{" "}
                    {data.pagination.totalUsers}
                  </PaginationText>
                  <Pagination
                    pageCount={data.pagination.totalPages}
                    currentPage={data.pagination.page}
                    onPageChange={(_, pageNum) => setPage(pageNum)}
                    showPages={{ narrow: false, regular: true, wide: true }}
                  />
                </PaginationContainer>
              )}
            </>
          )}
        </TableContainer>
      )}

      <CTASection
        style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
      >
        <CTATitle style={{ color: "var(--color-fg-default)" }}>
          Join the Leaderboard
        </CTATitle>
        <CTADescription style={{ color: "var(--color-fg-muted)" }}>
          Install Tokscale CLI and submit your usage data:
        </CTADescription>
        <CodeBlock>
          <CodeLine style={{ backgroundColor: "var(--color-bg-subtle)" }}>
            <CommandPrompt>$</CommandPrompt>
            <CommandPrefix>bunx</CommandPrefix>
            <CommandName>tokscale</CommandName>
            <CommandArg>login</CommandArg>
          </CodeLine>
          <CodeLine style={{ backgroundColor: "var(--color-bg-subtle)" }}>
            <CommandPrompt>$</CommandPrompt>
            <CommandPrefix>bunx</CommandPrefix>
            <CommandName>tokscale</CommandName>
            <CommandArg>submit</CommandArg>
          </CodeLine>
        </CodeBlock>
      </CTASection>
    </>
  );
}
