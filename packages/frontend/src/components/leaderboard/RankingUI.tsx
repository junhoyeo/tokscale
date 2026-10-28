"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import Link from "next/link";
import styled, { css } from "styled-components";
import { DitherAvatar } from "dither-kit/DitherAvatar";
import { DitherButton } from "dither-kit/DitherButton";
import { BAYER4, fillOf, type PixelColor } from "dither-kit/pixel";
import { rgb, type Rgb } from "dither-kit/palette";

/* Shared sizing for the app's actions: one control height per context
 * (32px for tab-like rows, 36px for actions), 44px where a thumb needs it. */
export const controlHeight = css`
  min-height: 36px;

  /* The thumb needs the height, not a bigger label. */
  @media (max-width: 640px) {
    min-height: 44px;
  }
`;


export const CompactBadge = styled.span`
  display: inline-flex;
  min-height: 22px;
  flex: none;
  align-items: center;
  padding: 0 8px;
  border-radius: var(--radius, 2px);
  background: var(--service-surface-muted);
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
  white-space: nowrap;
`;

export const MetricStrip = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  margin: 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);

  @media (min-width: 720px) {
    display: flex;
    max-width: max-content;
  }
`;

export const MetricItem = styled.div`
  min-width: 0;
  overflow: hidden;
  padding: 12px 16px 12px 0;

  &:nth-child(2n) {
    padding-right: 0;
    padding-left: 16px;
    border-left: 1px solid var(--service-border);
  }

  &:nth-child(n + 3) {
    border-top: 1px solid var(--service-border);
  }

  @media (min-width: 720px) {
    min-width: 152px;
    padding: 12px 20px;
    border-top: 0 !important;

    &:first-child {
      padding-left: 0;
    }

    &:last-child {
      padding-right: 0;
    }

    &:not(:first-child) {
      border-left: 1px solid var(--service-border);
    }
  }
`;

export const MetricLabel = styled.dt`
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 400;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

/* Every figure is white ink on tabular figures; colour is reserved for data
 * marks (the share bar), not the numbers themselves. `$accent` is accepted
 * for API compatibility and no longer changes the colour. */
export const MetricValue = styled.dd<{ $accent?: boolean }>`
  min-width: 0;
  margin: 4px 0 0;
  overflow: hidden;
  color: var(--service-text);
  font-size: 1.25rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 360px) {
    font-size: 1.125rem;
  }
`;

/* A small dithered square mark: the site's cell motif, denser the higher the
 * rank (first place nearly solid, third place sparse). Pure inline SVG,
 * SSR-stable, painted with the kit's Bayer matrix in the data hue. */
export function RankMark({
  level,
  color = "blue",
  cell = 2,
  title,
}: {
  level: 1 | 2 | 3;
  color?: PixelColor;
  cell?: number;
  title?: string;
}) {
  const n = 5;
  const density = level === 1 ? 1 : level === 2 ? 0.78 : 0.58;
  const fill = fillOf(color) as Rgb;
  const rects: React.ReactElement[] = [];
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      if (density <= BAYER4[y & 3][x & 3]) continue;
      rects.push(
        <rect
          key={`${x}-${y}`}
          x={x * cell}
          y={y * cell}
          width={cell}
          height={cell}
        />,
      );
    }
  }
  const size = n * cell;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      fill={rgb(fill)}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      style={{ imageRendering: "pixelated", flex: "0 0 auto" }}
    >
      {rects}
    </svg>
  );
}

/* A thin dithered bar whose length is the true share of the total, painted
 * with the kit's own Bayer engine (dither-kit/pixel + palette) at the kit's
 * 2px cell, with the same density ramp the kit's progress fill uses. The
 * track is empty ground: no fill, no outline. */
export function ShareBar({
  value,
  color = "blue",
  label,
}: {
  value: number;
  color?: PixelColor;
  label: string;
}) {
  const ratio = Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;

  return (
    <ShareBarTrack
      role="img"
      aria-label={label}
      style={{ "--share": `${(ratio * 100).toFixed(4)}%` } as React.CSSProperties}
    >
      {ratio > 0 && <ShareBarFill color={color} />}
    </ShareBarTrack>
  );
}

