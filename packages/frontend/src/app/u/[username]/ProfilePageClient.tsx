"use client";

import { useState, useMemo } from "react";
import styled from "styled-components";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import {
  ProfileHeader,
  ProfileTabBar,
  TokenBreakdown,
  ProfileModels,
  ProfileDevices,
  ProfileActivity,
  ProfileEmptyActivity,
  ProfileStats,
  type ProfileUser,
  type ProfileStatsData,
  type ProfileTab,
  type ModelUsage,
  type ProfileDevice,
} from "@/components/profile";
import type { TokenContributionData, DailyContribution, ClientType } from "@/lib/types";

interface ProfileData {
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
    submissionCount: number;
    activeDays: number;
  };
  dateRange: {
    start: string | null;
    end: string | null;
  };
  updatedAt: string | null;
  clients: string[];
  models: string[];
  modelUsage?: ModelUsage[];
  contributions: DailyContribution[];
  devices?: ProfileDevice[];
  deviceContributions?: DeviceContributionGroup[];
}

interface DeviceContributionGroup {
  id: string;
  name: string;
  os: string | null;
  contributions: DailyContribution[];
}

interface ProfilePageClientProps {
  initialData: ProfileData;
  username: string;
}

/** Build the contribution-graph payload from a daily contribution list. */
function buildGraphData(
  contributions: DailyContribution[],
  summary: {
    totalTokens: number;
    totalCost: number;
    activeDays: number;
    clients: string[];
    models: string[];
  },
  dateRange: { start: string | null; end: string | null }
): TokenContributionData | null {
  if (contributions.length === 0) return null;

  const maxCost = Math.max(...contributions.map((c) => c.totals.cost), 0);

  const yearMap = new Map<
    string,
    { totalTokens: number; totalCost: number; start: string; end: string }
  >();
  for (const day of contributions) {
    const year = day.date.split("-")[0];
    const existing = yearMap.get(year);
    if (existing) {
      existing.totalTokens += day.totals.tokens;
      existing.totalCost += day.totals.cost;
      if (day.date < existing.start) existing.start = day.date;
      if (day.date > existing.end) existing.end = day.date;
    } else {
      yearMap.set(year, {
        totalTokens: day.totals.tokens,
        totalCost: day.totals.cost,
        start: day.date,
        end: day.date,
      });
    }
  }

  const years = Array.from(yearMap.entries())
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([year, s]) => ({
      year,
      totalTokens: s.totalTokens,
      totalCost: s.totalCost,
      range: { start: s.start, end: s.end },
    }));

  return {
    meta: {
      generatedAt: new Date().toISOString(),
      version: "1.0.0",
      dateRange: {
        start: dateRange.start || contributions[0]?.date || "",
        end:
          dateRange.end || contributions[contributions.length - 1]?.date || "",
      },
    },
    summary: {
      totalTokens: summary.totalTokens,
      totalCost: summary.totalCost,
      totalDays: contributions.length,
      activeDays: summary.activeDays,
      averagePerDay:
        summary.activeDays > 0 ? summary.totalCost / summary.activeDays : 0,
      maxCostInSingleDay: maxCost,
      clients: summary.clients as ClientType[],
      models: summary.models,
    },
    years,
    contributions,
  };
}

/** Sum per-day stats — used to derive a single device's totals. */
function deriveStatsFromContributions(contributions: DailyContribution[]) {
  let totalTokens = 0;
  let totalCost = 0;
  let inputTokens = 0;
  let outputTokens = 0;
  let cacheReadTokens = 0;
  let cacheWriteTokens = 0;
  let activeDays = 0;
  for (const day of contributions) {
    totalTokens += day.totals.tokens;
    totalCost += day.totals.cost;
    inputTokens += day.tokenBreakdown.input;
    outputTokens += day.tokenBreakdown.output;
    cacheReadTokens += day.tokenBreakdown.cacheRead;
    cacheWriteTokens += day.tokenBreakdown.cacheWrite;
    if (day.totals.tokens > 0) activeDays += 1;
  }
  return {
    totalTokens,
    totalCost,
    inputTokens,
    outputTokens,
    cacheReadTokens,
    cacheWriteTokens,
    activeDays,
  };
}

