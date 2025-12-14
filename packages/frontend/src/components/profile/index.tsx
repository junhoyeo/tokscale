"use client";

import Image from "next/image";
import { GraphContainer } from "@/components/GraphContainer";
import type { TokenContributionData } from "@/lib/types";
import { formatNumber, formatCurrency } from "@/lib/utils";

export interface ProfileUser {
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  rank: number | null;
}

export interface ProfileStatsData {
  totalTokens: number;
  totalCost: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  activeDays: number;
  submissionCount?: number;
}

export interface ProfileHeaderProps {
  user: ProfileUser;
  stats: ProfileStatsData;
  lastUpdated?: string;
}

export function ProfileHeader({ user, stats, lastUpdated }: ProfileHeaderProps) {
  const avatarUrl = user.avatarUrl || `https://github.com/${user.username}.png`;

  return (
    <div
      className="flex flex-col gap-2 rounded-2xl border p-4 pb-[18px]"
      style={{ backgroundColor: "#141415", borderColor: "#262627" }}
    >
      <div className="flex flex-col lg:flex-row gap-6 lg:gap-10">
        <div
          className="flex flex-row items-center gap-[19px] rounded-[20px] py-3 pl-3 pr-8 flex-1"
          style={{ backgroundColor: "#111113" }}
        >
          <div
            className="relative w-[100px] h-[100px] rounded-[7px] overflow-hidden border-2 flex-shrink-0"
            style={{ borderColor: "#262627" }}
          >
            <Image
              src={avatarUrl}
              alt={user.username}
              fill
              className="object-cover"
            />
          </div>

          <div className="flex flex-col flex-1 min-w-0 justify-end gap-[6px] py-0 pb-1 h-[100px]">
            {user.rank && (
              <div
                className="w-8 h-8 rounded-lg flex items-center justify-center"
                style={{
                  background: "linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%)",
                  border: "1px solid #262627",
                }}
              >
                <span
                  className="text-base font-medium"
                  style={{ color: "#85CAFF" }}
                >
                  #{user.rank}
                </span>
              </div>
            )}

            <div className="flex flex-col gap-[6px] flex-1 justify-end min-w-0">
              <h1
                className="text-2xl font-bold truncate leading-[1.2]"
                style={{ color: "#FFFFFF" }}
              >
                {user.displayName || user.username}
              </h1>
              <p
                className="text-sm font-bold leading-none"
                style={{ color: "#696969" }}
              >
                @{user.username}
              </p>
            </div>
          </div>
        </div>

        <div className="flex flex-row items-center gap-7 h-[124px] flex-1">
          <div className="flex flex-col gap-[15px] flex-1 min-w-[120px]">
            <span
              className="text-base font-semibold leading-none"
              style={{ color: "#85CAFF" }}
            >
              Total Usage Cost
            </span>
            <span
              className="text-[27px] font-bold leading-none"
              style={{
                background: "linear-gradient(117deg, #169AFF 0%, #9FD4FB 26%, #B9DFF8 52%)",
                WebkitBackgroundClip: "text",
                WebkitTextFillColor: "transparent",
                backgroundClip: "text",
              }}
            >
              {formatCurrency(stats.totalCost)}
            </span>
          </div>

          <div className="flex flex-col gap-[15px] flex-1 min-w-[120px]">
            <span
              className="text-base font-semibold leading-none"
              style={{ color: "#FFFFFF" }}
            >
              Total Tokens
            </span>
            <span
              className="text-[27px] font-bold leading-none"
              style={{ color: "#FFFFFF" }}
            >
              {formatNumber(stats.totalTokens)}
            </span>
          </div>
        </div>
      </div>

      <div className="w-full h-px" style={{ backgroundColor: "#262627" }} />

      <div className="flex flex-col sm:flex-row items-start sm:items-end justify-between gap-3">
        {lastUpdated && (
          <span
            className="text-sm leading-[1.21]"
            style={{ color: "#696969" }}
          >
            Last Updated: {lastUpdated}
          </span>
        )}

        <div className="flex flex-row items-center gap-[6px]">
          <button
            className="flex flex-row items-center justify-center gap-[6px] rounded-full border py-[9px] pl-[10px] pr-[11px] transition-opacity hover:opacity-80"
            style={{ backgroundColor: "#212124", borderColor: "#262627" }}
          >
            <Image src="/icons/icon-share.svg" alt="Share" width={20} height={20} />
            <span
              className="text-sm leading-none"
              style={{ color: "#FFFFFF" }}
            >
              Share
            </span>
          </button>

          <a
            href={`https://github.com/${user.username}`}
            target="_blank"
            rel="noopener noreferrer"
            className="flex flex-row items-center justify-center gap-[6px] rounded-full border py-[9px] pl-[10px] pr-[11px] transition-opacity hover:opacity-80"
            style={{ backgroundColor: "#212124", borderColor: "#262627" }}
          >
            <Image src="/icons/icon-github.svg" alt="GitHub" width={20} height={20} />
            <span
              className="text-sm leading-none"
              style={{ color: "#FFFFFF" }}
            >
              GitHub
            </span>
          </a>
        </div>
      </div>
    </div>
  );
}