/* One track on every row: the fill grows from the track's left edge, so a
 * reader compares lengths down the column, not the fills' right ends. */
const ShareBarTrack = styled.span`
  display: flex;
  width: 100%;
  height: 8px;
`;

const ShareBarCanvas = styled.canvas<{ $color: PixelColor }>`
  display: block;
  width: var(--share);
  min-width: 3px;
  height: 100%;
  image-rendering: pixelated;
`;

/* The canvas paints a 2px-cell dither ramp across its own box; width comes
 * from CSS (the share), height from the track. Re-painted on resize. */
function ShareBarFill({ color }: { color: PixelColor }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d", { willReadFrequently: true });
    if (!canvas || !ctx) return;

    const cell = 2;
    const fill = fillOf(color) as Rgb;

    const paint = () => {
      const box = canvas.getBoundingClientRect();
      const cols = Math.round(box.width / cell);
      const rows = Math.max(1, Math.round(box.height / cell));
      if (cols < 1 || rows < 1) return;
      canvas.width = cols;
      canvas.height = rows;
      ctx.clearRect(0, 0, cols, rows);
      for (let y = 0; y < rows; y++) {
        for (let x = 0; x < cols; x++) {
          const density = 0.35 + 0.65 * ((x + 0.5) / cols);
          const lit = density > BAYER4[y & 3][x & 3];
          const k = 0.5 + density * 0.5;
          ctx.fillStyle = rgb(fill, 1, lit ? k : 0.12);
          ctx.fillRect(x, y, 1, 1);
        }
      }
    };

    const raf = requestAnimationFrame(paint);
    const ro =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(paint)
        : null;
    ro?.observe(canvas);
    return () => {
      cancelAnimationFrame(raf);
      ro?.disconnect();
    };
  }, [color]);

  return <ShareBarCanvas ref={canvasRef} $color={color} aria-hidden="true" />;
}

/* Thirty dithered columns, one per day, oldest left: the height of a column is
 * that day's tokens against the row's own busiest day, so the shape of a
 * month reads at a glance; the figure beside it carries the magnitude. Zero
 * days keep a one-cell baseline so the month's length is always visible.
 * Painted with the kit's Bayer matrix at a 2px cell, like the share bar. */
export function UsageBars({
  values,
  color = "blue",
  label,
}: {
  values: number[];
  color?: PixelColor;
  label: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const key = values.join(",");

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d", { willReadFrequently: true });
    if (!canvas || !ctx) return;

    const series = key ? key.split(",").map(Number) : [];
    const max = Math.max(0, ...series);
    const fill = fillOf(color) as Rgb;
    const cell = 2;

    const paint = () => {
      const box = canvas.getBoundingClientRect();
      const cols = Math.round(box.width / cell);
      const rows = Math.max(2, Math.round(box.height / cell));
      if (cols < series.length || series.length === 0) return;
      canvas.width = cols;
      canvas.height = rows;
      ctx.clearRect(0, 0, cols, rows);
      // Each day gets an equal slot; one empty cell separates neighbours.
      const slot = cols / series.length;
      const barCols = Math.max(1, Math.floor(slot) - 1);
      series.forEach((value, day) => {
        const x0 = Math.round(day * slot);
        const ratio = max > 0 ? value / max : 0;
        const height = value > 0 ? Math.max(2, Math.round(ratio * rows)) : 1;
        for (let y = rows - height; y < rows; y++) {
          for (let x = x0; x < x0 + barCols; x++) {
            // Denser toward the base; a zero day is a faint baseline cell.
            const density = value > 0 ? 0.45 + 0.55 * ((y + 1) / rows) : 0;
            const lit = density > BAYER4[y & 3][x & 3];
            ctx.fillStyle = rgb(fill, 1, value > 0 ? (lit ? 1 : 0.35) : 0.22);
            ctx.fillRect(x, y, 1, 1);
          }
        }
      });
    };

    const raf = requestAnimationFrame(paint);
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(paint) : null;
    ro?.observe(canvas);
    return () => {
      cancelAnimationFrame(raf);
      ro?.disconnect();
    };
  }, [color, key]);

  return <UsageBarsCanvas ref={canvasRef} role="img" aria-label={label} />;
}