/** Aggregate per-model usage from a daily contribution list. */
function deriveModelUsage(contributions: DailyContribution[]): {
  models: string[];
  modelUsage: ModelUsage[];
} {
  const map = new Map<string, { tokens: number; cost: number }>();
  const add = (modelId: string, tokens: number, cost: number) => {
    const entry = map.get(modelId) ?? { tokens: 0, cost: 0 };
    entry.tokens += tokens;
    entry.cost += cost;
    map.set(modelId, entry);
  };
  for (const day of contributions) {
    for (const client of day.clients) {
      if (client.models && Object.keys(client.models).length > 0) {
        for (const [modelId, m] of Object.entries(client.models)) {
          add(modelId, m.tokens || 0, m.cost || 0);
        }
      } else if (client.modelId) {
        // Legacy rows carry a single modelId without a nested models map.
        const t = client.tokens;
        add(
          client.modelId,
          (t.input || 0) +
            (t.output || 0) +
            (t.cacheRead || 0) +
            (t.cacheWrite || 0) +
            (t.reasoning || 0),
          client.cost || 0
        );
      }
    }
  }
  const totalCost = Array.from(map.values()).reduce((sum, m) => sum + m.cost, 0);
  const modelUsage = Array.from(map.entries())
    .filter(([model]) => model !== "<synthetic>")
    .map(([model, d]) => ({
      model,
      tokens: d.tokens,
      cost: d.cost,
      percentage: totalCost > 0 ? (d.cost / totalCost) * 100 : 0,
    }))
    .sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
  return { models: modelUsage.map((m) => m.model), modelUsage };
}

/** Total / per-device segmented filter, shared by the Activity and Models tabs. */
function DeviceFilterBar({
  devices,
  selected,
  onSelect,
}: {
  devices: DeviceContributionGroup[];
  selected: string;
  onSelect: (value: string) => void;
}) {
  return (
    <DeviceSelector aria-label="Filter by device">
      <DeviceSelectorButton
        type="button"
        data-active={selected === "total"}
        onClick={() => onSelect("total")}
      >
        Total
      </DeviceSelectorButton>
      {devices.map((device) => (
        <DeviceSelectorButton
          key={device.id}
          type="button"
          data-active={selected === device.id}
          onClick={() => onSelect(device.id)}
        >
          {device.name}
        </DeviceSelectorButton>
      ))}
    </DeviceSelector>
  );
}

