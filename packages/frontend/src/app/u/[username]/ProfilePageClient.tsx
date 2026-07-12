"use client";

import { useId, useMemo, useState } from "react";
import Link from "next/link";
import styled from "styled-components";
import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import {
  createContributionRangeOptions,
  getContributionDayForDate,
  getDefaultContributionDate,
  ProfileContributionBreakdown,
  ProfileContributionGraph,
  ProfileDevices,
  ProfileModels,
  ProfileOverview,
  ProfileTabBar,
  ProfileUsageChart,
  reconcileContributionSelectionRange,
  resolveContributionRange,
  resolveContributionSelectedDate,
  TokenBreakdown,
  type ModelUsage,
  type ProfileDevice,
  type ProfileStatsData,
  type ProfileTab,
  type ProfileContributionView,
  type ProfileUser,
} from "@/components/profile";
import type { DailyContribution } from "@/lib/types";
import { formatDuration } from "@/lib/format";
import { useSettings } from "@/lib/useSettings";

type ProfilePeriod = "all" | "week" | "month";

export interface ProfileData {
  user: {
    id: string;
    username: string;
    displayName: string | null;
    avatarUrl: string | null;
    createdAt: string;
    rank: number | null;
  };
  stats: {
    totalTokens: number;
    totalCost: number;
    inputTokens: number;
    outputTokens: number;
    cacheReadTokens: number;
    cacheWriteTokens: number;
    reasoningTokens?: number;
    submissionCount: number;
    activeDays: number;
    totalActiveTimeMs: number;
    sessionCount: number;
  };
  dateRange: {
    start: string | null;
    end: string | null;
  };
  chartRange?: {
    start: string | null;
    end: string | null;
  };
  updatedAt: string | null;
  clients: string[];
  models: string[];
  mcpServers?: string[];
  modelUsage?: ModelUsage[];
  contributions: DailyContribution[];
  period?: ProfilePeriod;
}

function getProfileChartRange(
  period: ProfilePeriod,
  apiRange: ProfileData["dateRange"],
  chartRange: ProfileData["chartRange"],
): { start: string | null; end: string | null } {
  return period === "all" ? (chartRange ?? apiRange) : apiRange;
}

interface ProfilePageClientProps {
  initialData: ProfileData;
  initialDevices?: ProfileDevice[];
  username: string;
}

const EARLY_ADOPTERS = ["code-yeongyu", "gtg7784", "qodot"];