const UsageBarsCanvas = styled.canvas`
  display: block;
  width: 100%;
  height: 24px;
  image-rendering: pixelated;
`;

export const GroupMarkFrame = styled.span<{ $size: "compact" | "detail" }>`
  display: inline-grid;
  width: ${({ $size }) => $size === "detail" ? "64px" : "42px"};
  height: ${({ $size }) => $size === "detail" ? "64px" : "42px"};
  flex: 0 0 auto;
  place-items: stretch;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--service-surface-muted);
`;

const MarkImage = styled.img`
  width: 100%;
  height: 100%;
  object-fit: cover;
`;

const AvatarFrame = styled.span<{ $size: number }>`
  display: grid;
  width: ${({ $size }) => `${$size}px`};
  height: ${({ $size }) => `${$size}px`};
  flex: 0 0 auto;
  place-items: stretch;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--service-surface-muted);
`;

const AvatarImage = styled.img`
  width: 100%;
  height: 100%;
  object-fit: cover;
`;

/* Real GitHub avatars stay real pictures, square with the site's 2px corner.
 * A missing or broken picture becomes the same dithered mark as missing
 * group art (deterministic from the name), never a broken-image icon. */
export function PersonAvatar({
  src,
  name,
  size = 36,
}: {
  src?: string | null;
  name: string;
  size?: number;
}) {
  // Remember which source failed rather than a boolean, so a new source is
  // automatically un-failed with no effect to reset it.
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const failed = !!src && failedSrc === src;

  // A picture that 404s before hydration never fires `error` on the hydrated
  // node, so check the committed element itself.
  const measure = useCallback(
    (image: HTMLImageElement | null) => {
      if (image && image.complete && image.naturalWidth === 0) setFailedSrc(src ?? null);
    },
    [src],
  );

  return (
    <AvatarFrame $size={size} aria-hidden="true">
      {src && !failed ? (
        <AvatarImage
          ref={measure}
          src={src}
          alt=""
          loading="lazy"
          onError={() => setFailedSrc(src)}
        />
      ) : (
        <DitherAvatar
          name={name || "anonymous"}
          color="blue"
          size={size}
          grid={6}
          cellPx={Math.max(2, Math.round(size / 6))}
          density={-0.12}
          animate={false}
          bloom="off"
        />
      )}
    </AvatarFrame>
  );
}

/* Missing group art becomes a dithered avatar generated from the group's
 * name (deterministic), always in the one data hue. */
export function GroupMark({
  name,
  avatarUrl,
  size = "compact",
}: {
  name: string;
  avatarUrl?: string | null;
  size?: "compact" | "detail";
}) {
  // As in PersonAvatar: the failing source is the state, so a new one clears
  // itself, and the check runs from the ref callback, not an effect.
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const failed = !!avatarUrl && failedSrc === avatarUrl;
  const px = size === "detail" ? 64 : 42;

  const measure = useCallback(
    (image: HTMLImageElement | null) => {
      if (image && image.complete && image.naturalWidth === 0) setFailedSrc(avatarUrl ?? null);
    },
    [avatarUrl],
  );

  return (
    <GroupMarkFrame $size={size} aria-hidden="true">
      {avatarUrl && !failed ? (
        <MarkImage ref={measure} src={avatarUrl} alt="" onError={() => setFailedSrc(avatarUrl)} />
      ) : (
        <DitherAvatar
          name={name}
          color="blue"
          size={px}
          grid={8}
          cellPx={Math.max(2, Math.round(px / 8))}
          density={-0.1}
          animate={false}
          bloom="off"
        />
      )}
    </GroupMarkFrame>
  );
}

export const MobileRankingList = styled.ol`
  margin: 0;
  padding: 0;

  @media (min-width: 720px) {
    display: none;
  }
`;

