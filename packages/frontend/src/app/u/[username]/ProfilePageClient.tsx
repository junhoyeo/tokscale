"use client";

import { useId, useMemo, useState } from "react";
import styled from "styled-components";
import { DitherTabPanel } from "dither-kit/DitherTabPanel";
import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import {
  createContributionRangeOptions,
  getDefaultContributionDate,
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
import { useSettings } from "@/lib/useSettings";
import { resolveEffectiveTimeZone } from "@/lib/timezone";
import type { ModerationNotice } from "@/lib/moderation/notice";

import { PROFILE_PERIOD_LABELS, type ProfilePeriod } from "@/lib/reportingPeriods";

export interface ProfileData {
  user: {
    id: string;
    username: string;
    displayName: string | null;
    avatarUrl: string | null;
    createdAt: string;
    rank: number | null;
    rankDateRange?: { start: string; end: string } | null;
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
  /** True once any accepted submission carried a backfill provenance tag. */
  hasBackfill?: boolean;
  modelUsage?: ModelUsage[];
  contributions: DailyContribution[];
  period?: ProfilePeriod;
}

// Both branches already end on or after the newest submitted date: the server
// anchors every window to it, so `chartRange.end` and the period `apiRange.end`
// it also fills are both that anchor. Nothing here may depend on the viewer's
// clock — contribution dates are calendar buckets the submitting machine
// already resolved.
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
  /**
   * Populated for every viewer of a hidden account, not just its owner. The
   * server resolves this outside the cached public payload and picks the
   * wording by audience, so the component renders whatever it is handed
   * without deciding who may see it — see lib/moderation/notice.ts.
   */
  moderationNotice?: ModerationNotice | null;
}

const EARLY_ADOPTERS = ["code-yeongyu", "gtg7784", "qodot"];

/**
 * Amber for the two cases the account holder may want to contest, neutral blue
 * when the cause is our own data problem — that one is informational, and
 * dressing it as a warning would imply they did something wrong. The colour
 * carries the same meaning to a visitor, who is reading the same decision.
 */
const ModerationNoticeBanner = styled.div<{ $tone: ModerationNotice["tone"] }>`
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
  padding: 0.875rem 1rem;
  border: 1px solid
    ${({ $tone }) =>
      $tone === "our-fault"
        ? "color-mix(in srgb, var(--service-accent) 40%, transparent)"
        : "color-mix(in srgb, #f4b740 45%, transparent)"};
  border-radius: var(--radius);
  background: ${({ $tone }) =>
    $tone === "our-fault"
      ? "var(--service-accent-soft)"
      : "color-mix(in srgb, #f4b740 9%, transparent)"};

  strong {
    color: var(--service-text);
    font-size: 0.9375rem;
    font-weight: 600;
  }

  span {
    color: var(--service-text-muted);
    font-size: 0.875rem;
    line-height: 1.55;
  }
`;

export default function ProfilePageClient({
  initialData,
  initialDevices,
  username,
  moderationNotice,
}: ProfilePageClientProps) {
  const [activeTab, setActiveTab] = useState<ProfileTab>("activity");
  const [contributionView, setContributionView] =
    useState<ProfileContributionView>("2d");
  const {
    paletteName: contributionPalette,
    setPalette: setContributionPalette,
    timezone: timezonePreference,
    mounted,
  } = useSettings();
  const contributionBreakdownId = useId();
  const data = initialData;
  const period = data.period ?? "all";
  // Absolute instants ("Updated", "Joined") render in the viewer's display
  // timezone; calendar-day buckets never do — see resolveEffectiveTimeZone.
  // UTC before mount so the first client render matches the server markup.
  const effectiveTimeZone = mounted
    ? resolveEffectiveTimeZone(timezonePreference)
    : "UTC";
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
  const user: ProfileUser = useMemo(
    () => ({
      username: data.user.username,
      displayName: data.user.displayName,
      avatarUrl: data.user.avatarUrl,
      createdAt: data.user.createdAt,
      rank: data.user.rank,
      rankDateRange: data.user.rankDateRange,
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

  const modelsCount = useMemo(
    () => data.models.filter((model) => model !== "<synthetic>").length,
    [data.models],
  );

  const showResubmitBanner =
    EARLY_ADOPTERS.includes(data.user.username) &&
    data.stats.submissionCount === 1;

  return (
    <div className="service-page-shell">
      <Navigation />

      <main className="service-main" id="main-content">
        <Stack>
          {moderationNotice && (
            <ModerationNoticeBanner role="status" $tone={moderationNotice.tone}>
              <strong>{moderationNotice.title}</strong>
              <span>{moderationNotice.message}</span>
            </ModerationNoticeBanner>
          )}

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
            timeZone={effectiveTimeZone}
          />

          {data.hasBackfill && (
            <BackfillBadge
              role="note"
              title="Totals include usage imported from a third-party export via `tokscale import`, not only locally-scanned CLI sessions."
            >
              Includes imported history
            </BackfillBadge>
          )}

          <ProfileTabBar activeTab={activeTab} onTabChange={setActiveTab}>
            <DitherTabPanel value="activity">
              {data.contributions.length > 0 ? (
                <ActivityLayout>
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
                        view={contributionView}
                        description={
                          period === "all"
                            ? selectedContributionRange?.value === "recent"
                              ? "Daily contribution density across the latest 12 months."
                              : `Daily contribution density across ${selectedContributionRange?.label ?? "the selected year"}.`
                            : period === "month"
                              ? "Daily contribution density across the last 30 days."
                              : period === "week"
                                ? "Daily contribution density across the last 7 days."
                                : `Daily contribution density for ${PROFILE_PERIOD_LABELS[period].toLowerCase()} (UTC).`
                        }
                      />


                  <UsageColumn>
                    <PanelBand>
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
                              : period === "week"
                                ? "Model activity for the last 7 days, grouped by provider."
                                : `Model activity for ${PROFILE_PERIOD_LABELS[period].toLowerCase()} (UTC), grouped by provider.`
                        }
                      />

                    </PanelBand>
                    <PanelBand>
                      <ProfileInsights
                        stats={stats}
                        favoriteModel={favoriteModel}
                        clients={data.clients}
                        mcpServers={data.mcpServers ?? []}
                        modelsCount={modelsCount}
                        period={period}
                      />
                    </PanelBand>
                    <PanelBand>
                      <TokenBreakdown stats={stats} />
                    </PanelBand>
                  </UsageColumn>
                </ActivityLayout>
              ) : (
                <EmptyState>
                  <EmptyTitle>No usage data in this range</EmptyTitle>
                  <EmptyCopy>
                    Daily activity appears after the next Tokscale submission.
                    Run <code>bunx tokscale submit</code> from the machine you
                    build on.
                  </EmptyCopy>
                </EmptyState>
              )}
            </DitherTabPanel>

            <DitherTabPanel value="models">
              <ModelsPanel>
                <ProfileModels
                  models={data.models}
                  modelUsage={data.modelUsage}
                />
              </ModelsPanel>
            </DitherTabPanel>
          </ProfileTabBar>

          <ProfileDevices devices={initialDevices ?? []} />
        </Stack>
      </main>

      <ServiceFooter />
    </div>
  );
}

function ProfileInsights({
  stats,
  favoriteModel,
  clients,
  mcpServers,
  modelsCount,
  period,
}: {
  stats: ProfileStatsData;
  favoriteModel: string | null;
  clients: string[];
  mcpServers: string[];
  modelsCount: number;
  period: ProfilePeriod;
}) {
  const rangeLabel =
    period === "all" ? "1y" : PROFILE_PERIOD_LABELS[period];
  const insights = [
    {
      label: `Favorite model (${rangeLabel})`,
      value: favoriteModel ?? "Not available",
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
          value:
            period === "month"
              ? "Last 30 days"
              : period === "week"
                ? "Last 7 days"
                : `${rangeLabel} (UTC)`,
        },
    {
      label: period === "all" ? "Models (latest)" : `Models (${rangeLabel})`,
      value: modelsCount.toLocaleString("en-US"),
    },
  ];

  return (
    <InsightsPanel aria-labelledby="profile-insights-title">
      <InsightsHeader>
        <InsightsTitle id="profile-insights-title">Usage details</InsightsTitle>
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

const Stack = styled.div`
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
`;

const UpdateNotice = styled.p`
  margin: 0;
  padding: 0.625rem 0.875rem;
  border: 1px solid color-mix(in srgb, #f4b740 30%, transparent);
  border-radius: var(--radius);
  background: color-mix(in srgb, #f4b740 8%, transparent);
  color: #f4d38a;
  font-size: 0.875rem;
  line-height: 1.5;

  code {
    padding: 1px 5px;
    border: 1px solid color-mix(in srgb, #f4b740 22%, transparent);
    border-radius: 2px;
    color: inherit;
    font-size: 0.8125rem;
  }
`;

const BackfillBadge = styled.span`
  align-self: flex-start;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.4;
`;

const ActivityLayout = styled.div`
  display: grid;
  min-width: 0;
  gap: 0.875rem;
  padding-top: 1.25rem;
  align-items: stretch;

  @media (min-width: 1360px) {
    grid-template-columns: minmax(620px, 1fr) minmax(0, 1.12fr);
  }
`;

const PanelBand = styled.div`
  min-width: 0;
`;

const UsageColumn = styled.div`
  display: grid;
  min-width: 0;
  gap: 0.875rem;
  align-content: start;
`;

const ModelsPanel = styled.div`
  min-width: 0;
  padding-top: 1.25rem;
`;

const EmptyState = styled.div`
  margin-top: 1.25rem;
  padding: 1.75rem 1.25rem;
  border: 1px solid var(--service-border);
  border-radius: var(--radius);
`;

const EmptyTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1.0625rem;
  font-weight: 600;
`;

const EmptyCopy = styled.p`
  max-width: 52ch;
  margin: 0.5rem 0 0;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
  line-height: 1.55;

  code {
    padding: 1px 5px;
    border: 1px solid var(--service-border);
    border-radius: 2px;
    background: var(--service-surface-muted);
    color: var(--service-text);
    font-size: 0.875rem;
  }
`;

const InsightsPanel = styled.section`
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: var(--radius);
  background: var(--service-surface);
  container-type: inline-size;
`;

const InsightsHeader = styled.div`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 1rem;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--service-border);
`;

const InsightsTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const InsightsGrid = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  margin: 0;
  padding: 0 1rem;

  @container (min-width: 42rem) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
`;

const InsightItem = styled.div`
  min-width: 0;
  padding: 0.75rem 0;
  border-top: 1px solid var(--service-border);

  &:nth-child(-n + 2) {
    border-top: 0;
  }

  &:nth-child(odd) {
    padding-right: 0.875rem;
  }

  &:nth-child(even) {
    padding-left: 0.875rem;
    border-left: 1px solid var(--service-border);
  }

  @container (min-width: 42rem) {
    padding: 0.75rem 0.875rem;

    &:first-child {
      padding-left: 0;
    }

    &:nth-child(n + 1) {
      border-top: 0;
    }

    &:nth-child(n + 2) {
      border-left: 1px solid var(--service-border);
    }
  }

  @container (max-width: 28rem) {
    &:nth-child(n + 3) {
      border-top: 1px solid var(--service-border);
    }
  }
`;

const InsightLabel = styled.dt`
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 400;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const InsightValue = styled.dd`
  overflow: hidden;
  margin: 0.25rem 0 0;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const MetadataRow = styled.div`
  display: grid;
  grid-template-columns: 8.5rem minmax(0, 1fr);
  gap: 0.75rem;
  align-items: start;
  padding: 0.625rem 1rem;
  border-top: 1px solid var(--service-border);

  @container (max-width: 28rem) {
    grid-template-columns: 1fr;
    gap: 0.375rem;
  }
`;

const MetadataLabel = styled.span`
  padding-top: 2px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

const MetadataItems = styled.div`
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  gap: 0.375rem 0.75rem;
`;

const MetadataChip = styled.span`
  max-width: 14rem;
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;