export default function ProfilePageClient({
  initialData,
  initialDevices,
  username,
}: ProfilePageClientProps) {
  const [activeTab, setActiveTab] = useState<ProfileTab>("activity");
  const [contributionView, setContributionView] =
    useState<ProfileContributionView>("2d");
  const {
    paletteName: contributionPalette,
    setPalette: setContributionPalette,
  } = useSettings();
  const contributionBreakdownId = useId();
  const data = initialData;
  const period = data.period ?? "all";
  const rollingChartRange = useMemo(
    () => getProfileChartRange(period, data.dateRange, data.chartRange),
    [period, data.chartRange, data.dateRange],
  );
  const contributionRangeOptions = useMemo(
    () =>
      period === "all"
        ? createContributionRangeOptions(
            data.contributions,
            rollingChartRange.start,
            rollingChartRange.end,
          )
        : [],
    [data.contributions, period, rollingChartRange],
  );
  const [contributionRangeValue, setContributionRangeValue] =
    useState("recent");
  const selectedContributionRange = useMemo(
    () =>
      resolveContributionRange(
        contributionRangeOptions,
        contributionRangeValue,
      ),
    [contributionRangeOptions, contributionRangeValue],
  );
  const chartRange = useMemo(
    () =>
      selectedContributionRange
        ? {
            start: selectedContributionRange.startDate,
            end: selectedContributionRange.endDate,
          }
        : rollingChartRange,
    [rollingChartRange, selectedContributionRange],
  );
  const contributionSelectionEnd = useMemo(() => {
    if (!chartRange.end || !rollingChartRange.end) return chartRange.end;
    return chartRange.end > rollingChartRange.end
      ? rollingChartRange.end
      : chartRange.end;
  }, [chartRange.end, rollingChartRange.end]);
  const contributionRangeIdentity = `${chartRange.start ?? ""}:${chartRange.end ?? ""}`;
  const [contributionSelection, setContributionSelection] = useState<{
    date: string | null;
    rangeIdentity: string;
  }>(() => ({ date: null, rangeIdentity: contributionRangeIdentity }));
  const reconciledContributionSelection = reconcileContributionSelectionRange(
    contributionSelection,
    contributionRangeIdentity,
  );
  if (reconciledContributionSelection !== contributionSelection) {
    setContributionSelection(reconciledContributionSelection);
  }
  const defaultContributionDate = useMemo(
    () =>
      getDefaultContributionDate(
        data.contributions,
        chartRange.start,
        contributionSelectionEnd,
      ),
    [data.contributions, chartRange.start, contributionSelectionEnd],
  );
  const selectedContributionDate = resolveContributionSelectedDate(
    reconciledContributionSelection,
    contributionRangeIdentity,
    defaultContributionDate,
  );
  const selectedContributionDay = useMemo(
    () =>
      getContributionDayForDate(data.contributions, selectedContributionDate),
    [data.contributions, selectedContributionDate],
  );

  const user: ProfileUser = useMemo(
    () => ({
      username: data.user.username,
      displayName: data.user.displayName,
      avatarUrl: data.user.avatarUrl,
      createdAt: data.user.createdAt,
      rank: data.user.rank,
    }),
    [data.user],
  );

  const stats: ProfileStatsData = useMemo(
    () => ({
      totalTokens: data.stats.totalTokens,
      totalCost: data.stats.totalCost,
      inputTokens: data.stats.inputTokens,
      outputTokens: data.stats.outputTokens,
      cacheReadTokens: data.stats.cacheReadTokens,
      cacheWriteTokens: data.stats.cacheWriteTokens,
      reasoningTokens: data.stats.reasoningTokens ?? 0,
      activeDays: data.stats.activeDays,
      submissionCount: data.stats.submissionCount,
      totalActiveTimeMs: data.stats.totalActiveTimeMs,
      sessionCount: data.stats.sessionCount,
    }),
    [data.stats],
  );

  const favoriteModel = useMemo(() => {
    const usage =
      data.modelUsage?.filter((item) => item.model !== "<synthetic>") ?? [];
    return (
      usage.reduce<ModelUsage | null>(
        (best, current) => (!best || current.cost > best.cost ? current : best),
        null,
      )?.model ?? null
    );
  }, [data.modelUsage]);

  const showResubmitBanner =
    EARLY_ADOPTERS.includes(data.user.username) &&
    data.stats.submissionCount === 1;

  return (
    <PageContainer>
      <Navigation />

      <MainContent id="main-content">
        <ContentWrapper>
          {showResubmitBanner && (
            <UpdateNotice role="status">
              <strong>Fresh detail is available.</strong> Re-submit with{" "}
              <code>bunx tokscale submit</code> to add daily model breakdowns.
            </UpdateNotice>
          )}

          <ProfileOverview
            user={user}
            stats={stats}
            lastUpdated={data.updatedAt ?? undefined}
            period={period}
          />

          <ViewControls aria-label="Profile data views">
            <TabsScroller>
              <ProfileTabBar activeTab={activeTab} onTabChange={setActiveTab} />
            </TabsScroller>
            <ProfilePeriodSelector username={username} current={period} />
          </ViewControls>

          {activeTab === "activity" && (
            <TabPanel
              role="tabpanel"
              id="tabpanel-activity"
              aria-labelledby="tab-activity"
            >
              {data.contributions.length > 0 ? (
                <ActivityDashboardGrid>
                  <ContributionColumn>
                    <ContributionArea>
                      <ProfileContributionGraph
                        breakdownId={contributionBreakdownId}
                        contributions={data.contributions}
                        onPaletteChange={setContributionPalette}
                        onRangeChange={setContributionRangeValue}
                        onSelectedDateChange={(date) => {
                          if (date) {
                            setContributionSelection({
                              date,
                              rangeIdentity: contributionRangeIdentity,
                            });
                          }
                        }}
                        onViewChange={setContributionView}
                        paletteName={contributionPalette}
                        persistentSelection
                        rangeStart={chartRange.start}
                        rangeEnd={chartRange.end}
                        rangeOptions={contributionRangeOptions}
                        rangeValue={selectedContributionRange?.value}
                        selectableRangeEnd={contributionSelectionEnd}
                        selectedDate={selectedContributionDate}
                        showBreakdown={false}
                        view={contributionView}
                        description={
                          period === "all"
                            ? selectedContributionRange?.value === "recent"
                              ? "Daily contribution density across the latest 12 months."
                              : `Daily contribution density across ${selectedContributionRange?.label ?? "the selected year"}.`
                            : period === "month"
                              ? "Daily contribution density across the last 30 days."
                              : "Daily contribution density across the last 7 days."
                        }
                      />
                    </ContributionArea>
                    {selectedContributionDay && (
                      <BreakdownArea>
                        <ProfileContributionBreakdown
                          day={selectedContributionDay}
                          id={contributionBreakdownId}
                          paletteName={contributionPalette}
                        />
                      </BreakdownArea>
                    )}
                  </ContributionColumn>
                  <UsageColumn>
                    <UsageArea>
                      <ProfileUsageChart
                        contributions={data.contributions}
                        averageWindowDays={period === "all" ? 30 : 7}
                        rangeStart={rollingChartRange.start}
                        rangeEnd={rollingChartRange.end}
                        description={
                          period === "all"
                            ? "Model activity for the latest 12 months, grouped by provider."
                            : period === "month"
                              ? "Model activity for the last 30 days, grouped by provider."
                              : "Model activity for the last 7 days, grouped by provider."
                        }
                      />
                    </UsageArea>
                    <DetailsArea>
                      <ProfileInsights
                        stats={stats}
                        favoriteModel={favoriteModel}
                        clients={data.clients}
                        models={data.models}
                        mcpServers={data.mcpServers ?? []}
                        period={period}
                      />
                    </DetailsArea>
                    <TokenArea>
                      <TokenBreakdown stats={stats} />
                    </TokenArea>
                  </UsageColumn>
                </ActivityDashboardGrid>
              ) : (
                <EmptyState>
                  <EmptyTitle>No usage history yet</EmptyTitle>
                  <EmptyCopy>
                    Daily activity will appear after the next Tokscale
                    submission.
                  </EmptyCopy>
                </EmptyState>
              )}
            </TabPanel>
          )}

          {activeTab === "models" && (
            <TabPanel
              role="tabpanel"
              id="tabpanel-models"
              aria-labelledby="tab-models"
            >
              <ProfileModels
                models={data.models}
                modelUsage={data.modelUsage}
              />
            </TabPanel>
          )}

          <ProfileDevices devices={initialDevices ?? []} />
        </ContentWrapper>
      </MainContent>

      <ServiceFooter />
    </PageContainer>
  );
}

