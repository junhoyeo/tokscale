"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { Pagination, Avatar } from "@primer/react";
import { TabBar } from "@/components/TabBar";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { LeaderboardSkeleton } from "@/components/Skeleton";
import { BlackholeHero } from "@/components/BlackholeHero";
import { formatNumber, formatCurrency } from "@/lib/utils";

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

export default function LeaderboardPage() {
  const [data, setData] = useState<LeaderboardData | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [period, setPeriod] = useState<Period>("all");
  const [page, setPage] = useState(1);

  useEffect(() => {
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
    <div className="min-h-screen flex flex-col" style={{ backgroundColor: "#141415" }}>
      <Navigation />

      <main className="flex-1 max-w-7xl mx-auto px-6 py-10 w-full">
        <BlackholeHero />

        <div className="mb-10">
          <h1 className="text-3xl font-bold mb-2" style={{ color: "#FFFFFF" }}>
            Leaderboard
          </h1>
          <p className="mb-6" style={{ color: "#696969" }}>
            See who&apos;s using the most AI tokens
          </p>

          <div className="grid grid-cols-2 md:grid-cols-4 gap-3 sm:gap-4">
            <div
              className="rounded-xl border p-3 sm:p-4"
              style={{ backgroundColor: "#141415", borderColor: "#262627" }}
            >
              <p className="text-xs sm:text-sm" style={{ color: "#696969" }}>
                Total Tokens
              </p>
              <p className="text-xl sm:text-2xl font-bold" style={{ color: "#FFFFFF" }}>
                {data ? formatNumber(data.stats.totalTokens) : "-"}
              </p>
            </div>
            <div
              className="rounded-xl border p-3 sm:p-4"
              style={{ backgroundColor: "#141415", borderColor: "#262627" }}
            >
              <p className="text-xs sm:text-sm" style={{ color: "#696969" }}>
                Total Cost
              </p>
              <p className="text-xl sm:text-2xl font-bold" style={{ color: "#53d1f3" }}>
                {data ? formatCurrency(data.stats.totalCost) : "-"}
              </p>
            </div>
            <div
              className="rounded-xl border p-3 sm:p-4"
              style={{ backgroundColor: "#141415", borderColor: "#262627" }}
            >
              <p className="text-xs sm:text-sm" style={{ color: "#696969" }}>
                Users
              </p>
              <p className="text-xl sm:text-2xl font-bold" style={{ color: "#FFFFFF" }}>
                {data ? data.stats.uniqueUsers : "-"}
              </p>
            </div>
            <div
              className="rounded-xl border p-3 sm:p-4"
              style={{ backgroundColor: "#141415", borderColor: "#262627" }}
            >
              <p className="text-xs sm:text-sm" style={{ color: "#696969" }}>
                Submissions
              </p>
              <p className="text-xl sm:text-2xl font-bold" style={{ color: "#FFFFFF" }}>
                {data ? data.stats.totalSubmissions : "-"}
              </p>
            </div>
          </div>
        </div>

        <div className="mb-6">
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
        </div>

        {isLoading ? (
          <LeaderboardSkeleton />
        ) : (
          <div
            className="rounded-2xl border overflow-hidden"
            style={{ backgroundColor: "#141415", borderColor: "#262627" }}
          >
            {!data || data.users.length === 0 ? (
              <div className="p-8 text-center">
                <p className="mb-4" style={{ color: "#696969" }}>
                  No submissions yet. Be the first!
                </p>
                <p className="text-sm" style={{ color: "#525252" }}>
                  Run{" "}
                  <code
                    className="px-2 py-1 rounded"
                    style={{ backgroundColor: "#262627" }}
                  >
                    tokscale login && tokscale submit
                  </code>
                </p>
              </div>
            ) : (
              <>
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[500px]">
                    <thead
                      className="border-b"
                      style={{ backgroundColor: "#1F1F20", borderColor: "#262627" }}
                    >
                      <tr>
                        <th
                          className="px-3 sm:px-6 py-3 text-left text-xs font-medium uppercase tracking-wider"
                          style={{ color: "#696969" }}
                        >
                          Rank
                        </th>
                        <th
                          className="px-3 sm:px-6 py-3 text-left text-xs font-medium uppercase tracking-wider"
                          style={{ color: "#696969" }}
                        >
                          User
                        </th>
                        <th
                          className="px-3 sm:px-6 py-3 text-right text-xs font-medium uppercase tracking-wider"
                          style={{ color: "#696969" }}
                        >
                          Tokens
                        </th>
                        <th
                          className="px-3 sm:px-6 py-3 text-right text-xs font-medium uppercase tracking-wider"
                          style={{ color: "#696969" }}
                        >
                          Cost
                        </th>
                        <th
                          className="px-3 sm:px-6 py-3 text-right text-xs font-medium uppercase tracking-wider hidden md:table-cell"
                          style={{ color: "#696969" }}
                        >
                          Submissions
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {data.users.map((user, index) => (
                        <tr
                          key={user.userId}
                          className="transition-colors hover:opacity-80"
                          style={{
                            borderBottom: index < data.users.length - 1 ? "1px solid #262627" : "none",
                          }}
                        >
                          <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap">
                            <span
                              className="text-base sm:text-lg font-bold"
                              style={{
                                color:
                                  user.rank === 1
                                    ? "#EAB308"
                                    : user.rank === 2
                                    ? "#9CA3AF"
                                    : user.rank === 3
                                    ? "#D97706"
                                    : "#696969",
                              }}
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
                                <p
                                  className="font-medium text-sm sm:text-base truncate max-w-[120px] sm:max-w-none group-hover:opacity-80 transition-opacity"
                                  style={{ color: "#FFFFFF" }}
                                >
                                  {user.displayName || user.username}
                                </p>
                                <p
                                  className="text-xs sm:text-sm truncate max-w-[120px] sm:max-w-none"
                                  style={{ color: "#696969" }}
                                >
                                  @{user.username}
                                </p>
                              </div>
                            </Link>
                          </td>
                          <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap text-right">
                            <span
                              className="font-medium text-sm sm:text-base"
                              style={{ color: "#FFFFFF" }}
                            >
                              {formatNumber(user.totalTokens)}
                            </span>
                          </td>
                          <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap text-right">
                            <span
                              className="font-medium text-sm sm:text-base"
                              style={{ color: "#53d1f3" }}
                            >
                              {formatCurrency(user.totalCost)}
                            </span>
                          </td>
                          <td className="px-3 sm:px-6 py-3 sm:py-4 whitespace-nowrap text-right hidden md:table-cell">
                            <span style={{ color: "#696969" }}>{user.submissionCount}</span>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>

                {data.pagination.totalPages > 1 && (
                  <div
                    className="px-3 sm:px-6 py-3 sm:py-4 border-t flex flex-col sm:flex-row items-center justify-between gap-3"
                    style={{ borderColor: "#262627" }}
                  >
                    <p className="text-xs sm:text-sm text-center sm:text-left" style={{ color: "#696969" }}>
                      Showing {(data.pagination.page - 1) * data.pagination.limit + 1}-
                      {Math.min(data.pagination.page * data.pagination.limit, data.pagination.totalUsers)} of{" "}
                      {data.pagination.totalUsers}
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

        <div
          className="mt-8 p-6 rounded-2xl border"
          style={{ backgroundColor: "#141415", borderColor: "#262627" }}
        >
          <h2 className="text-lg font-semibold mb-3" style={{ color: "#FFFFFF" }}>
            Join the Leaderboard
          </h2>
          <p className="mb-4" style={{ color: "#9CA3AF" }}>
            Install Tokscale CLI and submit your usage data:
          </p>
          <div className="space-y-2 font-mono text-sm">
            <div className="p-3 rounded-lg" style={{ backgroundColor: "#1F1F20", color: "#9CA3AF" }}>
              <span style={{ color: "#53d1f3" }}>$</span> npx tokscale login
            </div>
            <div className="p-3 rounded-lg" style={{ backgroundColor: "#1F1F20", color: "#9CA3AF" }}>
              <span style={{ color: "#53d1f3" }}>$</span> npx tokscale submit
            </div>
          </div>
        </div>
      </main>

      <Footer />
    </div>
  );
}
