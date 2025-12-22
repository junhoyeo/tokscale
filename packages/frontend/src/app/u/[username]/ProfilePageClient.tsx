"use client";

import { useState, useEffect, useMemo } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { ProfileSkeleton } from "@/components/Skeleton";
import {
  ProfileHeader,
  ProfileTabBar,
  TokenBreakdown,
  ProfileModels,
  ProfileActivity,
  ProfileEmptyActivity,
  ProfileStats,
  type ProfileUser,
  type ProfileStatsData,
  type ProfileTab,
  type ModelUsage,
} from "@/components/profile";
import type { TokenContributionData, DailyContribution, SourceType } from "@/lib/types";
import { calculateCurrentStreak, calculateLongestStreak } from "@/lib/utils";

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
  sources: string[];
  models: string[];
  modelUsage?: ModelUsage[];
  contributions: DailyContribution[];
}

export default function ProfilePageClient() {
  const params = useParams();
  const username = params.username as string;
  const [data, setData] = useState<ProfileData | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<ProfileTab>("activity");

  useEffect(() => {
    fetch(`/api/users/${username}`)
      .then((res) => {
        if (!res.ok) throw new Error("User not found");
        return res.json();
      })
      .then((result) => {
        setData(result);
        setIsLoading(false);
      })
      .catch((err) => {
        setError(err.message);
        setIsLoading(false);
      });
  }, [username]);

  const graphData: TokenContributionData | null = useMemo(() => {
    if (!data || data.contributions.length === 0) return null;

    const contributions = data.contributions;
    const totalCost = data.stats.totalCost;
    const totalTokens = data.stats.totalTokens;
    const maxCost = Math.max(...contributions.map((c) => c.totals.cost), 0);

    const yearMap = new Map<string, { totalTokens: number; totalCost: number; start: string; end: string }>();
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
      .map(([year, stats]) => ({
        year,
        totalTokens: stats.totalTokens,
        totalCost: stats.totalCost,
        range: { start: stats.start, end: stats.end },
      }));

    return {
      meta: {
        generatedAt: new Date().toISOString(),
        version: "1.0.0",
        dateRange: {
          start: data.dateRange.start || contributions[0]?.date || "",
          end: data.dateRange.end || contributions[contributions.length - 1]?.date || "",
        },
      },
      summary: {
        totalTokens,
        totalCost,
        totalDays: contributions.length,
        activeDays: data.stats.activeDays,
        averagePerDay: data.stats.activeDays > 0 ? totalCost / data.stats.activeDays : 0,
        maxCostInSingleDay: maxCost,
        sources: data.sources as SourceType[],
        models: data.models,
      },
      years,
      contributions: contributions as DailyContribution[],
    };
  }, [data]);

  const user: ProfileUser | null = useMemo(() => {
    if (!data) return null;
    return {
      username: data.user.username,
      displayName: data.user.displayName,
      avatarUrl: data.user.avatarUrl,
      rank: data.user.rank,
    };
  }, [data]);

  const stats: ProfileStatsData | null = useMemo(() => {
    if (!data) return null;
    return {
      totalTokens: data.stats.totalTokens,
      totalCost: data.stats.totalCost,
      inputTokens: data.stats.inputTokens,
      outputTokens: data.stats.outputTokens,
      cacheReadTokens: data.stats.cacheReadTokens,
      cacheWriteTokens: data.stats.cacheWriteTokens,
      activeDays: data.stats.activeDays,
      submissionCount: data.stats.submissionCount,
    };
  }, [data]);

  if (isLoading) {
    return (
      <div className="min-h-screen flex flex-col" style={{ backgroundColor: "#10121C" }}>
        <Navigation />
        <main className="flex-1 max-w-[800px] mx-auto px-4 sm:px-6 py-6 sm:py-10 w-full">
          <ProfileSkeleton />
        </main>
        <Footer />
      </div>
    );
  }

  if (error || !data || !user || !stats) {
    return (
      <div className="min-h-screen flex flex-col" style={{ backgroundColor: "#10121C" }}>
        <Navigation />
        <main className="flex-1 max-w-[800px] mx-auto px-6 py-10 w-full">
          <div className="text-center py-20">
            <h1 className="text-2xl font-bold text-white mb-2">
              User Not Found
            </h1>
            <p className="mb-6" style={{ color: "#696969" }}>
              The user @{username} doesn&apos;t exist or hasn&apos;t submitted any data yet.
            </p>
            <Link
              href="/"
              className="inline-flex items-center px-4 py-2 text-white font-medium rounded-lg transition-colors hover:opacity-90"
              style={{ backgroundColor: "#53d1f3" }}
            >
              Back to Leaderboard
            </Link>
          </div>
        </main>
        <Footer />
      </div>
    );
  }

  const EARLY_ADOPTERS = ["code-yeongyu", "gtg7784", "qodot"];
  const showResubmitBanner = EARLY_ADOPTERS.includes(data.user.username) && data.stats.submissionCount === 1;

  return (
    <div className="min-h-screen flex flex-col" style={{ backgroundColor: "#10121C" }}>
      <Navigation />

      {showResubmitBanner && (
        <div className="bg-amber-500/10 border-b border-amber-500/20">
          <div className="max-w-[800px] mx-auto px-4 sm:px-6 py-3">
            <p className="text-sm text-amber-200">
              <span className="font-semibold">Update available:</span>{" "}
              If you&apos;re <span className="font-semibold">@{data.user.username}</span>, please re-submit your data with{" "}
              <code className="px-1.5 py-0.5 rounded bg-amber-500/20 font-mono text-xs">bunx tokscale submit</code>{" "}
              to see detailed model breakdowns per day.
            </p>
          </div>
        </div>
      )}

      <main className="flex-1 max-w-[800px] mx-auto px-4 sm:px-6 py-6 sm:py-10 w-full">
        <div className="flex flex-col gap-8">
          <ProfileHeader
            user={user}
            stats={stats}
            lastUpdated={data.dateRange.end || undefined}
          />

          <ProfileTabBar activeTab={activeTab} onTabChange={setActiveTab} />

          {activeTab === "activity" && (
            graphData ? (
              <div className="flex flex-col gap-6">
                <ProfileActivity data={graphData} />
                <ProfileStats
                  stats={stats}
                  currentStreak={calculateCurrentStreak(graphData.contributions)}
                  longestStreak={calculateLongestStreak(graphData.contributions)}
                  favoriteModel={data.models.filter(m => m !== "<synthetic>")[0]}
                />
              </div>
            ) : <ProfileEmptyActivity />
          )}
          {activeTab === "breakdown" && <TokenBreakdown stats={stats} />}
          {activeTab === "models" && <ProfileModels models={data.models} modelUsage={data.modelUsage} />}
        </div>
      </main>

      <Footer />
    </div>
  );
}