function ProfileInsights({
  stats,
  favoriteModel,
  clients,
  models,
  mcpServers,
  period,
}: {
  stats: ProfileStatsData;
  favoriteModel: string | null;
  clients: string[];
  models: string[];
  mcpServers: string[];
  period: ProfilePeriod;
}) {
  const rangeLabel =
    period === "all" ? "1y" : period === "month" ? "30d" : "7d";
  const activityLabel = period === "all" ? "all-time" : rangeLabel;
  const insights = [
    {
      label: `Favorite model (${rangeLabel})`,
      value: favoriteModel ?? "Not available",
    },
    {
      label: `Active time (${activityLabel})`,
      value: stats.totalActiveTimeMs
        ? formatDuration(stats.totalActiveTimeMs)
        : "Not available",
    },
    period === "all"
      ? {
          label: "Sessions (all-time)",
          value: stats.sessionCount
            ? stats.sessionCount.toLocaleString("en-US")
            : "Not available",
        }
      : {
          label: "Selected range",
          value: period === "month" ? "Last 30 days" : "Last 7 days",
        },
    {
      label: period === "all" ? "Models (latest)" : `Models (${rangeLabel})`,
      value: models
        .filter((model) => model !== "<synthetic>")
        .length.toLocaleString("en-US"),
    },
  ];

  return (
    <InsightsPanel aria-labelledby="profile-insights-title">
      <InsightsHeader>
        <InsightsTitle id="profile-insights-title">Usage details</InsightsTitle>
        <InsightsDescription>Scope is shown for each fact.</InsightsDescription>
      </InsightsHeader>

      <InsightsGrid>
        {insights.map((item) => (
          <InsightItem key={item.label}>
            <InsightLabel>{item.label}</InsightLabel>
            <InsightValue title={item.value}>{item.value}</InsightValue>
          </InsightItem>
        ))}
      </InsightsGrid>

      {clients.length > 0 && (
        <MetadataList
          label={
            period === "all"
              ? "Providers (latest)"
              : `Providers (${rangeLabel})`
          }
          items={clients}
        />
      )}
      {mcpServers.length > 0 && (
        <MetadataList label="MCP servers (latest)" items={mcpServers} />
      )}
    </InsightsPanel>
  );
}

