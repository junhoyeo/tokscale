"use client";

import Link from "next/link";
import styled from "styled-components";
import { getGroupMonogram } from "./presentation";

export const CompactBadge = styled.span`
  display: inline-flex;
  min-height: 22px;
  align-items: center;
  padding: 0 8px;
  border: 1px solid var(--service-border);
  border-radius: 999px;
  background: var(--service-surface-muted);
  color: var(--service-text-muted);
  font-size: 0.75rem;
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
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

export const MetricValue = styled.dd<{ $accent?: boolean }>`
  min-width: 0;
  margin: 4px 0 0;
  overflow: hidden;
  color: ${({ $accent }) => $accent ? "var(--service-accent-hover)" : "var(--service-text)"};
  font-size: 1.125rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 640px) {
    font-size: 1.25rem;
  }

  @media (max-width: 360px) {
    font-size: 1.125rem;
  }
`;

const Mark = styled.span<{ $size: "compact" | "detail" }>`
  display: inline-grid;
  width: ${({ $size }) => $size === "detail" ? "64px" : "42px"};
  height: ${({ $size }) => $size === "detail" ? "64px" : "42px"};
  flex: 0 0 auto;
  place-items: center;
  overflow: hidden;
  border: 1px solid var(--service-border-strong);
  border-radius: ${({ $size }) => $size === "detail" ? "12px" : "9px"};
  background: var(--service-surface-muted);
  color: var(--service-accent-hover);
  font-size: ${({ $size }) => $size === "detail" ? "1rem" : "0.75rem"};
  font-weight: 600;
  letter-spacing: 0.04em;
`;

const MarkImage = styled.img`
  width: 100%;
  height: 100%;
  object-fit: cover;
`;

export function GroupMark({
  name,
  avatarUrl,
  size = "compact",
}: {
  name: string;
  avatarUrl?: string | null;
  size?: "compact" | "detail";
}) {
  return (
    <Mark $size={size} aria-hidden="true">
      {avatarUrl ? <MarkImage src={avatarUrl} alt="" /> : getGroupMonogram(name)}
    </Mark>
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

const MobileRankingLink = styled(Link)<{ $current: boolean }>`
  display: grid;
  grid-template-columns: 34px 38px minmax(0, 1fr) auto;
  grid-template-rows: auto auto;
  gap: 3px 8px;
  align-items: center;
  min-height: 80px;
  padding: 10px 8px;
  background: ${({ $current }) => $current ? "var(--service-accent-soft)" : "transparent"};
  color: inherit;
  text-decoration: none;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }
`;

const MobileRank = styled.span`
  grid-row: 1 / 3;
  align-self: center;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;

  &[data-rank="1"] { color: #f4c95d; }
  &[data-rank="2"] { color: #c4ccda; }
  &[data-rank="3"] { color: #d99a68; }
`;

const MobileAvatar = styled.img`
  grid-row: 1 / 3;
  width: 38px;
  height: 38px;
  align-self: center;
  border-radius: 50%;
  object-fit: cover;
  outline: 1px solid var(--service-border);
  outline-offset: -1px;
`;

const MobileIdentity = styled.div`
  min-width: 0;
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
  color: var(--service-accent-hover);
  font-size: 0.9375rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const MobilePrimaryLabel = styled.p`
  margin: 1px 0 0;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  text-align: right;
`;

const MobileMeta = styled.p`
  grid-column: 3 / 5;
  margin: 0;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
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
}) {
  return (
    <MobileRankingItem>
      <MobileRankingLink
        href={href}
        $current={isCurrentUser}
        aria-current={isCurrentUser ? "true" : undefined}
      >
        <MobileRank data-rank={rank <= 3 ? rank : undefined}>#{rank}</MobileRank>
        <MobileAvatar
          src={avatarUrl || `https://github.com/${username}.png`}
          alt=""
        />
        <MobileIdentity>
          <MobileName>{displayName}</MobileName>
          <MobileUsername>@{username}</MobileUsername>
        </MobileIdentity>
        <MobilePrimary>
          <MobilePrimaryValue>{primaryValue}</MobilePrimaryValue>
          <MobilePrimaryLabel>{primaryLabel}</MobilePrimaryLabel>
        </MobilePrimary>
        <MobileMeta>{meta}</MobileMeta>
      </MobileRankingLink>
    </MobileRankingItem>
  );
}

export const SegmentedGroup = styled.div`
  display: inline-flex;
  width: max-content;
  max-width: 100%;
  box-sizing: border-box;
  align-items: center;
  gap: 2px;
  padding: 2px;
  overflow-x: auto;
  border: 1px solid var(--service-border);
  border-radius: 8px;
  background: var(--service-surface-muted);
  scrollbar-width: none;
  overscroll-behavior-inline: contain;

  &::-webkit-scrollbar {
    display: none;
  }
`;

export const SegmentButton = styled.button<{ $active: boolean }>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 30px;
  padding: 0 10px;
  border: 0;
  border-radius: 6px;
  background: ${({ $active }) => $active ? "var(--service-surface)" : "transparent"};
  color: ${({ $active }) => $active ? "var(--service-text)" : "var(--service-text-muted)"};
  font-size: 0.8125rem;
  font-weight: 500;
  text-decoration: none;
  white-space: nowrap;

  &:hover {
    color: var(--service-text);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }

  @media (max-width: 640px) {
    min-height: 40px;
    padding: 0 12px;
    font-size: 1rem;
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

const actionStyles = `
  display: inline-flex;
  min-height: 36px;
  align-items: center;
  justify-content: center;
  padding: 0 12px;
  border-radius: 8px;
  font-size: 0.875rem;
  font-weight: 600;
  text-decoration: none;
  white-space: nowrap;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

export const PrimaryActionLink = styled(Link)`
  ${actionStyles}
  border: 1px solid var(--service-accent);
  background: var(--service-accent);
  color: #fff;

  &:hover {
    border-color: var(--service-accent-hover);
    background: var(--service-accent-hover);
  }
`;

export const SecondaryActionLink = styled(Link)`
  ${actionStyles}
  border: 1px solid var(--service-border-strong);
  background: transparent;
  color: var(--service-text);

  &:hover {
    background: var(--service-surface-muted);
  }
`;

export const SecondaryButton = styled.button`
  ${actionStyles}
  border: 1px solid var(--service-border-strong);
  background: transparent;
  color: var(--service-text);

  &:hover:not(:disabled) {
    background: var(--service-surface-muted);
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
`;
