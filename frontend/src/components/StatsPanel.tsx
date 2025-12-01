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
      className="rounded-lg border p-4"
      style={{
        backgroundColor: "var(--color-card-bg)",
        borderColor: "var(--color-border-default)",
      }}
    >
      <h3
        className="text-sm font-semibold mb-3 uppercase tracking-wide"
        style={{ color: "var(--color-fg-muted)" }}
      >
        Statistics
      </h3>

      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-4">
        {/* Total Cost */}
        <StatItem
          label="Total Cost"
          value={formatCurrency(summary.totalCost)}
          highlightColor={palette.grade4}
          highlight
        />

        {/* Total Tokens */}
        <StatItem
          label="Total Tokens"
          value={formatTokenCount(summary.totalTokens)}
        />

        {/* Active Days */}
        <StatItem
          label="Active Days"
          value={`${summary.activeDays} / ${summary.totalDays}`}
        />

        {/* Avg Per Day */}
        <StatItem
          label="Avg / Day"
          value={formatCurrency(summary.averagePerDay)}
        />

        {/* Current Streak */}
        <StatItem
          label="Current Streak"
          value={`${currentStreak} day${currentStreak !== 1 ? "s" : ""}`}
        />

        {/* Longest Streak */}
        <StatItem
          label="Longest Streak"
          value={`${longestStreak} day${longestStreak !== 1 ? "s" : ""}`}
        />

        {/* Best Day */}
        {bestDay && bestDay.totals.cost > 0 && (
          <StatItem
            label="Best Day"
            value={formatDate(bestDay.date)}
            subValue={formatCurrency(bestDay.totals.cost)}
          />
        )}

        {/* Models Used */}
        <StatItem
          label="Models"
          value={summary.models.length.toString()}
        />
      </div>

      {/* Sources */}
      <div
        className="mt-4 pt-4 border-t flex flex-wrap gap-2"
        style={{ borderColor: "var(--color-border-default)" }}
      >
        <span className="text-xs uppercase tracking-wide mr-2" style={{ color: "var(--color-fg-muted)" }}>
          Sources:
        </span>
        {summary.sources.map((source) => (
          <span
            key={source}
            className="text-xs px-2 py-0.5 rounded"
            style={{
              backgroundColor: `${palette.grade3}20`,
              color: "var(--color-fg-default)",
            }}
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
    <div>
      <div className="text-xs uppercase tracking-wide mb-1" style={{ color: "var(--color-fg-muted)" }}>
        {label}
      </div>
      <div
        className={`font-semibold ${highlight ? "text-lg" : "text-base"}`}
        style={{ color: highlight && highlightColor ? highlightColor : "var(--color-fg-default)" }}
      >
        {value}
      </div>
      {subValue && (
        <div className="text-xs" style={{ color: "var(--color-fg-muted)" }}>
          {subValue}
        </div>
      )}
    </div>
  );
}
