"use client";

import { useState } from "react";
import type { TokenContributionData } from "@/lib/types";
import { DataInput } from "@/components/DataInput";
import { GraphContainer } from "@/components/GraphContainer";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";

export default function LocalViewerPage() {
  const [data, setData] = useState<TokenContributionData | null>(null);

  return (
    <div className="min-h-screen flex flex-col" style={{ backgroundColor: "#141415" }}>
      <Navigation />

      <main className="flex-1 max-w-7xl mx-auto px-6 py-10 w-full">
        <div className="mb-8">
          <h1 className="text-3xl font-bold mb-2" style={{ color: "#FFFFFF" }}>
            Local Viewer
          </h1>
          <p style={{ color: "#696969" }}>
            View your token usage data locally without submitting
          </p>
        </div>

        {!data ? (
          <DataInput onDataLoaded={setData} />
        ) : (
          <div className="space-y-8">
            <div
              className="rounded-2xl border p-5"
              style={{ backgroundColor: "#141415", borderColor: "#262627" }}
            >
              <div className="flex flex-wrap items-center gap-4 text-sm">
                <span style={{ color: "#696969" }}>Data loaded:</span>
                <span className="font-semibold" style={{ color: "#FFFFFF" }}>
                  {data.meta.dateRange.start} - {data.meta.dateRange.end}
                </span>
                <span style={{ color: "#3F3F3F" }}>|</span>
                <span className="font-semibold" style={{ color: "#53d1f3" }}>
                  ${data.summary.totalCost.toFixed(2)} total
                </span>
                <span style={{ color: "#3F3F3F" }}>|</span>
                <span style={{ color: "#9CA3AF" }}>
                  {data.summary.activeDays} active days
                </span>
                <button
                  onClick={() => setData(null)}
                  className="ml-auto px-4 py-2 text-sm font-medium rounded-lg transition-opacity hover:opacity-80"
                  style={{ color: "#9CA3AF" }}
                >
                  Load Different Data
                </button>
              </div>
            </div>
            <GraphContainer data={data} />
          </div>
        )}
      </main>

      <Footer />
    </div>
  );
}