function MetadataList({ label, items }: { label: string; items: string[] }) {
  return (
    <MetadataRow>
      <MetadataLabel>{label}</MetadataLabel>
      <MetadataItems role="list" aria-label={label}>
        {items.map((item) => (
          <MetadataChip role="listitem" key={item} title={item}>
            {item}
          </MetadataChip>
        ))}
      </MetadataItems>
    </MetadataRow>
  );
}

const PERIOD_OPTIONS: Array<{ value: ProfilePeriod; label: string }> = [
  { value: "all", label: "Lifetime" },
  { value: "month", label: "30d" },
  { value: "week", label: "7d" },
];

function buildProfilePeriodHref(
  username: string,
  period: ProfilePeriod,
): string {
  const basePath = `/u/${encodeURIComponent(username)}`;
  return period === "all" ? basePath : `${basePath}?period=${period}`;
}

function ProfilePeriodSelector({
  username,
  current,
}: {
  username: string;
  current: ProfilePeriod;
}) {
  return (
    <PeriodSelectorContainer aria-label="Usage range">
      {PERIOD_OPTIONS.map((option) => {
        const isActive = current === option.value;
        return (
          <PeriodLink
            key={option.value}
            href={buildProfilePeriodHref(username, option.value)}
            $active={isActive}
            aria-current={isActive ? "page" : undefined}
          >
            {option.label}
          </PeriodLink>
        );
      })}
    </PeriodSelectorContainer>
  );
}

const PageContainer = styled.div`
  isolation: isolate;
  color-scheme: dark;
  min-height: 100dvh;
  display: flex;
  flex-direction: column;
  background: var(--service-canvas);
  color: var(--service-text);
`;

const MainContent = styled.main`
  width: 100%;
  max-width: 1500px;
  flex: 1;
  margin: 0 auto;
  padding: 96px 32px 40px;

  @media (max-width: 520px) {
    padding-right: 16px;
    padding-left: 16px;
    padding-top: 88px;
    padding-bottom: 40px;
  }
`;

const ContentWrapper = styled.div`
  display: flex;
  flex-direction: column;
  gap: 20px;
`;

const UpdateNotice = styled.p`
  margin: 0;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, #f4b740 28%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, #f4b740 9%, transparent);
  color: #f4d38a;
  font-size: 16px;
  line-height: 24px;

  code {
    padding: 2px 5px;
    border-radius: 4px;
    background: color-mix(in srgb, #f4b740 12%, transparent);
    color: inherit;
    font-size: 0.8125rem;
  }

  @media (min-width: 640px) {
    font-size: 14px;
    line-height: 20px;
  }
`;

const ViewControls = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-width: 0;

  @media (max-width: 560px) {
    align-items: flex-start;
    flex-direction: column;
  }
`;

const TabsScroller = styled.div`
  max-width: 100%;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;

  &::-webkit-scrollbar {
    display: none;
  }
`;

const PeriodSelectorContainer = styled.nav`
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--service-border);
  border-radius: 10px;
  background: var(--service-surface-muted);
