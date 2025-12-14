"use client";

import type { TokenContributionData, GraphColorPalette } from "@/lib/types";
import {
  formatCurrency,
  formatTokenCount,
  formatDate,
  calculateCurrentStreak,
  calculateLongestStreak,
  findBestDay,
} from "@/lib/utils";

interface StatsPanelProps {
  data: TokenContributionData;
  palette: GraphColorPalette;
}

export function StatsPanel({ data, palette }: StatsPanelProps) {
  const { summary, contributions } = data;
  const currentStreak = calculateCurrentStreak(contributions);
  const longestStreak = calculateLongestStreak(contributions);
  const bestDay = findBestDay(contributions);

  return (
    <div
      className="rounded-2xl border p-6 shadow-sm transition-shadow hover:shadow-md"
      style={{
        backgroundColor: "#1F1F20",
        borderColor: "#262627",
      }}
    >
      <h3 className="text-sm font-bold mb-4 uppercase tracking-wider" style={{ color: "#696969" }}>
        Statistics
      </h3>

      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-6">
        <StatItem label="Total Cost" value={formatCurrency(summary.totalCost)} highlightColor={palette.grade4} highlight />
        <StatItem label="Total Tokens" value={formatTokenCount(summary.totalTokens)} />
        <StatItem label="Active Days" value={`${summary.activeDays} / ${summary.totalDays}`} />
        <StatItem label="Avg / Day" value={formatCurrency(summary.averagePerDay)} />
        <StatItem label="Current Streak" value={`${currentStreak} day${currentStreak !== 1 ? "s" : ""}`} />
        <StatItem label="Longest Streak" value={`${longestStreak} day${longestStreak !== 1 ? "s" : ""}`} />
        {bestDay && bestDay.totals.cost > 0 && (
          <StatItem label="Best Day" value={formatDate(bestDay.date)} subValue={formatCurrency(bestDay.totals.cost)} />
        )}
        <StatItem label="Models" value={summary.models.length.toString()} />
      </div>

      <div className="mt-6 pt-6 border-t flex flex-wrap gap-2 items-center" style={{ borderColor: "#262627" }}>
        <span className="text-xs font-semibold uppercase tracking-wider mr-3" style={{ color: "#696969" }}>
          Sources:
        </span>
        {summary.sources.map((source) => (
          <span
            key={source}
            className="text-xs px-3 py-1.5 rounded-full font-medium transition-all duration-200 hover:scale-105"
            style={{ backgroundColor: `${palette.grade3}20`, color: "#FFFFFF" }}
          >
            {source}
          </span>
        ))}
      </div>
    </div>
  );
}

interface StatItemProps {
  label: string;
  value: string;
  subValue?: string;
  highlightColor?: string;
  highlight?: boolean;
}

function StatItem({ label, value, subValue, highlightColor, highlight }: StatItemProps) {
  return (
    <div className="space-y-1">
      <div className="text-xs font-semibold uppercase tracking-wider" style={{ color: "#696969" }}>
        {label}
      </div>
      <div
        className={`font-bold tracking-tight ${highlight ? "text-xl" : "text-lg"}`}
        style={{ color: highlight && highlightColor ? highlightColor : "#FFFFFF" }}
      >
        {value}
      </div>
      {subValue && (
        <div className="text-xs font-medium" style={{ color: "#696969" }}>
          {subValue}
        </div>
      )}
    </div>
  );
}