export type ProfileTab = "activity" | "breakdown" | "models";

export interface ProfileTabBarProps {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
}

export function ProfileTabBar({ activeTab, onTabChange }: ProfileTabBarProps) {
  const tabs: { id: ProfileTab; label: string }[] = [
    { id: "activity", label: "Activity" },
    { id: "breakdown", label: "Token Breakdown" },
    { id: "models", label: "Models Used" },
  ];

  return (
    <div
      className="inline-flex flex-row items-center rounded-[25px] border p-[6px]"
      style={{
        width: "fit-content",
        backgroundColor: "#1F1F20",
        borderColor: "#262627",
      }}
    >
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => onTabChange(tab.id)}
          className="flex items-center justify-center rounded-[25px] px-5 py-[10px] transition-colors"
          style={{
            backgroundColor: activeTab === tab.id ? "#2C2C2F" : "transparent",
          }}
        >
          <span
            className="text-lg font-semibold leading-none whitespace-nowrap"
            style={{
              color: activeTab === tab.id ? "#FFFFFF" : "rgba(255, 255, 255, 0.5)",
            }}
          >
            {tab.label}
          </span>
        </button>
      ))}
    </div>
  );
}

export interface TokenBreakdownProps {
  stats: ProfileStatsData;
}

export function TokenBreakdown({ stats }: TokenBreakdownProps) {
  const { totalTokens, inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens } = stats;

  const tokenTypes = [
    { label: "Input", value: inputTokens, color: "#006edb", percentage: totalTokens > 0 ? (inputTokens / totalTokens) * 100 : 0 },
    { label: "Output", value: outputTokens, color: "#894ceb", percentage: totalTokens > 0 ? (outputTokens / totalTokens) * 100 : 0 },
    { label: "Cache Read", value: cacheReadTokens, color: "#30a147", percentage: totalTokens > 0 ? (cacheReadTokens / totalTokens) * 100 : 0 },
    { label: "Cache Write", value: cacheWriteTokens, color: "#eb670f", percentage: totalTokens > 0 ? (cacheWriteTokens / totalTokens) * 100 : 0 },
  ];

  return (
    <div
      className="rounded-2xl border p-4 sm:p-6"
      style={{ backgroundColor: "#141415", borderColor: "#262627" }}
    >
      {totalTokens > 0 && (
        <div className="mb-6">
          <div
            className="h-3 rounded-full overflow-hidden flex"
            style={{ backgroundColor: "#262627" }}
          >
            {tokenTypes.map((type) => (
              <div
                key={type.label}
                style={{
                  width: `${type.percentage}%`,
                  backgroundColor: type.color,
                }}
                title={`${type.label}: ${formatNumber(type.value)}`}
              />
            ))}
          </div>
        </div>
      )}

      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        {tokenTypes.map((type) => (
          <div key={type.label} className="flex items-center gap-3">
            <div className="w-3 h-3 rounded-full flex-shrink-0" style={{ backgroundColor: type.color }} />
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <p className="text-xs" style={{ color: "#696969" }}>{type.label}</p>
                {type.percentage > 0 && (
                  <span className="text-xs" style={{ color: "#525252" }}>
                    {type.percentage.toFixed(1)}%
                  </span>
                )}
              </div>
              <p
                className="text-base sm:text-lg font-semibold"
                style={{ color: "#FFFFFF" }}
              >
                {formatNumber(type.value)}
              </p>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export interface ProfileStatsProps {
  stats: ProfileStatsData;
  currentStreak?: number;
  longestStreak?: number;
  favoriteModel?: string;
}

export function ProfileStats({ stats, currentStreak = 0, longestStreak = 0, favoriteModel }: ProfileStatsProps) {
  const statItems = [
    { label: "Active Days", value: stats.activeDays.toString(), color: "#53d1f3" },
    { label: "Current Streak", value: `${currentStreak} days`, color: "#53d1f3" },
    { label: "Longest Streak", value: `${longestStreak} days`, color: "#53d1f3" },
    { label: "Submissions", value: (stats.submissionCount ?? 0).toString(), color: "#53d1f3" },
  ];

  return (
    <div
      className="rounded-2xl border p-4 sm:p-6"
      style={{ backgroundColor: "#141415", borderColor: "#262627" }}
    >
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 sm:gap-6">
        {statItems.map((item) => (
          <div key={item.label} className="flex flex-col gap-1">
            <p className="text-xs sm:text-sm" style={{ color: "#696969" }}>{item.label}</p>
            <p
              className="text-lg sm:text-xl font-bold"
              style={{ color: item.color }}
            >
              {item.value}
            </p>
          </div>
        ))}
      </div>

      {favoriteModel && (
        <div className="mt-4 pt-4 border-t" style={{ borderColor: "#262627" }}>
          <div className="flex items-center gap-2">
            <span className="text-sm" style={{ color: "#696969" }}>Favorite Model:</span>
            <span
              className="px-2 py-1 rounded-md text-sm font-medium"
              style={{ backgroundColor: "#262627", color: "#FFFFFF" }}
            >
              {favoriteModel}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

const MODEL_COLORS: Record<string, string> = {
  "claude": "#D97706",
  "sonnet": "#D97706",
  "opus": "#DC2626",
  "haiku": "#059669",
  "gpt": "#10B981",
  "o1": "#6366F1",
  "o3": "#8B5CF6",
  "gemini": "#3B82F6",
  "deepseek": "#06B6D4",
  "codex": "#F59E0B",
};

function getModelColor(modelName: string): string {
  const lowerName = modelName.toLowerCase();
  for (const [key, color] of Object.entries(MODEL_COLORS)) {
    if (lowerName.includes(key)) return color;
  }
  return "#6B7280";
}

export interface ModelUsage {
  model: string;
  tokens: number;
  cost: number;
  percentage: number;
}

export interface ProfileModelsProps {
  models: string[];
  modelUsage?: ModelUsage[];
}

export function ProfileModels({ models, modelUsage }: ProfileModelsProps) {
  const filteredModels = models.filter((m) => m !== "<synthetic>");

  if (filteredModels.length === 0) return null;

  if (modelUsage && modelUsage.length > 0) {
    const sortedUsage = [...modelUsage].sort((a, b) => b.cost - a.cost);

    return (
      <div
        className="rounded-2xl border overflow-hidden"
        style={{ backgroundColor: "#141415", borderColor: "#262627" }}
      >
        <div
          className="grid grid-cols-[1fr_auto_auto_auto] gap-4 px-4 sm:px-6 py-3 text-xs font-medium uppercase tracking-wider border-b"
          style={{ backgroundColor: "#1F1F20", borderColor: "#262627", color: "#696969" }}
        >
          <div>Model</div>
          <div className="text-right w-20 sm:w-24">Tokens</div>
          <div className="text-right w-16 sm:w-20">Cost</div>
          <div className="text-right w-12 sm:w-16">%</div>
        </div>

        <div className="divide-y" style={{ borderColor: "#262627" }}>
          {sortedUsage.map((usage, index) => (
            <div
              key={usage.model}
              className="grid grid-cols-[1fr_auto_auto_auto] gap-4 px-4 sm:px-6 py-3 items-center"
              style={{
                backgroundColor: index % 2 === 1 ? "#1A1A1B" : "transparent",
              }}
            >
              <div className="flex items-center gap-2 min-w-0">
                <div
                  className="w-2 h-2 rounded-full flex-shrink-0"
                  style={{ backgroundColor: getModelColor(usage.model) }}
                />
                <span
                  className="text-sm font-medium truncate"
                  style={{ color: "#FFFFFF" }}
                >
                  {usage.model}
                </span>
              </div>
              <div className="text-right w-20 sm:w-24">
                <span className="text-sm" style={{ color: "#FFFFFF" }}>
                  {formatNumber(usage.tokens)}
                </span>
              </div>
              <div className="text-right w-16 sm:w-20">
                <span className="text-sm font-medium" style={{ color: "#53d1f3" }}>
                  {formatCurrency(usage.cost)}
                </span>
              </div>
              <div className="text-right w-12 sm:w-16">
                <span className="text-sm" style={{ color: "#696969" }}>
                  {usage.percentage.toFixed(1)}%
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div
      className="rounded-2xl border p-4 sm:p-6"
      style={{ backgroundColor: "#141415", borderColor: "#262627" }}
    >
      <div className="flex flex-wrap gap-2">
        {filteredModels.map((model) => (
          <span
            key={model}
            className="flex items-center gap-2 px-3 py-1.5 rounded-full text-sm font-medium"
            style={{ backgroundColor: "#262627", color: "#FFFFFF" }}
          >
            <span
              className="w-2 h-2 rounded-full"
              style={{ backgroundColor: getModelColor(model) }}
            />
            {model}
          </span>
        ))}
      </div>
    </div>
  );
}

export interface ProfileActivityProps {
  data: TokenContributionData;
}

export function ProfileActivity({ data }: ProfileActivityProps) {
  return (
    <div className="overflow-x-auto -mx-4 sm:mx-0 px-4 sm:px-0">
      <div className="min-w-[600px] sm:min-w-0">
        <GraphContainer data={data} />
      </div>
    </div>
  );
}

export function ProfileEmptyActivity() {
  return (
    <div
      className="rounded-2xl border p-6 sm:p-8 text-center"
      style={{ backgroundColor: "#141415", borderColor: "#262627" }}
    >
      <p className="text-sm sm:text-base" style={{ color: "#696969" }}>
        No contribution data available yet.
      </p>
    </div>
  );
}


