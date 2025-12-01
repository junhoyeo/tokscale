"use client";

import { useState } from "react";
import type { TokenContributionData } from "@/lib/types";
import { DataInput } from "@/components/DataInput";
import { GraphContainer } from "@/components/GraphContainer";

export default function Home() {
  const [data, setData] = useState<TokenContributionData | null>(null);

  const handleDataLoaded = (loadedData: TokenContributionData) => {
    setData(loadedData);
  };

  const handleReset = () => {
    setData(null);
  };

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-950 transition-colors">
      {/* Header */}
      <header className="border-b border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900">
        <div className="max-w-7xl mx-auto px-4 py-4 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-green-400 to-blue-500 flex items-center justify-center">
              <svg
                className="w-5 h-5 text-white"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
                />
              </svg>
            </div>
            <div>
              <h1 className="text-lg font-bold text-gray-900 dark:text-white">
                Token Tracker
              </h1>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                Visualize your AI token usage
              </p>
            </div>
          </div>

          {data && (
            <button
              onClick={handleReset}
              className="px-4 py-2 text-sm font-medium text-gray-600 dark:text-gray-300 hover:text-gray-900 dark:hover:text-white transition-colors"
            >
              Load Different Data
            </button>
          )}
        </div>
      </header>

      {/* Main Content */}
      <main className="max-w-7xl mx-auto px-4 py-8">
        {!data ? (
          <DataInput onDataLoaded={handleDataLoaded} />
        ) : (
          <div className="space-y-6">
            {/* Data Summary */}
            <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4">
              <div className="flex flex-wrap items-center gap-4 text-sm">
                <span className="text-gray-500 dark:text-gray-400">
                  Data loaded:
                </span>
                <span className="font-medium text-gray-900 dark:text-white">
                  {data.meta.dateRange.start} → {data.meta.dateRange.end}
                </span>
                <span className="text-gray-400 dark:text-gray-600">|</span>
                <span className="font-medium text-green-600 dark:text-green-400">
                  ${data.summary.totalCost.toFixed(2)} total
                </span>
                <span className="text-gray-400 dark:text-gray-600">|</span>
                <span className="text-gray-600 dark:text-gray-300">
                  {data.summary.activeDays} active days
                </span>
              </div>
            </div>

            {/* Graph Container */}
            <GraphContainer data={data} />
          </div>
        )}
      </main>

      {/* Footer */}
      <footer className="border-t border-gray-200 dark:border-gray-800 mt-auto">
        <div className="max-w-7xl mx-auto px-4 py-4">
          <p className="text-center text-xs text-gray-500 dark:text-gray-400">
            Token Tracker • Visualize OpenCode, Claude Code, Codex, and Gemini usage
          </p>
        </div>
      </footer>
    </div>
  );
}
