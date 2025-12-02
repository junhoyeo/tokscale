"use client";

import { useState } from "react";
import type { TokenContributionData } from "@/lib/types";
import { DataInput } from "@/components/DataInput";
import { GraphContainer } from "@/components/GraphContainer";
import { ThemeToggle } from "@/components/ThemeToggle";
import { useSettings } from "@/lib/useSettings";

export default function Home() {
  const [data, setData] = useState<TokenContributionData | null>(null);
  const { theme, setTheme, mounted } = useSettings();

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-950 transition-colors duration-300">
      <header className="sticky top-0 z-50 border-b border-gray-200/80 dark:border-gray-800/80 bg-white/80 dark:bg-gray-900/80 backdrop-blur-xl">
        <div className="max-w-7xl mx-auto px-6 py-5 flex items-center justify-between">
          <div className="flex items-center gap-4">
            <div className="w-10 h-10 rounded-2xl bg-gradient-to-br from-green-400 to-emerald-600 flex items-center justify-center shadow-lg shadow-green-500/25 dark:shadow-green-500/15">
              <svg className="w-5 h-5 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2.5} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
              </svg>
            </div>
            <div>
              <h1 className="text-xl font-bold tracking-tight text-gray-900 dark:text-white">Token Tracker</h1>
              <p className="text-sm text-gray-500 dark:text-gray-400">Visualize your AI token usage</p>
            </div>
          </div>

          <div className="flex items-center gap-4">
            <ThemeToggle theme={theme} onThemeChange={setTheme} mounted={mounted} />
            {data && (
              <button
                onClick={() => setData(null)}
                className="px-5 py-2.5 text-sm font-medium text-gray-600 dark:text-gray-300 hover:text-gray-900 dark:hover:text-white hover:bg-gray-100 dark:hover:bg-gray-800 rounded-full transition-all duration-200"
              >
                Load Different Data
              </button>
            )}
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-6 py-10">
        {!data ? (
          <DataInput onDataLoaded={setData} />
        ) : (
          <div className="space-y-8">
            <div className="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200/80 dark:border-gray-800/80 p-5 shadow-sm">
              <div className="flex flex-wrap items-center gap-4 text-sm">
                <span className="text-gray-500 dark:text-gray-400">Data loaded:</span>
                <span className="font-semibold text-gray-900 dark:text-white">{data.meta.dateRange.start} → {data.meta.dateRange.end}</span>
                <span className="text-gray-300 dark:text-gray-700">•</span>
                <span className="font-semibold text-green-600 dark:text-green-400">${data.summary.totalCost.toFixed(2)} total</span>
                <span className="text-gray-300 dark:text-gray-700">•</span>
                <span className="text-gray-600 dark:text-gray-300">{data.summary.activeDays} active days</span>
              </div>
            </div>
            <GraphContainer data={data} />
          </div>
        )}
      </main>

      <footer className="border-t border-gray-200/80 dark:border-gray-800/80 mt-auto bg-white/50 dark:bg-gray-900/50">
        <div className="max-w-7xl mx-auto px-6 py-6">
          <p className="text-center text-sm text-gray-500 dark:text-gray-400">
            Token Tracker • Visualize OpenCode, Claude Code, Codex, and Gemini usage
          </p>
        </div>
      </footer>
    </div>
  );
}
