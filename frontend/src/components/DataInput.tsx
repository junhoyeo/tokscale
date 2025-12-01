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
        setError(
          "Invalid data format. Expected TokenContributionData structure with meta, summary, years, and contributions."
        );
        return;
      }

      onDataLoaded(parsed);
      setError(null);
    } catch (err) {
      setError(
        `Invalid JSON: ${err instanceof Error ? err.message : "Unknown error"}`
      );
    }
  }, [rawJson, onDataLoaded]);

  const loadSampleData = useCallback(async () => {
    setIsLoading(true);
    setError(null);

    try {
      const response = await fetch("/sample-data.json");
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      const data = await response.json();

      if (!isValidContributionData(data)) {
        throw new Error("Sample data has invalid format");
      }

      setRawJson(JSON.stringify(data, null, 2));
      onDataLoaded(data);
    } catch (err) {
      setError(
        `Failed to load sample data: ${
          err instanceof Error ? err.message : "Unknown error"
        }`
      );
    } finally {
      setIsLoading(false);
    }
  }, [onDataLoaded]);

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setRawJson(e.target.value);
    setError(null);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Ctrl/Cmd + Enter to parse
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      parseJson();
    }
  };

  return (
    <div className="w-full max-w-4xl mx-auto p-4">
      <div className="mb-4">
        <h2 className="text-xl font-semibold mb-2 text-gray-800 dark:text-gray-200">
          Load Token Usage Data
        </h2>
        <p className="text-sm text-gray-600 dark:text-gray-400">
          Paste JSON from{" "}
          <code className="px-1 py-0.5 bg-gray-100 dark:bg-gray-800 rounded text-xs">
            token-tracker graph
          </code>{" "}
          command, or load sample data.
        </p>
      </div>

      {/* Textarea */}
      <div className="mb-4">
        <textarea
          value={rawJson}
          onChange={handleTextareaChange}
          onKeyDown={handleKeyDown}
          placeholder='{"meta": {...}, "summary": {...}, "contributions": [...]}'
          className={`w-full h-64 p-3 font-mono text-sm rounded-lg border resize-y
            bg-white dark:bg-gray-900
            text-gray-900 dark:text-gray-100
            placeholder-gray-400 dark:placeholder-gray-600
            ${
              error
                ? "border-red-500 focus:border-red-500 focus:ring-red-500"
                : "border-gray-300 dark:border-gray-700 focus:border-blue-500 focus:ring-blue-500"
            }
            focus:outline-none focus:ring-2 focus:ring-opacity-50
            transition-colors`}
        />
        <p className="mt-1 text-xs text-gray-500 dark:text-gray-500">
          Tip: Press Ctrl+Enter (Cmd+Enter on Mac) to parse
        </p>
      </div>

      {/* Error message */}
      {error && (
        <div className="mb-4 p-3 rounded-lg bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800">
          <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
        </div>
      )}

      {/* Buttons */}
      <div className="flex gap-3">
        <button
          onClick={parseJson}
          disabled={isLoading || !rawJson.trim()}
          className={`px-4 py-2 rounded-lg font-medium text-sm
            bg-blue-600 hover:bg-blue-700 active:bg-blue-800
            text-white
            disabled:opacity-50 disabled:cursor-not-allowed
            transition-colors`}
        >
          Parse JSON
        </button>

        <button
          onClick={loadSampleData}
          disabled={isLoading}
          className={`px-4 py-2 rounded-lg font-medium text-sm
            bg-gray-200 hover:bg-gray-300 active:bg-gray-400
            dark:bg-gray-700 dark:hover:bg-gray-600 dark:active:bg-gray-500
            text-gray-800 dark:text-gray-200
            disabled:opacity-50 disabled:cursor-not-allowed
            transition-colors`}
        >
          {isLoading ? (
            <span className="flex items-center gap-2">
              <svg
                className="animate-spin h-4 w-4"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                ></circle>
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                ></path>
              </svg>
              Loading...
            </span>
          ) : (
            "Load Sample Data"
          )}
        </button>
      </div>

      {/* Instructions */}
      <div className="mt-6 p-4 rounded-lg bg-gray-50 dark:bg-gray-800/50 border border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-semibold mb-2 text-gray-700 dark:text-gray-300">
          How to get your data
        </h3>
        <ol className="text-sm text-gray-600 dark:text-gray-400 space-y-1 list-decimal list-inside">
          <li>
            Install token-tracker:{" "}
            <code className="px-1 py-0.5 bg-gray-100 dark:bg-gray-800 rounded text-xs">
              npx tsx src/cli.ts graph
            </code>
          </li>
          <li>
            Run the graph command:{" "}
            <code className="px-1 py-0.5 bg-gray-100 dark:bg-gray-800 rounded text-xs">
              token-tracker graph
            </code>
          </li>
          <li>Copy the JSON output and paste it above</li>
        </ol>
      </div>
    </div>
  );
}