export default function ProfilePageClient({ initialData, username }: ProfilePageClientProps) {
  const [activeTab, setActiveTab] = useState<ProfileTab>("activity");
  const data = initialData;

  const [selectedDevice, setSelectedDevice] = useState<string>("total");

  const deviceContributions: DeviceContributionGroup[] = useMemo(
    () => data.deviceContributions ?? [],
    [data]
  );
  const hasDeviceBreakdown = deviceContributions.length > 1;

  const favoriteModel = useMemo(
    () =>
      data.modelUsage && data.modelUsage.length > 0
        ? data.modelUsage.reduce(
            (max, cur) => (cur.cost > max.cost ? cur : max),
            data.modelUsage[0]
          ).model
        : undefined,
    [data]
  );

  const activeGraphData: TokenContributionData | null = useMemo(() => {
    if (selectedDevice === "total") {
      return buildGraphData(
        data.contributions,
        {
          totalTokens: data.stats.totalTokens,
          totalCost: data.stats.totalCost,
          activeDays: data.stats.activeDays,
          clients: data.clients,
          models: data.models,
        },
        data.dateRange
      );
    }
    const device = deviceContributions.find((d) => d.id === selectedDevice);
    if (!device) return null;
    const derived = deriveStatsFromContributions(device.contributions);
    const clients = Array.from(
      new Set(
        device.contributions.flatMap((c) => c.clients.map((cl) => cl.client))
      )
    );
    const models = Array.from(
      new Set(
        device.contributions.flatMap((c) =>
          c.clients.flatMap((cl) => Object.keys(cl.models ?? {}))
        )
      )
    );
    return buildGraphData(
      device.contributions,
      {
        totalTokens: derived.totalTokens,
        totalCost: derived.totalCost,
        activeDays: derived.activeDays,
        clients,
        models,
      },
      { start: null, end: null }
    );
  }, [selectedDevice, data, deviceContributions]);

  const user: ProfileUser = useMemo(() => ({
    username: data.user.username,
    displayName: data.user.displayName,
    avatarUrl: data.user.avatarUrl,
    rank: data.user.rank,
  }), [data]);

  const stats: ProfileStatsData = useMemo(() => ({
    totalTokens: data.stats.totalTokens,
    totalCost: data.stats.totalCost,
    inputTokens: data.stats.inputTokens,
    outputTokens: data.stats.outputTokens,
    cacheReadTokens: data.stats.cacheReadTokens,
    cacheWriteTokens: data.stats.cacheWriteTokens,
    activeDays: data.stats.activeDays,
    submissionCount: data.stats.submissionCount,
  }), [data]);

  const activeStats: ProfileStatsData = useMemo(() => {
    if (selectedDevice === "total") return stats;
    const device = deviceContributions.find((d) => d.id === selectedDevice);
    const derived = device
      ? deriveStatsFromContributions(device.contributions)
      : {
          totalTokens: 0,
          totalCost: 0,
          inputTokens: 0,
          outputTokens: 0,
          cacheReadTokens: 0,
          cacheWriteTokens: 0,
          activeDays: 0,
        };
    return { ...derived, submissionCount: data.stats.submissionCount };
  }, [selectedDevice, deviceContributions, stats, data]);

  const activeModelData = useMemo<{ models: string[]; modelUsage: ModelUsage[] }>(() => {
    if (selectedDevice === "total") {
      return { models: data.models, modelUsage: data.modelUsage ?? [] };
    }
    const device = deviceContributions.find((d) => d.id === selectedDevice);
    if (!device) return { models: [], modelUsage: [] };
    return deriveModelUsage(device.contributions);
  }, [selectedDevice, data, deviceContributions]);

const EARLY_ADOPTERS = ["code-yeongyu", "gtg7784", "qodot"];
  const showResubmitBanner = EARLY_ADOPTERS.includes(data.user.username) && data.stats.submissionCount === 1;

  return (
    <PageContainer style={{ backgroundColor: "#10121C" }}>
      <Navigation />

      {showResubmitBanner && (
        <BannerWrapper>
          <BannerContent>
            <BannerText>
              <BannerBold>Update available:</BannerBold>{" "}
              If you&apos;re <BannerBold>@{data.user.username}</BannerBold>, please re-submit your data with{" "}
              <BannerCode>bunx tokscale submit</BannerCode>{" "}
              to see detailed model breakdowns per day.
            </BannerText>
          </BannerContent>
        </BannerWrapper>
      )}

      <MainContent>
        <ContentWrapper>
          <ProfileHeader
            user={user}
            stats={stats}
            lastUpdated={data.updatedAt || undefined}
          />

          <ProfileTabBar activeTab={activeTab} onTabChange={setActiveTab} />

          {activeTab === "activity" && (
            <div
              role="tabpanel"
              id="tabpanel-activity"
              aria-labelledby="tab-activity"
            >
              {data.contributions.length === 0 ? (
                <ProfileEmptyActivity />
              ) : (
                <ActivitySection>
                  {hasDeviceBreakdown && (
                    <DeviceFilterBar
                      devices={deviceContributions}
                      selected={selectedDevice}
                      onSelect={setSelectedDevice}
                    />
                  )}
                  {activeGraphData ? (
                    <>
                      <ProfileActivity
                        key={selectedDevice}
                        data={activeGraphData}
                      />
                      <ProfileStats
                        stats={activeStats}
                        favoriteModel={
                          selectedDevice === "total" ? favoriteModel : undefined
                        }
                      />
                    </>
                  ) : (
                    <ProfileEmptyActivity />
                  )}
                </ActivitySection>
              )}
            </div>
          )}
          {activeTab === "breakdown" && (
            <div
              role="tabpanel"
              id="tabpanel-breakdown"
              aria-labelledby="tab-breakdown"
            >
              <BreakdownSection>
                <TokenBreakdown stats={stats} />
                {data.devices && data.devices.length > 0 && (
                  <ProfileDevices devices={data.devices} />
                )}
              </BreakdownSection>
            </div>
          )}
          {activeTab === "models" && (
            <div
              role="tabpanel"
              id="tabpanel-models"
              aria-labelledby="tab-models"
            >
              <ModelsSection>
                {hasDeviceBreakdown && (
                  <DeviceFilterBar
                    devices={deviceContributions}
                    selected={selectedDevice}
                    onSelect={setSelectedDevice}
                  />
                )}
                <ProfileModels
                  models={activeModelData.models}
                  modelUsage={activeModelData.modelUsage}
                />
              </ModelsSection>
            </div>
          )}
        </ContentWrapper>
      </MainContent>

      <Footer />
    </PageContainer>
  );
}

