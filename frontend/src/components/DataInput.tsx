"use client";

import { useState, useCallback } from "react";
import type { TokenContributionData } from "@/lib/types";
import { isValidContributionData } from "@/lib/utils";

interface DataInputProps {
  onDataLoaded: (data: TokenContributionData) => void;
}

export function DataInput({ onDataLoaded }: DataInputProps) {
  const [rawJson, setRawJson] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  const parseJson = useCallback(() => {
    setError(null);

    if (!rawJson.trim()) {
      setError("Please enter JSON data");
      return;
    }

    try {
      const parsed = JSON.parse(rawJson);

      if (!isValidContributionData(parsed)) {
        setError("Invalid data format. Expected TokenContributionData structure with meta, summary, years, and contributions.");
        return;
      }

      onDataLoaded(parsed);
    } catch (err) {
      setError(`Invalid JSON: ${err instanceof Error ? err.message : "Unknown error"}`);
    }
  }, [rawJson, onDataLoaded]);

  const loadSampleData = useCallback(async () => {
    setIsLoading(true);
    setError(null);

    try {
      const response = await fetch("/sample-data.json");
      if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`);

      const data = await response.json();
      if (!isValidContributionData(data)) throw new Error("Sample data has invalid format");

      setRawJson(JSON.stringify(data, null, 2));
      onDataLoaded(data);
    } catch (err) {
      setError(`Failed to load sample data: ${err instanceof Error ? err.message : "Unknown error"}`);
    } finally {
      setIsLoading(false);
    }
  }, [onDataLoaded]);

  return (
    <div className="w-full max-w-4xl mx-auto">
      <div className="mb-8">
        <h2 className="text-3xl font-bold tracking-tight mb-3 text-gray-900 dark:text-white">Load Token Usage Data</h2>
        <p className="text-base text-gray-600 dark:text-gray-400">
          Paste JSON from <code className="px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded-lg text-sm font-mono">token-tracker graph</code> command, or load sample data.
        </p>
      </div>

      <div className="mb-6">
        <textarea
          value={rawJson}
          onChange={(e) => { setRawJson(e.target.value); setError(null); }}
          onKeyDown={(e) => { if ((e.ctrlKey || e.metaKey) && e.key === "Enter") { e.preventDefault(); parseJson(); } }}
          placeholder='{"meta": {...}, "summary": {...}, "contributions": [...]}'
          className={`w-full h-72 p-4 font-mono text-sm rounded-2xl border-2 resize-y bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-600 ${
            error ? "border-red-400 focus:border-red-500 focus:ring-red-500/20" : "border-gray-200 dark:border-gray-700 focus:border-green-500 focus:ring-green-500/20"
          } focus:outline-none focus:ring-4 transition-all duration-200 shadow-sm`}
        />
        <p className="mt-2 text-sm text-gray-500 dark:text-gray-500">Tip: Press Ctrl+Enter (Cmd+Enter on Mac) to parse</p>
      </div>

      {error && (
        <div className="mb-6 p-4 rounded-2xl bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800/50">
          <p className="text-sm font-medium text-red-600 dark:text-red-400">{error}</p>
        </div>
      )}

      <div className="flex gap-4">
        <button
          onClick={parseJson}
          disabled={isLoading || !rawJson.trim()}
          className="px-6 py-3 rounded-full font-semibold text-sm bg-green-600 hover:bg-green-700 active:bg-green-800 text-white disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-200 hover:shadow-lg hover:shadow-green-500/25 hover:-translate-y-0.5 active:translate-y-0"
        >
          Parse JSON
        </button>

        <button
          onClick={loadSampleData}
          disabled={isLoading}
          className="px-6 py-3 rounded-full font-semibold text-sm bg-gray-100 hover:bg-gray-200 active:bg-gray-300 dark:bg-gray-800 dark:hover:bg-gray-700 dark:active:bg-gray-600 text-gray-800 dark:text-gray-200 disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-200 hover:shadow-md hover:-translate-y-0.5 active:translate-y-0"
        >
          {isLoading ? (
            <span className="flex items-center gap-2">
              <svg className="animate-spin h-4 w-4" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
              </svg>
              Loading...
            </span>
          ) : (
            "Load Sample Data"
          )}
        </button>
      </div>

      <div className="mt-10 p-6 rounded-2xl bg-gray-50 dark:bg-gray-800/50 border border-gray-200/80 dark:border-gray-700/80">
        <h3 className="text-base font-bold mb-4 text-gray-800 dark:text-gray-200">How to get your data</h3>
        <ol className="text-sm text-gray-600 dark:text-gray-400 space-y-3 list-decimal list-inside">
          <li className="leading-relaxed">
            Install token-tracker: <code className="px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded-lg text-xs font-mono">npx tsx src/cli.ts graph</code>
          </li>
          <li className="leading-relaxed">
            Run the graph command: <code className="px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded-lg text-xs font-mono">token-tracker graph</code>
          </li>
          <li className="leading-relaxed">Copy the JSON output and paste it above</li>
        </ol>
      </div>
    </div>
  );
}
