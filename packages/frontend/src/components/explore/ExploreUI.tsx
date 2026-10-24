"use client";

/**
 * Shared presentation pieces for the Humans / Models / Agents surfaces.
 * Thin 1px rules, tabular numbers, one accent. Page titles use the display
 * face when the v2 tokens define --font-display, and fall back to the UI font.
 */
import Link from "next/link";
import styled from "styled-components";
import { SegmentButton, SegmentedGroup } from "@/components/leaderboard/RankingUI";
import { agentIconUrl } from "@/lib/agentIcons";
import { SOURCE_LOGOS } from "@/lib/constants";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import type { AgentRef, Coverage } from "@/lib/explore";
import { clampShare, describeCoverage, formatShare, type UsersChange } from "./format";

export { ModelMark } from "./ModelMark";

export const DISPLAY_FONT = "var(--font-display, inherit) !important";

export const PageHeader = styled.header`
  display: grid;
  gap: 8px;
  margin-bottom: 24px;
`;

export const PageTitle = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-family: ${DISPLAY_FONT};
  font-size: 2rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
  text-wrap: balance;

  @media (max-width: 640px) {
    font-size: 1.75rem;
  }
`;

export const PageLead = styled.p`
  max-width: 68ch;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
  line-height: 1.55;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

export const SectionTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

export const SectionNote = styled.p`
  margin: 4px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

export const Section = styled.section`
  display: grid;
  align-content: start;
  gap: 12px;
  min-width: 0;
  padding-top: 20px;
  border-top: 1px solid var(--service-border);
`;

export const Muted = styled.span`
  color: var(--service-text-muted);
`;

export const Num = styled.span`
  font-variant-numeric: tabular-nums;
`;

export const ControlBar = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px 20px;
  margin-bottom: 16px;
`;

const ControlGroup = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
`;

const ControlLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

/** Segment that meets the 44px touch target on phones and coarse pointers. */
export const TouchSegment = styled(SegmentButton)`
  @media (max-width: 640px), (pointer: coarse) {
    min-height: 44px;
    padding: 0 14px;
  }
`;

export interface LinkOption {
  href: string;
  label: string;
  active: boolean;
}

/**
 * Link-based segmented navigation. Each option is a real URL, so state lives
 * in the address bar, works without JavaScript, and survives back/forward.
 */
export function LinkSegments({ label, visibleLabel, options }: { label: string; visibleLabel?: string; options: LinkOption[] }) {
  return (
    <ControlGroup>
      {visibleLabel ? <ControlLabel aria-hidden="true">{visibleLabel}</ControlLabel> : null}
      <SegmentedGroup as="nav" aria-label={label}>
        {options.map((option) => (
          <TouchSegment
            key={option.href + option.label}
            as={Link}
            href={option.href}
            scroll={false}
            $active={option.active}
            aria-current={option.active ? "page" : undefined}
          >
            {option.label}
          </TouchSegment>
        ))}
      </SegmentedGroup>
    </ControlGroup>
  );
}

export function ButtonSegments<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: ReadonlyArray<{ value: T; label: string; count?: number }>;
  onChange: (value: T) => void;
}) {
  return (
    <SegmentedGroup role="group" aria-label={label}>
      {options.map((option) => (
        <TouchSegment
          key={option.value}
          type="button"
          $active={value === option.value}
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
          {option.count !== undefined ? <CountTag>{option.count}</CountTag> : null}
        </TouchSegment>
      ))}
    </SegmentedGroup>
  );
}

const CountTag = styled.span`
  margin-left: 6px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
`;

const CoverageText = styled.p`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px 0;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
  font-variant-numeric: tabular-nums;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const CoverageItem = styled.span`
  &:not(:last-child)::after {
    content: "·";
    margin: 0 8px;
    color: var(--service-border-strong);
  }
`;

export const InlineLink = styled(Link)`
  color: var(--service-accent-hover);
  text-decoration: none;

  &:hover {
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

/**
 * One line that says what a table covers: dates, accounts, how much was
 * identified, and when the newest data arrived.
 */
export function CoverageLine({ coverage, className }: { coverage: Coverage; className?: string }) {
  const facts = describeCoverage(coverage);
  return (
    <CoverageText className={className}>
      <CoverageItem>{facts.range}</CoverageItem>
      <CoverageItem>{facts.accounts}</CoverageItem>
      <CoverageItem>{facts.identified}</CoverageItem>
      {facts.updated ? <CoverageItem>Data through {facts.updated}</CoverageItem> : null}
      <CoverageItem>
        <InlineLink href="/methodology">How we count</InlineLink>
      </CoverageItem>
    </CoverageText>
  );
}

export const NewBadge = styled.span`
  display: inline-flex;
  min-height: 18px;
  align-items: center;
  padding: 0 6px;
  border: 1px solid rgba(87, 165, 255, 0.45);
  border-radius: 999px;
  color: var(--service-accent-hover);
  font-size: 0.6875rem;
  font-weight: 600;
  letter-spacing: 0.02em;
  text-transform: uppercase;
  white-space: nowrap;
