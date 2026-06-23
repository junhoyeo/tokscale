"use client";

import type { SquircleBorderDef } from "../hooks";

interface SquircleBorderGradient {
  colors: [string, string];
  /** Y position (in element pixels) where the color transition begins */
  transitionY: number;
}
interface SquircleBorderProps {
  def: SquircleBorderDef | null;
  color?: string;
  gradient?: SquircleBorderGradient;
}

export function SquircleBorder({
  def,
  color = "#10233E",
  gradient,
}: SquircleBorderProps) {
  if (!def) return null;
  const {
    outerClipId, innerClipId, maskId,
    outerPath, innerPath,
    width, height, cornerRadius, borderWidth, bottomOnly,
  } = def;

  return (
    <svg
      style={{
        position: "absolute",
        inset: 0,
        width: "100%",
        height: "100%",
        pointerEvents: "none",
        zIndex: 10,
      }}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <defs>
        <clipPath id={outerClipId}>
          <path
            d={outerPath}
            transform={bottomOnly ? `translate(0, -${cornerRadius})` : undefined}
          />
        </clipPath>
        <clipPath id={innerClipId}>
          <path
            d={innerPath}
            transform={
              bottomOnly
                ? `translate(${borderWidth}, ${borderWidth - cornerRadius})`
                : `translate(${borderWidth}, ${borderWidth})`
            }
          />
        </clipPath>
        <mask id={maskId}>
          <rect
            width={width}
            height={height}
            fill="white"
            clipPath={`url(#${outerClipId})`}
          />
          <rect
            width={width - borderWidth * 2}
            height={height - borderWidth * 2}
            x={borderWidth}
            y={borderWidth}
            fill="black"
            clipPath={`url(#${innerClipId})`}
          />
        </mask>
        {gradient && gradient.transitionY > 0 && height > 0 && (
          <linearGradient
            id={`${outerClipId}-gradient`}
            x1="0"
            y1="0"
            x2="0"
            y2={String(height)}
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0" stopColor={gradient.colors[0]} />
            <stop
              offset={String(gradient.transitionY / height)}
              stopColor={gradient.colors[0]}
            />
            <stop offset="1" stopColor={gradient.colors[1]} />
          </linearGradient>
        )}
      </defs>
      <rect
        width={width}
        height={height}
        fill={
          gradient && gradient.transitionY > 0 && height > 0
            ? `url(#${outerClipId}-gradient)`
            : color
        }
        mask={`url(#${maskId})`}
      />
    </svg>
  );
}
