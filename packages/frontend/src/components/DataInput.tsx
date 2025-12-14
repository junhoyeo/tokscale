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
        <h2 className="text-3xl font-bold tracking-tight mb-3" style={{ color: "#FFFFFF" }}>
          Load Token Usage Data
        </h2>
        <p style={{ color: "#696969" }}>
          Paste JSON from{" "}
          <code
            className="px-2 py-1 rounded-lg text-sm font-mono"
            style={{ backgroundColor: "#262627" }}
          >
            tokscale graph
          </code>{" "}
          command, or load sample data.
        </p>
      </div>

      <div className="mb-6">
        <textarea
          value={rawJson}
          onChange={(e) => {
            setRawJson(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              parseJson();
            }
          }}
          placeholder='{"meta": {...}, "summary": {...}, "contributions": [...]}'
          className={`w-full h-72 p-4 font-mono text-sm rounded-2xl border-2 resize-y focus:outline-none focus:ring-4 transition-all duration-200 ${
            error ? "border-red-500 focus:ring-red-500/20" : "border-[#262627]"
          }`}
          style={{
            backgroundColor: "#1F1F20",
            color: "#FFFFFF",
          }}
          onFocus={(e) => {
            if (!error) {
              e.currentTarget.style.borderColor = "#53d1f3";
            }
          }}
          onBlur={(e) => {
            if (!error) {
              e.currentTarget.style.borderColor = "#262627";
            }
          }}
        />
        <p className="mt-2 text-sm" style={{ color: "#696969" }}>
          Tip: Press Ctrl+Enter (Cmd+Enter on Mac) to parse
        </p>
      </div>

      {error && (
        <div
          className="mb-6 p-4 rounded-2xl border"
          style={{ backgroundColor: "rgba(239, 68, 68, 0.1)", borderColor: "rgba(239, 68, 68, 0.3)" }}
        >
          <p className="text-sm font-medium" style={{ color: "#EF4444" }}>
            {error}
          </p>
        </div>
      )}

      <div className="flex gap-4">
        <button
          onClick={parseJson}
          disabled={isLoading || !rawJson.trim()}
          className="px-6 py-3 rounded-full font-semibold text-sm text-white disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-200 hover:shadow-lg hover:-translate-y-0.5 active:translate-y-0 hover:opacity-90"
          style={{ backgroundColor: "#53d1f3", boxShadow: "0 10px 15px -3px rgba(83, 209, 243, 0.25)" }}
        >
          Parse JSON
        </button>

        <button
          onClick={loadSampleData}
          disabled={isLoading}
          className="px-6 py-3 rounded-full font-semibold text-sm disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-200 hover:shadow-md hover:-translate-y-0.5 active:translate-y-0"
          style={{ backgroundColor: "#262627", color: "#FFFFFF" }}
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
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </svg>
              Loading...
            </span>
          ) : (
            "Load Sample Data"
          )}
        </button>
      </div>

      <div
        className="mt-10 p-6 rounded-2xl border"
        style={{ backgroundColor: "#1F1F20", borderColor: "#262627" }}
      >
        <h3 className="text-base font-bold mb-4" style={{ color: "#FFFFFF" }}>
          How to get your data
        </h3>
        <ol className="text-sm space-y-3 list-decimal list-inside" style={{ color: "#696969" }}>
          <li className="leading-relaxed">
            Install token-tracker:{" "}
            <code
              className="px-2 py-1 rounded-lg text-xs font-mono"
              style={{ backgroundColor: "#262627" }}
            >
              npx tsx src/cli.ts graph
            </code>
          </li>
          <li className="leading-relaxed">
            Run the graph command:{" "}
            <code
              className="px-2 py-1 rounded-lg text-xs font-mono"
              style={{ backgroundColor: "#262627" }}
            >
              tokscale graph
            </code>
          </li>
          <li className="leading-relaxed">Copy the JSON output and paste it above</li>
        </ol>
      </div>
    </div>
  );
}