`;

const ChangeWrap = styled(InstantTooltip)<{ $kind: UsersChange["kind"] }>`
  display: inline-flex;
  align-items: center;
  gap: 4px;
  /* One accent: gains use the accent, losses and no change stay muted. The
     sign and the screen-reader sentence carry the meaning, not the color. */
  color: ${({ $kind }) => ($kind === "up" || $kind === "new" ? "var(--service-accent-hover)" : "var(--service-text-muted)")};
  font-size: 0.75rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const ChangePercent = styled.span`
  color: var(--service-text-muted);
`;

/** "+42 (+9%)" or "NEW +142". Empty for all-time views. */
export function ChangeText({ change }: { change: UsersChange }) {
  if (change.kind === "none") return null;
  return (
    <ChangeWrap $kind={change.kind} content={change.description} describe={false}>
      {change.kind === "new" ? <NewBadge>New</NewBadge> : null}
      <span aria-hidden="true">{change.text}</span>
      {change.percentText ? <ChangePercent aria-hidden="true">({change.percentText})</ChangePercent> : null}
      <VisuallyHidden>{change.description}</VisuallyHidden>
    </ChangeWrap>
  );
}

export const VisuallyHidden = styled.span`
  position: absolute !important;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
`;

const MeterRow = styled.span`
  display: inline-flex;
  width: 100%;
  min-width: 0;
  align-items: center;
  gap: 10px;
`;

const MeterTrack = styled.span`
  position: relative;
  display: block;
  flex: 1 1 auto;
  min-width: 48px;
  height: 6px;
  overflow: hidden;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.08);
`;

const MeterFill = styled.span`
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: 999px;
  background: var(--service-accent);
`;

const MeterValue = styled.span`
  min-width: 3.25em;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
  text-align: right;
`;

/**
 * Horizontal share bar. The full track is 100% (for adoption: every account
 * active in the period), so bar lengths compare honestly across rows.
 */
export function ShareMeter({ share, label }: { share: number; label?: string }) {
  const value = clampShare(share);
  return (
    <MeterRow>
      <MeterTrack aria-hidden="true">
        <MeterFill style={{ width: `${(value * 100).toFixed(2)}%` }} />
      </MeterTrack>
      <MeterValue>
        {formatShare(value)}
        {label ? <VisuallyHidden> {label}</VisuallyHidden> : null}
      </MeterValue>
    </MeterRow>
  );
}

const MarkImage = styled.img<{ $size: number }>`
  width: ${({ $size }) => $size}px;
  height: ${({ $size }) => $size}px;
  flex: 0 0 auto;
  border-radius: 4px;
  object-fit: cover;
  outline: 1px solid var(--service-border);
  outline-offset: -1px;
`;

const MarkMonogram = styled.span<{ $size: number }>`
  display: inline-grid;
  width: ${({ $size }) => $size}px;
  height: ${({ $size }) => $size}px;
  flex: 0 0 auto;
  place-items: center;
  border: 1px solid var(--service-border-strong);
  border-radius: 4px;
  color: var(--service-text-muted);
  font-size: ${({ $size }) => Math.max(9, Math.round($size * 0.45))}px;
  font-weight: 600;
`;

export function agentLogoUrl(agent: Pick<AgentRef, "key" | "logoUrl">): string | null {
  if (agent.logoUrl) return agent.logoUrl;
  const key = agent.key.toLowerCase();
  const icon = agentIconUrl(key);
  if (icon) return icon;
  return Object.prototype.hasOwnProperty.call(SOURCE_LOGOS, key)
    ? SOURCE_LOGOS[key as keyof typeof SOURCE_LOGOS]
    : null;
}

/** Decorative agent logo; the agent name is always rendered next to it. */
export function AgentMark({ agent, size = 18 }: { agent: Pick<AgentRef, "key" | "name" | "logoUrl">; size?: number }) {
  const src = agentLogoUrl(agent);
  if (src) {
    return <MarkImage src={src} alt="" width={size} height={size} $size={size} loading="lazy" />;
  }
  return (
    <MarkMonogram $size={size} aria-hidden="true">
      {agent.name.trim().charAt(0).toUpperCase() || "?"}
    </MarkMonogram>
  );
}

export const EmptyNote = styled.div`
  padding: 32px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.9375rem;
  text-align: center;
`;