const PageContainer = styled.div`
  min-height: 100vh;
  display: flex;
  flex-direction: column;

  padding-top: 64px;
`;

const BannerWrapper = styled.div`
  background-color: rgba(245, 158, 11, 0.1);
  border-bottom: 1px solid rgba(245, 158, 11, 0.2);
`;

const BannerContent = styled.div`
  max-width: 800px;
  margin-left: auto;
  margin-right: auto;
  padding-left: 16px;
  padding-right: 16px;
  padding-top: 12px;
  padding-bottom: 12px;

  @media (min-width: 640px) {
    padding-left: 24px;
    padding-right: 24px;
  }
`;

const BannerText = styled.p`
  font-size: 14px;
  color: #fde68a;
`;

const BannerBold = styled.span`
  font-weight: 600;
`;

const BannerCode = styled.code`
  padding-left: 6px;
  padding-right: 6px;
  padding-top: 2px;
  padding-bottom: 2px;
  border-radius: 4px;
  background-color: rgba(245, 158, 11, 0.2);
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  font-size: 12px;
`;

const MainContent = styled.main`
  flex: 1;
  max-width: 800px;
  margin-left: auto;
  margin-right: auto;
  padding-left: 16px;
  padding-right: 16px;
  padding-top: 24px;
  padding-bottom: 24px;
  width: 100%;

  @media (min-width: 640px) {
    padding-left: 24px;
    padding-right: 24px;
    padding-top: 40px;
    padding-bottom: 40px;
  }
`;

const ContentWrapper = styled.div`
  display: flex;
  flex-direction: column;
  gap: 32px;
`;

const ActivitySection = styled.div`
  display: flex;
  flex-direction: column;
  gap: 24px;
`;

const DeviceSelector = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
`;

const DeviceSelectorButton = styled.button`
  padding: 6px 14px;
  border-radius: 9999px;
  border: 1px solid var(--color-border-default);
  background-color: transparent;
  color: var(--color-fg-muted);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.15s ease, color 0.15s ease, border-color 0.15s ease;

  &[data-active="true"] {
    background-color: #006edb;
    border-color: #006edb;
    color: #ffffff;
  }

  &:hover[data-active="false"] {
    color: var(--color-fg-default);
    border-color: var(--color-fg-muted);
  }
`;

const BreakdownSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: 24px;
`;

const ModelsSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: 24px;
`;
