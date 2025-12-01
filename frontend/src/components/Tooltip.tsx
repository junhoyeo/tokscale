"use client";

import { useEffect, useRef, useState } from "react";
import type { DailyContribution, TooltipPosition, GraphColorPalette } from "@/lib/types";
import { formatDate, formatCurrency, formatTokenCount } from "@/lib/utils";

interface TooltipProps {
  day: DailyContribution | null;
  position: TooltipPosition | null;
  visible: boolean;
  palette: GraphColorPalette;
}

export function Tooltip({ day, position, visible, palette }: TooltipProps) {
  const tooltipRef = useRef<HTMLDivElement>(null);
  const [adjustedPosition, setAdjustedPosition] = useState<TooltipPosition | null>(null);

  // Adjust position to prevent overflow
  useEffect(() => {
    if (!visible || !position || !tooltipRef.current) {
      setAdjustedPosition(null);
      return;
    }

    const tooltip = tooltipRef.current;
    const rect = tooltip.getBoundingClientRect();
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;

    let x = position.x + 15; // Offset from cursor
    let y = position.y + 15;

    // Prevent horizontal overflow
    if (x + rect.width > viewportWidth - 10) {
      x = position.x - rect.width - 15;
    }

    // Prevent vertical overflow
    if (y + rect.height > viewportHeight - 10) {
      y = position.y - rect.height - 15;
    }

    // Ensure minimum position
    x = Math.max(10, x);
    y = Math.max(10, y);

    setAdjustedPosition({ x, y });
  }, [visible, position]);

  if (!visible || !day || !adjustedPosition) return null;

  const { totals, tokenBreakdown } = day;

  return (
    <div
      ref={tooltipRef}
      className="fixed z-50 pointer-events-none"
      style={{
        left: adjustedPosition.x,
        top: adjustedPosition.y,
      }}
    >
      <div
        className="rounded-lg shadow-lg border p-3 min-w-[200px]"
        style={{
          backgroundColor: "var(--color-card-bg)",
          borderColor: "var(--color-border-default)",
          color: "var(--color-fg-default)",
        }}
      >
        {/* Date */}
        <div className="font-semibold mb-2" style={{ color: "var(--color-fg-default)" }}>
          {formatDate(day.date)}
        </div>

        {/* Divider */}
        <div className="border-t my-2" style={{ borderColor: "var(--color-border-muted)" }} />

        {/* Cost (highlighted) */}
        <div className="flex justify-between items-center mb-2">
          <span className="text-sm" style={{ color: "var(--color-fg-muted)" }}>
            Cost
          </span>
          <span
            className="font-bold text-lg"
            style={{
              color:
                day.intensity >= 3
                  ? palette.grade4
                  : day.intensity >= 2
                  ? palette.grade3
                  : "var(--color-fg-default)",
            }}
          >
            {formatCurrency(totals.cost)}
          </span>
        </div>

        {/* Divider */}
        <div className="border-t my-2" style={{ borderColor: "var(--color-border-muted)" }} />

        {/* Token breakdown */}
        <div className="space-y-1 text-sm">
          <TokenRow label="Input" value={tokenBreakdown.input} />
          <TokenRow label="Output" value={tokenBreakdown.output} />
          <TokenRow label="Cache Read" value={tokenBreakdown.cacheRead} />
          <TokenRow label="Cache Write" value={tokenBreakdown.cacheWrite} />
          {tokenBreakdown.reasoning > 0 && (
            <TokenRow label="Reasoning" value={tokenBreakdown.reasoning} />
          )}
        </div>

        {/* Divider */}
        <div className="border-t my-2" style={{ borderColor: "var(--color-border-muted)" }} />

        {/* Total tokens */}
        <div className="flex justify-between items-center">
          <span className="text-sm font-medium" style={{ color: "var(--color-fg-muted)" }}>
            Total
          </span>
          <span className="font-semibold" style={{ color: "var(--color-fg-default)" }}>
            {formatTokenCount(totals.tokens)} tokens
          </span>
        </div>

        {/* Messages count */}
        <div className="flex justify-between items-center mt-1">
          <span className="text-sm" style={{ color: "var(--color-fg-muted)" }}>
            Messages
          </span>
          <span className="text-sm" style={{ color: "var(--color-fg-default)" }}>
            {totals.messages.toLocaleString()}
          </span>
        </div>
      </div>
    </div>
  );
}

interface TokenRowProps {
  label: string;
  value: number;
}

function TokenRow({ label, value }: TokenRowProps) {
  if (value === 0) return null;

  return (
    <div className="flex justify-between items-center">
      <span style={{ color: "var(--color-fg-muted)" }}>{label}</span>
      <span className="font-mono" style={{ color: "var(--color-fg-default)" }}>
        {formatTokenCount(value)}
      </span>
    </div>
  );
}
