"use client";

import { useState, useEffect, useMemo } from "react";
import Link from "next/link";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { GraphContainer } from "@/components/GraphContainer";
import { ProfileSkeleton } from "@/components/Skeleton";
import type { TokenContributionData } from "@/lib/types";

// Mockup user data for @junhoyeo
const MOCK_USER = {
  id: "mockup-junhoyeo",
  username: "junhoyeo",
  displayName: "Junho Yeo",
  avatarUrl: "https://avatars.githubusercontent.com/u/32605822?v=4",
  createdAt: "2024-01-01T00:00:00Z",
  rank: 1,
};

function formatNumber(num: number): string {
  if (num >= 1_000_000_000) return `${(num / 1_000_000_000).toFixed(2)}B`;
  if (num >= 1_000_000) return `${(num / 1_000_000).toFixed(1)}M`;
  if (num >= 1_000) return `${(num / 1_000).toFixed(1)}K`;
  return num.toLocaleString();
}

function formatCurrency(amount: number): string {
  if (amount >= 1000) return `$${(amount / 1000).toFixed(2)}K`;
  return `$${amount.toFixed(2)}`;
}

export default function JunhoyeoMockupPage() {
  const [data, setData] = useState<TokenContributionData | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    fetch("/junhoyeo-data.json")
      .then((res) => res.json())
      .then((result) => {
        setData(result);
        setIsLoading(false);
      })
      .catch(() => {
        setIsLoading(false);
      });
  }, []);

  // Calculate stats from data
  const stats = useMemo(() => {
    if (!data) return null;

    let inputTokens = 0;
    let outputTokens = 0;
    let cacheReadTokens = 0;
    let cacheWriteTokens = 0;

    for (const day of data.contributions) {
      inputTokens += day.tokenBreakdown.input;
      outputTokens += day.tokenBreakdown.output;
      cacheReadTokens += day.tokenBreakdown.cacheRead;
      cacheWriteTokens += day.tokenBreakdown.cacheWrite;
    }

    return {
      totalTokens: data.summary.totalTokens,
      totalCost: data.summary.totalCost,
      inputTokens,
      outputTokens,
      cacheReadTokens,
      cacheWriteTokens,
      activeDays: data.summary.activeDays,
      submissionCount: 1,
    };
  }, [data]);

  if (isLoading) {
    return (
      <div className="min-h-screen flex flex-col bg-gray-50 dark:bg-gray-950">
        <Navigation />
        <main className="flex-1 max-w-7xl mx-auto px-4 sm:px-6 py-6 sm:py-10 w-full">
          <ProfileSkeleton />
        </main>
        <Footer />
      </div>
    );
  }

  if (!data || !stats) {
    return (
      <div className="min-h-screen flex flex-col bg-gray-50 dark:bg-gray-950">
        <Navigation />
        <main className="flex-1 max-w-7xl mx-auto px-6 py-10 w-full">
          <div className="text-center py-20">
            <h1 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">
              Failed to load data
            </h1>
            <p className="text-gray-500 dark:text-gray-400 mb-6">
              Make sure junhoyeo-data.json exists in the public folder.
            </p>
            <Link
              href="/"
              className="inline-flex items-center px-4 py-2 bg-green-600 text-white font-medium rounded-lg hover:bg-green-700 transition-colors"
            >
              Back to Home
            </Link>
          </div>
        </main>
        <Footer />
      </div>
    );
  }

  return (
    <div className="min-h-screen flex flex-col bg-gray-50 dark:bg-gray-950 transition-colors duration-300">
      <Navigation />

      <main className="flex-1 max-w-7xl mx-auto px-4 sm:px-6 py-6 sm:py-10 w-full">
        {/* Mockup Badge */}
        <div className="mb-6 p-3 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-xl">
          <p className="text-sm text-amber-800 dark:text-amber-200 flex items-center gap-2">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <span>
              <strong>Mockup Preview</strong> — This is a preview using local token usage data from your machine.
            </span>
          </p>
        </div>

        {/* User Header */}
        <div className="mb-6 sm:mb-8">
          <div className="flex items-start gap-4 sm:gap-6 mb-6">
            <img
              src={MOCK_USER.avatarUrl}
              alt={MOCK_USER.username}
              className="w-20 h-20 sm:w-28 sm:h-28 rounded-2xl sm:rounded-3xl ring-4 ring-gray-200 dark:ring-gray-700 shadow-xl"
            />
            <div className="flex-1 min-w-0">
              <div className="flex flex-wrap items-center gap-2 sm:gap-3 mb-1">
                <h1 className="text-2xl sm:text-3xl font-bold text-gray-900 dark:text-white">
                  {MOCK_USER.displayName}
                </h1>
                <span className="px-3 py-1 text-sm font-bold bg-gradient-to-r from-yellow-400 to-amber-500 text-white rounded-full shadow-lg shadow-yellow-500/25 flex items-center gap-1">
                  <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
                  </svg>
                  #1
                </span>
              </div>
              <p className="text-base sm:text-lg text-gray-500 dark:text-gray-400 mb-3">
                @{MOCK_USER.username}
              </p>
              <div className="flex flex-wrap gap-2">
                {data.summary.sources.map((source) => (
                  <span
                    key={source}
                    className="px-3 py-1 text-xs font-semibold bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300 rounded-full capitalize"
                  >
                    {source}
                  </span>
                ))}
              </div>
            </div>
          </div>
        </div>

        {/* Hero Stats */}
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-3 sm:gap-4 mb-6 sm:mb-8">
          <div className="bg-gradient-to-br from-green-500 to-emerald-600 rounded-2xl p-4 sm:p-6 text-white shadow-xl shadow-green-500/20">
            <p className="text-sm sm:text-base text-green-100 mb-1">Total Cost</p>
            <p className="text-3xl sm:text-4xl font-bold">{formatCurrency(stats.totalCost)}</p>
            <p className="text-xs sm:text-sm text-green-200 mt-1">
              {data.meta.dateRange.start} - {data.meta.dateRange.end}
            </p>
          </div>
          <div className="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800 p-4 sm:p-6 shadow-sm">
            <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400">Total Tokens</p>
            <p className="text-2xl sm:text-3xl font-bold text-gray-900 dark:text-white">
              {formatNumber(stats.totalTokens)}
            </p>
            <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
              ~{(stats.totalTokens / 750).toFixed(0).replace(/\B(?=(\d{3})+(?!\d))/g, ",")} words
            </p>
          </div>
          <div className="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800 p-4 sm:p-6 shadow-sm">
            <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400">Active Days</p>
            <p className="text-2xl sm:text-3xl font-bold text-gray-900 dark:text-white">
              {stats.activeDays}
            </p>
            <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
              {(stats.totalCost / stats.activeDays).toFixed(2)}/day avg
            </p>
          </div>
          <div className="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800 p-4 sm:p-6 shadow-sm">
            <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400">Models Used</p>
            <p className="text-2xl sm:text-3xl font-bold text-gray-900 dark:text-white">
              {data.summary.models.length}
            </p>
            <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
              across {data.summary.sources.length} platforms
            </p>
          </div>
        </div>

        {/* Token Breakdown with Visual Bar */}
        <div className="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800 p-4 sm:p-6 mb-6 sm:mb-8 shadow-sm">
          <h2 className="text-lg sm:text-xl font-bold text-gray-900 dark:text-white mb-4">
            Token Breakdown
          </h2>
          
          {/* Visual breakdown bar */}
          <div className="mb-6">
            <div className="h-4 rounded-full overflow-hidden flex bg-gray-100 dark:bg-gray-800">
              <div 
                className="bg-blue-500" 
                style={{ width: `${(stats.inputTokens / stats.totalTokens) * 100}%` }}
                title={`Input: ${formatNumber(stats.inputTokens)}`}
              />
              <div 
                className="bg-purple-500" 
                style={{ width: `${(stats.outputTokens / stats.totalTokens) * 100}%` }}
                title={`Output: ${formatNumber(stats.outputTokens)}`}
              />
              <div 
                className="bg-green-500" 
                style={{ width: `${(stats.cacheReadTokens / stats.totalTokens) * 100}%` }}
                title={`Cache Read: ${formatNumber(stats.cacheReadTokens)}`}
              />
              <div 
                className="bg-amber-500" 
                style={{ width: `${(stats.cacheWriteTokens / stats.totalTokens) * 100}%` }}
                title={`Cache Write: ${formatNumber(stats.cacheWriteTokens)}`}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
            <div className="flex items-center gap-3">
              <div className="w-3 h-3 rounded-full bg-blue-500" />
              <div>
                <p className="text-xs text-gray-500 dark:text-gray-400">Input</p>
                <p className="text-base sm:text-lg font-semibold text-gray-900 dark:text-white">
                  {formatNumber(stats.inputTokens)}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-3 h-3 rounded-full bg-purple-500" />
              <div>
                <p className="text-xs text-gray-500 dark:text-gray-400">Output</p>
                <p className="text-base sm:text-lg font-semibold text-gray-900 dark:text-white">
                  {formatNumber(stats.outputTokens)}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-3 h-3 rounded-full bg-green-500" />
              <div>
                <p className="text-xs text-gray-500 dark:text-gray-400">Cache Read</p>
                <p className="text-base sm:text-lg font-semibold text-gray-900 dark:text-white">
                  {formatNumber(stats.cacheReadTokens)}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-3 h-3 rounded-full bg-amber-500" />
              <div>
                <p className="text-xs text-gray-500 dark:text-gray-400">Cache Write</p>
                <p className="text-base sm:text-lg font-semibold text-gray-900 dark:text-white">
                  {formatNumber(stats.cacheWriteTokens)}
                </p>
              </div>
            </div>
          </div>
        </div>

        {/* Models Used */}
        <div className="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800 p-4 sm:p-6 mb-6 sm:mb-8 shadow-sm">
          <h2 className="text-lg sm:text-xl font-bold text-gray-900 dark:text-white mb-4">
            Models Used
          </h2>
          <div className="flex flex-wrap gap-2">
            {data.summary.models.filter(m => m !== '<synthetic>').map((model) => (
              <span
                key={model}
                className="px-3 py-1.5 text-sm font-medium bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300 rounded-lg"
              >
                {model}
              </span>
            ))}
          </div>
        </div>

        {/* Contribution Graph */}
        <div className="mb-6 sm:mb-8">
          <h2 className="text-lg sm:text-xl font-bold text-gray-900 dark:text-white mb-4">
            Activity
          </h2>
          <div className="overflow-x-auto -mx-4 sm:mx-0 px-4 sm:px-0">
            <div className="min-w-[600px] sm:min-w-0">
              <GraphContainer data={data} />
            </div>
          </div>
        </div>

        {/* Call to Action */}
        <div className="bg-gradient-to-r from-green-500 to-emerald-600 rounded-2xl p-6 sm:p-8 text-white shadow-xl">
          <h2 className="text-xl sm:text-2xl font-bold mb-2">Want your own profile?</h2>
          <p className="text-green-100 mb-4">
            Share your AI token usage and compete on the leaderboard!
          </p>
          <div className="flex flex-wrap gap-3">
            <code className="px-4 py-2 bg-black/20 rounded-lg text-sm font-mono">
              token-tracker login
            </code>
            <code className="px-4 py-2 bg-black/20 rounded-lg text-sm font-mono">
              token-tracker submit
            </code>
          </div>
        </div>
      </main>

      <Footer />
    </div>
  );
}