const MobileRankingItem = styled.li`
  list-style: none;

  &:not(:last-child) {
    border-bottom: 1px solid var(--service-border);
  }
`;

/* Phone rows are two-line items: identity and the key figure on line 1,
 * the secondary fields in grey on line 2, the whole row one link. */
const MobileRankingLink = styled(Link)<{ $current: boolean }>`
  display: grid;
  grid-template-columns: 44px 34px minmax(0, 1fr) auto;
  grid-template-rows: auto auto;
  column-gap: 10px;
  align-items: center;
  min-height: 76px;
  padding: 10px 0;
  background: ${({ $current }) => $current ? "var(--service-surface-muted)" : "transparent"};
  color: inherit;
  text-decoration: none;

  /* The row is one link: the same hover as other row links (globals.css). */
  &:hover {
    background: ${({ $current }) => $current ? "var(--service-surface-muted)" : "var(--v2-row-hover)"};
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }
`;

const MobileRank = styled.span`
  display: inline-flex;
  grid-row: 1 / 3;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
`;

const MobileAvatar = styled.span`
  display: grid;
  grid-row: 1 / 3;
  align-self: center;
`;

const MobileIdentity = styled.div`
  min-width: 0;
  padding-top: 2px;
`;

const MobileName = styled.p`
  margin: 0;
  overflow: hidden;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const MobileUsername = styled.p`
  margin: 0;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const MobilePrimary = styled.div`
  align-self: center;
  text-align: right;
`;

const MobilePrimaryValue = styled.p`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const MobilePrimaryLabel = styled.p`
  margin: 1px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  text-align: right;
`;

const MobileMeta = styled.p`
  grid-column: 2 / 5;
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 10px;
  margin: 0;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const MobileShare = styled.span`
  flex: 0 1 64px;
  min-width: 24px;
`;

const MobileRecent = styled.span`
  flex: 0 0 96px;

  canvas {
    height: 18px;
  }
`;

export function MobileRankingRow({
  rank,
  href,
  avatarUrl,
  username,
  displayName,
  primaryLabel,
  primaryValue,
  meta,
  isCurrentUser,
  share,
  recent,
}: {
  rank: number;
  href: string;
  avatarUrl: string | null;
  username: string;
  displayName: string;
  primaryLabel: string;
  primaryValue: string;
  meta: string;
  isCurrentUser: boolean;
  /** 0–1 share of the period total; renders a thin dithered bar on line 2. */
  share?: number | null;
  /** Daily tokens over the past 30 days; replaces the share bar when given. */
  recent?: number[] | null;
}) {
  return (
    <MobileRankingItem>
      <MobileRankingLink
        href={href}
        $current={isCurrentUser}
        aria-current={isCurrentUser ? "true" : undefined}
      >
        <MobileRank>
          {rank <= 3 && <RankMark level={rank as 1 | 2 | 3} cell={2} />}
          {rank}
        </MobileRank>
        <MobileAvatar>
          <PersonAvatar
            src={avatarUrl || `https://github.com/${username}.png`}
            name={displayName || username}
            size={34}
          />
        </MobileAvatar>
        <MobileIdentity>
          <MobileName>{displayName}</MobileName>
          <MobileUsername>@{username}</MobileUsername>
        </MobileIdentity>
        <MobilePrimary>
          <MobilePrimaryValue>{primaryValue}</MobilePrimaryValue>
          <MobilePrimaryLabel>{primaryLabel}</MobilePrimaryLabel>
        </MobilePrimary>
        <MobileMeta>
          {recent ? (
            <MobileRecent>
              <UsageBars values={recent} label={`${displayName}: daily tokens over the past 30 days`} />
            </MobileRecent>
          ) : share != null ? (
            <MobileShare>
              <ShareBar value={share} label={`${displayName}: share of tokens`} />
            </MobileShare>
          ) : null}
          {meta}
        </MobileMeta>
      </MobileRankingLink>
    </MobileRankingItem>
  );
}

/* Selection is a line, not a fill: selected = ink label with a 2px accent
 * line exactly as wide as the label, on the row's bottom edge; unselected =
 * muted label. `--segment-pad-x` widens the hit area without moving the
 * line off the label. */
