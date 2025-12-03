"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { SegmentedControl, Pagination, Avatar } from "@primer/react";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { LeaderboardSkeleton } from "@/components/Skeleton";

type Period = "all" | "month" | "week";

interface LeaderboardUser {
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

interface LeaderboardData {
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

function formatNumber(num: number): string {
  if (num >= 1_000_000_000) return `${(num / 1_000_000_000).toFixed(1)}B`;
  if (num >= 1_000_000) return `${(num / 1_000_000).toFixed(1)}M`;
  if (num >= 1_000) return `${(num / 1_000).toFixed(1)}K`;
  return num.toLocaleString();
}

function formatCurrency(amount: number): string {
  return `$${amount.toFixed(2)}`;
}

export default function LeaderboardPage() {
  const [data, setData] = useState<LeaderboardData | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [period, setPeriod] = useState<Period>("all");
  const [page, setPage] = useState(1);

  useEffect(() => {
    setIsLoading(true);
    fetch(`/api/leaderboard?period=${period}&page=${page}&limit=25`)
      .then((res) => res.json())
      .then((result) => {
        setData(result);
        setIsLoading(false);
      })
      .catch(() => {
        setIsLoading(false);
      });
  }, [period, page]);

  return (
    <div className="min-h-screen flex flex-col bg-gray-50 dark:bg-gray-950 transition-colors duration-300">
      <Navigation />

      <main className="flex-1 max-w-7xl mx-auto px-6 py-10 w-full">
        {/* Hero Stats */}
        <div className="mb-10">
          <h1 className="text-3xl font-bold text-gray-900 dark:text-white mb-2">
            Leaderboard
          </h1>
          <p className="text-gray-500 dark:text-gray-400 mb-6">
            See who's using the most AI tokens
          </p>

          <div className="grid grid-cols-2 md:grid-cols-4 gap-3 sm:gap-4">
            <div className="bg-white dark:bg-gray-900 rounded-xl border border-gray-200 dark:border-gray-800 p-3 sm:p-4">
              <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400">
                Total Tokens
              </p>
              <p className="text-xl sm:text-2xl font-bold text-gray-900 dark:text-white">
                {data ? formatNumber(data.stats.totalTokens) : "-"}
              </p>
            </div>
            <div className="bg-white dark:bg-gray-900 rounded-xl border border-gray-200 dark:border-gray-800 p-3 sm:p-4">
              <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400">
                Total Cost
              </p>
              <p className="text-xl sm:text-2xl font-bold text-green-600 dark:text-green-400">
                {data ? formatCurrency(data.stats.totalCost) : "-"}
              </p>
            </div>
            <div className="bg-white dark:bg-gray-900 rounded-xl border border-gray-200 dark:border-gray-800 p-3 sm:p-4">
              <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400">
                Users
              </p>
              <p className="text-xl sm:text-2xl font-bold text-gray-900 dark:text-white">
                {data ? data.stats.uniqueUsers : "-"}
              </p>
            </div>
            <div className="bg-white dark:bg-gray-900 rounded-xl border border-gray-200 dark:border-gray-800 p-3 sm:p-4">
              <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400">
                Submissions
              </p>
              <p className="text-xl sm:text-2xl font-bold text-gray-900 dark:text-white">
                {data ? data.stats.totalSubmissions : "-"}
              </p>
            </div>
          </div>
        </div>

        {/* Period Filter */}
        <div className="mb-6">
          <SegmentedControl 
            aria-label="Period filter"
            onChange={(index) => {
              const periods: Period[] = ["all", "month", "week"];
              setPeriod(periods[index]);
              setPage(1);
            }}
          >
            <SegmentedControl.Button selected={period === "all"}>
              All Time
            </SegmentedControl.Button>
            <SegmentedControl.Button selected={period === "month"}>
              This Month
            </SegmentedControl.Button>
            <SegmentedControl.Button selected={period === "week"}>
              This Week
            </SegmentedControl.Button>
          </SegmentedControl>
        </div>

        {/* Leaderboard Table */}
        {isLoading ? (
          <LeaderboardSkeleton />
        ) : (
        <div className="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800 overflow-hidden">
          {!data || data.users.length === 0 ? (
            <div className="p-8 text-center">
              <p className="text-gray-500 dark:text-gray-400 mb-4">
                No submissions yet. Be the first!
              </p>
              <p className="text-sm text-gray-400 dark:text-gray-500">
                Run{" "}
                <code className="px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded">
                  token-tracker login && token-tracker submit
                </code>
              </p>
            </div>
          ) : (
            <>
              <div className="overflow-x-auto">
                <table className="w-full min-w-[500px]">
                  <thead className="bg-gray-50 dark:bg-gray-800/50 border-b border-gray-200 dark:border-gray-800">
                    <tr>
                      <th className="px-3 sm:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                        Rank
                      </th>
                      <th className="px-3 sm:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                        User
                      </th>
                      <th className="px-3 sm:px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                        Tokens
                      </th>
                      <th className="px-3 sm:px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                        Cost
                      </th>
                      <th className="px-3 sm:px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider hidden md:table-cell">
                        Submissions
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-200 dark:divide-gray-800">
                    {data.users.map((user) => (
                      <tr
                        key={user.userId}
                        className="hover:bg-gray-50 dark:hover:bg-gray-800/50 transition-colors"
                      >
                        <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap">
                          <span
                            className={`text-base sm:text-lg font-bold ${
                              user.rank === 1
                                ? "text-yellow-500"
                                : user.rank === 2
                                ? "text-gray-400"
                                : user.rank === 3
                                ? "text-amber-600"
                                : "text-gray-500 dark:text-gray-400"
                            }`}
                          >
                            #{user.rank}
                          </span>
                        </td>
                        <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap">
                          <Link
                            href={`/u/${user.username}`}
                            className="flex items-center gap-2 sm:gap-3 group"
                          >
                            <Avatar
                              src={user.avatarUrl || `https://github.com/${user.username}.png`}
                              alt={user.username}
                              size={40}
                            />
                            <div className="min-w-0">
                              <p className="font-medium text-sm sm:text-base text-gray-900 dark:text-white group-hover:text-green-600 dark:group-hover:text-green-400 transition-colors truncate max-w-[120px] sm:max-w-none">
                                {user.displayName || user.username}
                              </p>
                              <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400 truncate max-w-[120px] sm:max-w-none">
                                @{user.username}
                              </p>
                            </div>
                          </Link>
                        </td>
                        <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap text-right">
                          <span className="font-medium text-sm sm:text-base text-gray-900 dark:text-white">
                            {formatNumber(user.totalTokens)}
                          </span>
                        </td>
                        <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap text-right">
                          <span className="font-medium text-sm sm:text-base text-green-600 dark:text-green-400">
                            {formatCurrency(user.totalCost)}
                          </span>
                        </td>
                        <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap text-right hidden md:table-cell">
                          <span className="text-gray-500 dark:text-gray-400">
                            {user.submissionCount}
                          </span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>

              {/* Pagination */}
              {data.pagination.totalPages > 1 && (
                <div className="px-3 sm:px-6 py-3 sm:py-4 border-t border-gray-200 dark:border-gray-800 flex flex-col sm:flex-row items-center justify-between gap-3">
                  <p className="text-xs sm:text-sm text-gray-500 dark:text-gray-400 text-center sm:text-left">
                    Showing {(data.pagination.page - 1) * data.pagination.limit + 1}-{Math.min(
                      data.pagination.page * data.pagination.limit,
                      data.pagination.totalUsers
                    )}{" "}
                    of {data.pagination.totalUsers}
                  </p>
                  <Pagination
                    pageCount={data.pagination.totalPages}
                    currentPage={data.pagination.page}
                    onPageChange={(e, pageNum) => setPage(pageNum)}
                    showPages={{ narrow: false, regular: true, wide: true }}
                  />
                </div>
              )}
            </>
          )}
        </div>
        )}

        {/* CLI Instructions */}
        <div className="mt-8 p-6 bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white mb-3">
            Join the Leaderboard
          </h2>
          <p className="text-gray-600 dark:text-gray-300 mb-4">
            Install Token Tracker CLI and submit your usage data:
          </p>
          <div className="space-y-2 font-mono text-sm">
            <div className="p-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-gray-700 dark:text-gray-300">
              <span className="text-green-600 dark:text-green-400">$</span> npx
              token-tracker login
            </div>
            <div className="p-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-gray-700 dark:text-gray-300">
              <span className="text-green-600 dark:text-green-400">$</span> npx
              token-tracker submit
            </div>
          </div>
        </div>
      </main>

      <Footer />
    </div>
  );
}
