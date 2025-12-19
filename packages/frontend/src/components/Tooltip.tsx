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
      role="tooltip"
      className="fixed z-50 pointer-events-none"
      style={{ left: adjustedPosition.x, top: adjustedPosition.y }}
    >
      <div
        className="rounded-2xl shadow-xl border p-4 min-w-[220px] backdrop-blur-sm"
        style={{
          backgroundColor: "var(--color-card-bg)",
          borderColor: "var(--color-border-default)",
          color: "var(--color-fg-default)",
        }}
      >
        <div className="font-bold text-base mb-3" style={{ color: "var(--color-fg-default)" }}>
          {formatDate(day.date)}
        </div>

        <div className="border-t my-3" style={{ borderColor: "var(--color-border-default)" }} />

        <div className="flex justify-between items-center mb-3">
          <span className="text-sm font-medium" style={{ color: "var(--color-fg-muted)" }}>Total Tokens</span>
          <span
            className="font-bold text-xl tracking-tight"
            style={{
              color: day.intensity >= 3 ? palette.grade1 : day.intensity >= 2 ? palette.grade2 : "var(--color-fg-default)",
            }}
          >
            {formatTokenCount(totals.tokens)}
          </span>
        </div>

        <div className="border-t my-3" style={{ borderColor: "var(--color-border-default)" }} />

        <div className="space-y-2 text-sm">
          <TokenRow label="Input" value={tokenBreakdown.input} />
          <TokenRow label="Output" value={tokenBreakdown.output} />
          <TokenRow label="Cache Read" value={tokenBreakdown.cacheRead} />
          <TokenRow label="Cache Write" value={tokenBreakdown.cacheWrite} />
          {tokenBreakdown.reasoning > 0 && <TokenRow label="Reasoning" value={tokenBreakdown.reasoning} />}
        </div>

        <div className="border-t my-3" style={{ borderColor: "var(--color-border-default)" }} />

        <div className="flex justify-between items-center">
          <span className="text-sm font-semibold" style={{ color: "var(--color-fg-muted)" }}>Cost</span>
          <span className="font-bold" style={{ color: "var(--color-fg-default)" }}>
            {formatCurrency(totals.cost)}
          </span>
        </div>

        <div className="flex justify-between items-center mt-2">
          <span className="text-sm font-medium" style={{ color: "var(--color-fg-muted)" }}>Messages</span>
          <span className="text-sm font-semibold" style={{ color: "var(--color-fg-default)" }}>
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
      <span className="font-medium" style={{ color: "var(--color-fg-muted)" }}>{label}</span>
      <span className="font-mono font-semibold" style={{ color: "var(--color-fg-default)" }}>
        {formatTokenCount(value)}
      </span>
    </div>
  );
}