export const SegmentedGroup = styled.div`
  display: inline-flex;
  width: max-content;
  max-width: 100%;
  box-sizing: border-box;
  align-items: center;
  gap: 16px;
  overflow-x: auto;
  scrollbar-width: none;
  overscroll-behavior-inline: contain;

  &::-webkit-scrollbar {
    display: none;
  }

  /* A tab row scrolls sideways inside its own row on a phone; it never
   * wraps into a second line and never moves the page. */
  @media (max-width: 640px) {
    flex-wrap: nowrap;
    overflow-x: auto;
    gap: 8px;
    padding-bottom: 2px;
  }
`;

export const SegmentButton = styled.button<{ $active: boolean }>`
  --segment-pad-x: 0px;

  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 32px;
  padding: 0 var(--segment-pad-x);
  border: 0;
  background: transparent;
  color: ${({ $active }) => $active ? "var(--service-text)" : "var(--service-text-muted)"};
  font-size: 0.875rem;
  font-weight: 500;
  text-decoration: none;
  white-space: nowrap;

  &::after {
    position: absolute;
    right: var(--segment-pad-x);
    bottom: 0;
    left: var(--segment-pad-x);
    height: 2px;
    background: var(--service-accent);
    content: "";
    opacity: ${({ $active }) => $active ? 1 : 0};
  }

  &:hover {
    color: var(--service-text);
  }

  &:hover::after {
    opacity: ${({ $active }) => $active ? 1 : 0.4};
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }

  @media (max-width: 640px) {
    min-height: 44px;
  }
`;

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
}

export function SegmentedControl<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: readonly SegmentedOption<T>[];
  onChange: (value: T) => void;
}) {
  return (
    <SegmentedGroup role="group" aria-label={label}>
      {options.map((option) => (
        <SegmentButton
          key={option.value}
          type="button"
          $active={value === option.value}
          aria-pressed={value === option.value}
          disabled={option.disabled}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </SegmentButton>
      ))}
    </SegmentedGroup>
  );
}

/* The primary action is a solid fill, not a dither pattern: a label sits on
 * top of it, and dotted/hatched let the black page through under white type.
 * #0066e6 is the accent's filled step (globals.css: 5.2:1 with white, where
 * the lighter accents fall to 4.3:1). */
export const PRIMARY_FILL = "#0066e6";

const primaryStyles = css`
  min-height: 36px;
  padding: 0 16px;
  font-size: 0.875rem;
  font-weight: 500;
  color: #ffffff;

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

/* The kit's button as the screen's primary, one per screen at most. The
 * variant and fill are fixed here so no call site can put a label on a
 * see-through fill. */
export const PrimaryButton = styled(DitherButton).attrs({
  color: PRIMARY_FILL,
  variant: "solid" as const,
})`
  ${primaryStyles}
`;

/* The one primary action per screen, rendered as a Next link. Layout props
 * (className, style) pass through; the variant and fill do not, so a call
 * site can place the button but cannot reopen the see-through fill. */
export function PrimaryActionLink({
  href,
  children,
  className,
  style,
}: {
  href: string;
  children: React.ReactNode;
  className?: string;
  style?: React.CSSProperties;
}) {
  return (
    <PrimaryButton
      className={className}
      style={style}
      nativeButton={false}
      role="link"
      render={<Link href={href} />}
    >
      {children}
    </PrimaryButton>
  );
}

const textActionStyles = css`
  display: inline-flex;
  ${controlHeight}
  align-items: center;
  justify-content: center;
  padding: 0 2px;
  border: 0;
  background: transparent;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
  text-decoration: none;
  white-space: nowrap;

  &:hover {
    text-decoration: underline;
    text-underline-offset: 4px;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

/* Secondary actions are plain text in ink: underline on hover/focus. */
export const SecondaryActionLink = styled(Link)`
  ${textActionStyles}
`;

export const SecondaryButton = styled.button`
  ${textActionStyles}

  &:disabled {
    cursor: not-allowed;
    opacity: 0.5;
    text-decoration: none;
  }
`;