`;

const PeriodLink = styled(Link)<{ $active: boolean }>`
  position: relative;
  display: inline-flex;
  min-width: 44px;
  height: 28px;
  align-items: center;
  justify-content: center;
  padding: 0 10px;
  border-radius: 7px;
  background: ${({ $active }) =>
    $active ? "var(--service-surface)" : "transparent"};
  color: ${({ $active }) =>
    $active ? "var(--service-text)" : "var(--service-text-muted)"};
  font-size: 13px;
  font-weight: 500;
  text-decoration: none;

  &:hover {
    color: var(--service-text);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    &::after {
      position: absolute;
      inset: 50% auto auto 50%;
      width: max(100%, 48px);
      height: max(100%, 48px);
      content: "";
      transform: translate(-50%, -50%);
    }
  }
`;

const TabPanel = styled.section`
  min-width: 0;
`;

const ActivityDashboardGrid = styled.div`
  display: grid;
  min-width: 0;
  gap: 14px;

  @media (min-width: 1360px) {
    grid-template-columns: minmax(650px, 1fr) minmax(0, 1.2fr);
    align-items: start;
  }
`;

const DashboardArea = styled.div`
  min-width: 0;
`;

const DashboardColumn = styled.div`
  display: grid;
  min-width: 0;
  gap: 14px;
  align-content: start;
`;

const ContributionColumn = styled(DashboardColumn)``;

const UsageColumn = styled(DashboardColumn)``;

const UsageArea = styled(DashboardArea)``;

const ContributionArea = styled(DashboardArea)``;

const BreakdownArea = styled(DashboardArea)``;

const DetailsArea = styled(DashboardArea)``;

const TokenArea = styled(DashboardArea)``;

const EmptyState = styled.div`
  padding: 40px 24px;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  text-align: center;
`;

const EmptyTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 18px;
  font-weight: 500;
`;

const EmptyCopy = styled.p`
  max-width: 42ch;
  margin: 8px auto 0;
  color: var(--service-text-muted);
  font-size: 16px;
  line-height: 24px;

  @media (min-width: 640px) {
    font-size: 14px;
    line-height: 20px;
  }
`;

const InsightsPanel = styled.section`
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
  container-type: inline-size;
`;

const InsightsHeader = styled.div`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--service-border);

  @container (max-width: 28rem) {
    align-items: flex-start;
    flex-direction: column;
    gap: 4px;
  }
`;

const InsightsTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 16px;
  font-weight: 500;
`;

const InsightsDescription = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 13px;
`;

const InsightsGrid = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  margin: 0;
  padding: 0 16px;

  @container (min-width: 42rem) {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
`;

const InsightItem = styled.div`
  min-width: 0;
  padding: 14px 0;
  border-top: 1px solid var(--service-border);

  &:nth-child(-n + 2) {
    border-top: 0;
  }

  &:nth-child(odd) {
    padding-right: 14px;
  }

  &:nth-child(even) {
    padding-left: 14px;
    border-left: 1px solid var(--service-border);
  }

  @container (min-width: 42rem) {
    padding: 14px;
    border-top: 0;
    border-left: 1px solid var(--service-border);

    &:first-child {
      padding-left: 0;
      border-left: 0;
    }

    &:last-child {
      padding-right: 0;
    }
  }
`;

const InsightLabel = styled.dt`
  overflow: hidden;
  color: var(--service-text);
  font-size: 13px;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const InsightValue = styled.dd`
  overflow: hidden;
  margin: 4px 0 0;
  color: var(--service-text-muted);
  font-size: 16px;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (min-width: 640px) {
    font-size: 14px;
  }
`;

const MetadataRow = styled.div`
  display: grid;
  grid-template-columns: 96px minmax(0, 1fr);
  gap: 12px;
  align-items: start;
  padding: 10px 16px;
  border-top: 1px solid var(--service-border);

  @container (max-width: 28rem) {
    grid-template-columns: 1fr;
    gap: 8px;
  }
`;

const MetadataLabel = styled.span`
  padding-top: 4px;
  color: var(--service-text);
  font-size: 13px;
  font-weight: 500;
`;

const MetadataItems = styled.div`
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  gap: 6px;
`;

const MetadataChip = styled.span`
  max-width: 180px;
  overflow: hidden;
  padding: 3px 8px;
  border-radius: 999px;
  background: var(--service-surface-muted);
  color: var(--service-text-muted);
  font-size: 13px;
  text-overflow: ellipsis;
  white-space: nowrap;
`;
