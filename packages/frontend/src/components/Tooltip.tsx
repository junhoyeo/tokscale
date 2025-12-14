"use client";

import { useRef } from "react";
import type { DailyContribution, TooltipPosition, GraphColorPalette } from "@/lib/types";
import { formatDate, formatCurrency, formatTokenCount } from "@/lib/utils";

interface TooltipProps {
  day: DailyContribution | null;
  position: TooltipPosition | null;
  visible: boolean;
  palette: GraphColorPalette;
}

function useAdjustedPosition(
  position: TooltipPosition | null,
  visible: boolean,
  tooltipRef: React.RefObject<HTMLDivElement | null>
): TooltipPosition | null {
  if (!visible || !position) return null;

  const tooltip = tooltipRef.current;
  if (!tooltip) {
    return { x: position.x + 15, y: position.y + 15 };
  }

  const rect = tooltip.getBoundingClientRect();
  const viewportWidth = typeof window !== "undefined" ? window.innerWidth : 1920;
  const viewportHeight = typeof window !== "undefined" ? window.innerHeight : 1080;

  let x = position.x + 15;
  let y = position.y + 15;

  if (x + rect.width > viewportWidth - 10) {
    x = position.x - rect.width - 15;
  }
  if (y + rect.height > viewportHeight - 10) {
    y = position.y - rect.height - 15;
  }

  return { x: Math.max(10, x), y: Math.max(10, y) };
}

export function Tooltip({ day, position, visible, palette }: TooltipProps) {
  const tooltipRef = useRef<HTMLDivElement>(null);
  const adjustedPosition = useAdjustedPosition(position, visible, tooltipRef);

  if (!visible || !day || !adjustedPosition) return null;

  const { totals, tokenBreakdown } = day;

  return (
    <div
      ref={tooltipRef}
      className="fixed z-50 pointer-events-none"
      style={{ left: adjustedPosition.x, top: adjustedPosition.y }}
    >
      <div
        className="rounded-2xl shadow-xl border p-4 min-w-[220px] backdrop-blur-sm"
        style={{
          backgroundColor: "#1F1F20",
          borderColor: "#262627",
          color: "#FFFFFF",
        }}
      >
        <div className="font-bold text-base mb-3" style={{ color: "#FFFFFF" }}>
          {formatDate(day.date)}
        </div>

        <div className="border-t my-3" style={{ borderColor: "#262627" }} />

        <div className="flex justify-between items-center mb-3">
          <span className="text-sm font-medium" style={{ color: "#696969" }}>Cost</span>
          <span
            className="font-bold text-xl tracking-tight"
            style={{
              color: day.intensity >= 3 ? palette.grade4 : day.intensity >= 2 ? palette.grade3 : "#FFFFFF",
            }}
          >
            {formatCurrency(totals.cost)}
          </span>
        </div>

        <div className="border-t my-3" style={{ borderColor: "#262627" }} />

        <div className="space-y-2 text-sm">
          <TokenRow label="Input" value={tokenBreakdown.input} />
          <TokenRow label="Output" value={tokenBreakdown.output} />
          <TokenRow label="Cache Read" value={tokenBreakdown.cacheRead} />
          <TokenRow label="Cache Write" value={tokenBreakdown.cacheWrite} />
          {tokenBreakdown.reasoning > 0 && <TokenRow label="Reasoning" value={tokenBreakdown.reasoning} />}
        </div>

        <div className="border-t my-3" style={{ borderColor: "#262627" }} />

        <div className="flex justify-between items-center">
          <span className="text-sm font-semibold" style={{ color: "#696969" }}>Total</span>
          <span className="font-bold" style={{ color: "#FFFFFF" }}>
            {formatTokenCount(totals.tokens)} tokens
          </span>
        </div>

        <div className="flex justify-between items-center mt-2">
          <span className="text-sm font-medium" style={{ color: "#696969" }}>Messages</span>
          <span className="text-sm font-semibold" style={{ color: "#FFFFFF" }}>
            {totals.messages.toLocaleString()}
          </span>
        </div>
      </div>
    </div>
  );
}

function TokenRow({ label, value }: { label: string; value: number }) {
  if (value === 0) return null;

  return (
    <div className="flex justify-between items-center">
      <span className="font-medium" style={{ color: "#696969" }}>{label}</span>
      <span className="font-mono font-semibold" style={{ color: "#FFFFFF" }}>
        {formatTokenCount(value)}
      </span>
    </div>
  );
}
