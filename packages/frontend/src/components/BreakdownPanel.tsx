"use client";

import type { DailyContribution, GraphColorPalette, SourceType } from "@/lib/types";
import { formatDateFull, formatCurrency, formatTokenCount, groupSourcesByType, sortSourcesByCost } from "@/lib/utils";
import { SOURCE_DISPLAY_NAMES, SOURCE_COLORS } from "@/lib/constants";
import { ProviderLogo } from "./ProviderLogo";
import { SourceLogo } from "./SourceLogo";

interface BreakdownPanelProps {
  day: DailyContribution | null;
  onClose: () => void;
  palette: GraphColorPalette;
}

export function BreakdownPanel({ day, onClose, palette }: BreakdownPanelProps) {
  if (!day) return null;

  const groupedSources = groupSourcesByType(day.sources);
  const sortedSourceTypes = Array.from(groupedSources.keys()).sort();

  return (
    <div
      className="mt-8 rounded-2xl border overflow-hidden shadow-sm transition-shadow hover:shadow-md"
      style={{ backgroundColor: "#1F1F20", borderColor: "#262627" }}
    >
      <div className="flex items-center justify-between px-6 py-4 border-b" style={{ borderColor: "#262627" }}>
        <h3 className="font-bold text-lg" style={{ color: "#FFFFFF" }}>
          {formatDateFull(day.date)} - Detailed Breakdown
        </h3>
        <button
          onClick={onClose}
          className="p-2 rounded-full hover:bg-white/10 transition-all duration-200 hover:scale-110"
          style={{ color: "#696969" }}
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <div className="p-6">
        {day.sources.length === 0 ? (
          <p className="text-center py-8 text-sm font-medium" style={{ color: "#696969" }}>
            No activity on this day
          </p>
        ) : (
          <div className="space-y-6">
            {sortedSourceTypes.map((sourceType) => {
              const sources = sortSourcesByCost(groupedSources.get(sourceType) || []);
              const sourceTotalCost = sources.reduce((sum, s) => sum + s.cost, 0);
              return (
                <SourceSection key={sourceType} sourceType={sourceType} sources={sources} totalCost={sourceTotalCost} palette={palette} />
              );
            })}
          </div>
        )}

        {day.sources.length > 0 && (
          <div className="mt-6 pt-6 border-t flex flex-wrap gap-6 text-sm" style={{ borderColor: "#262627" }}>
            <div className="font-medium" style={{ color: "#696969" }}>
              Total: <span className="font-bold text-base" style={{ color: "#FFFFFF" }}>{formatCurrency(day.totals.cost)}</span>
            </div>
            <div className="font-medium" style={{ color: "#696969" }}>
              across <span className="font-semibold" style={{ color: "#FFFFFF" }}>{sortedSourceTypes.length} source{sortedSourceTypes.length !== 1 ? "s" : ""}</span>
            </div>
            <div className="font-medium" style={{ color: "#696969" }}>
              <span className="font-semibold" style={{ color: "#FFFFFF" }}>
                {new Set(day.sources.map((s) => s.modelId)).size} model{new Set(day.sources.map((s) => s.modelId)).size !== 1 ? "s" : ""}
              </span>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

interface SourceSectionProps {
  sourceType: SourceType;
  sources: DailyContribution["sources"];
  totalCost: number;
  palette: GraphColorPalette;
}

function SourceSection({ sourceType, sources, totalCost, palette }: SourceSectionProps) {
  const sourceColor = SOURCE_COLORS[sourceType] || palette.grade3;

  return (
    <div>
      <div className="flex items-center gap-3 mb-3">
        <span
          className="inline-flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-semibold transition-transform hover:scale-105"
          style={{ backgroundColor: `${sourceColor}20`, color: sourceColor }}
        >
          <SourceLogo sourceId={sourceType} height={14} />
          {SOURCE_DISPLAY_NAMES[sourceType] || sourceType}
        </span>
        <span className="text-sm font-bold" style={{ color: "#FFFFFF" }}>{formatCurrency(totalCost)}</span>
      </div>

      <div className="ml-5 space-y-3">
        {sources.map((source, index) => (
          <ModelRow key={`${source.modelId}-${index}`} source={source} isLast={index === sources.length - 1} palette={palette} />
        ))}
      </div>
    </div>
  );
}

interface ModelRowProps {
  source: DailyContribution["sources"][0];
  isLast: boolean;
  palette: GraphColorPalette;
}

function ModelRow({ source, isLast, palette }: ModelRowProps) {
  const { modelId, providerId, tokens, cost, messages } = source;

  return (
    <div className="relative">
      <div className="absolute left-0 top-0 w-4 h-full" style={{ color: "#696969" }}>
        <span className="absolute left-0 top-3 w-3 border-t" style={{ borderColor: "#262627" }} />
        {!isLast && <span className="absolute left-0 top-0 h-full border-l" style={{ borderColor: "#262627" }} />}
      </div>

      <div className="ml-6">
        <div className="flex items-center gap-3 flex-wrap">
          <span className="font-mono text-sm font-semibold" style={{ color: "#FFFFFF" }}>{modelId}</span>
          {providerId && (
            <span
              className="inline-flex items-center px-2 py-1 rounded-md"
              style={{ backgroundColor: "rgba(133, 202, 255, 0.15)", color: "#696969" }}
            >
              <ProviderLogo providerId={providerId} height={12} />
            </span>
          )}
          <span className="font-bold text-sm" style={{ color: palette.grade4 }}>{formatCurrency(cost)}</span>
        </div>

        <div className="mt-2 grid grid-cols-2 sm:grid-cols-3 md:grid-cols-5 gap-x-4 gap-y-2 text-xs">
          {tokens.input > 0 && <TokenBadge label="Input" value={tokens.input} />}
          {tokens.output > 0 && <TokenBadge label="Output" value={tokens.output} />}
          {tokens.cacheRead > 0 && <TokenBadge label="Cache Read" value={tokens.cacheRead} />}
          {tokens.cacheWrite > 0 && <TokenBadge label="Cache Write" value={tokens.cacheWrite} />}
          {tokens.reasoning > 0 && <TokenBadge label="Reasoning" value={tokens.reasoning} />}
        </div>

        <div className="mt-2 text-xs font-medium" style={{ color: "#696969" }}>
          {messages.toLocaleString()} message{messages !== 1 ? "s" : ""}
        </div>
      </div>
    </div>
  );
}

function TokenBadge({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="font-medium" style={{ color: "#696969" }}>{label}:</span>
      <span className="font-mono font-semibold" style={{ color: "#FFFFFF" }}>{formatTokenCount(value)}</span>
    </div>
  );
}
