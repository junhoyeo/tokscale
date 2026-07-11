"use client";

import { useMemo, useState, useSyncExternalStore } from "react";
import Image from "next/image";
import styled, { css } from "styled-components";
import { toast } from "react-toastify";
import { formatCurrency, formatNumber } from "@/lib/utils";
import { ProfileEmbedDialog } from "./ProfileEmbedDialog";
import type { ProfileStatsData, ProfileUser } from "./types";

export interface ProfileOverviewProps {
  user: ProfileUser;
  stats: ProfileStatsData;
  lastUpdated?: string | null;
  period?: "all" | "month" | "week";
  className?: string;
}

const OverviewPanel = styled.section`
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
  color: var(--service-text);
  container-type: inline-size;
`;

const OverviewHeader = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.875rem 1.25rem;
  padding: 0.875rem 1rem;

  @media (min-width: 640px) {
    padding: 1rem 1.125rem;
  }
`;

const Identity = styled.div`
  display: flex;
  min-width: 0;
  flex: 1 1 19rem;
  align-items: center;
  gap: 1rem;
`;

const Avatar = styled.div`
  position: relative;
  width: 72px;
  height: 72px;
  overflow: hidden;
  flex: 0 0 auto;
  border: 1px solid var(--service-border-strong);
  border-radius: 12px;
  background: var(--service-surface-muted);

  @media (min-width: 640px) {
    width: 80px;
    height: 80px;
  }
`;

const AvatarImage = styled(Image)`
  object-fit: cover;
`;

const IdentityCopy = styled.div`
  min-width: 0;
`;

const DisplayName = styled.h1`
  overflow: hidden;
  margin: 0;
  color: var(--service-text);
  font-size: clamp(1.125rem, 4vw, 1.375rem);
  font-weight: 600;
  line-height: 1.2;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const Handle = styled.p`
  overflow: hidden;
  margin: 0.2rem 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.25;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const Metadata = styled.ul`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.25rem 0.5rem;
  margin-top: 0.4rem;
  margin-bottom: 0;
  padding: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.3;
  list-style: none;
`;

const MetadataItem = styled.li`
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;

  &:not(:first-child)::before {
    width: 2px;
    height: 2px;
    border-radius: 50%;
    background: var(--service-border-strong);
    content: "";
  }
`;

const RankItem = styled.li`
  display: inline-flex;
  align-items: baseline;
  gap: 0.25rem;
  padding: 0.1875rem 0.4375rem;
  border: 1px solid color-mix(in srgb, var(--service-accent) 42%, transparent);
  border-radius: 0.375rem;
  background: var(--service-accent-soft);
  color: var(--service-text);
  font-size: 0.75rem;
  font-weight: 600;
  line-height: 1.2;

  strong {
    color: var(--service-accent);
    font-weight: 700;
  }
`;

const Actions = styled.div`
  display: flex;
  flex: 0 0 auto;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.375rem;

  @media (max-width: 479px) {
    width: 100%;
  }
`;

const actionStyles = css`
  display: inline-flex;
  min-height: 34px;
  align-items: center;
  justify-content: center;
  gap: 0.375rem;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  padding: 0.4rem 0.625rem;
  color: var(--service-text);
  font: inherit;
  font-size: 0.8125rem;
  font-weight: 550;
  line-height: 1;
  text-decoration: none;
  cursor: pointer;
  transition:
    border-color 140ms ease,
    background-color 140ms ease,
    color 140ms ease;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (hover: hover) {
    &:hover {
      border-color: var(--service-border-strong);
    }
  }

  @media (pointer: coarse) {
    min-height: 44px;
    padding-top: 0.65rem;
    padding-bottom: 0.65rem;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const PrimaryAction = styled.button`
  ${actionStyles}
  border-color: var(--service-accent);
  background: var(--service-accent);
  color: var(--service-accent-foreground);

  @media (hover: hover) {
    &:hover {
      border-color: var(--service-accent-hover);
      background: var(--service-accent-hover);
    }
  }
`;

const SecondaryAction = styled.button`
  ${actionStyles}
  background: var(--service-surface-muted);

  @media (hover: hover) {
    &:hover {
      background: var(--service-accent-soft);
    }
  }
`;

const GhostAction = styled.a`
  ${actionStyles}
  background: transparent;

  @media (hover: hover) {
    &:hover {
      background: var(--service-surface-muted);
    }
  }
`;

const Metrics = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  margin: 0;
  border-top: 1px solid var(--service-border);
  text-align: left;

  @container (min-width: 40rem) {
    grid-template-columns: repeat(4, 9.25rem);
    justify-content: start;
  }
`;

const Metric = styled.div`
  display: flex;
  min-width: 0;
  flex-direction: column;
  align-items: flex-start;
  padding: 0.6875rem 1rem;
  text-align: left;

  &:nth-child(even) {
    border-left: 1px solid var(--service-border);
  }

  &:nth-child(n + 3) {
    border-top: 1px solid var(--service-border);
  }

  @container (min-width: 40rem) {
    padding-right: 1.25rem;
    padding-left: 1.25rem;

    &:not(:first-child) {
      border-left: 1px solid var(--service-border);
    }

    &:nth-child(n + 3) {
      border-top: 0;
    }
  }
`;

const MetricLabel = styled.dt`
  width: 100%;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  line-height: 1.2;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
`;

const MetricValue = styled.dd`
  width: 100%;
  overflow: hidden;
  margin: 0.3rem 0 0;
  color: var(--service-text);
  font-size: clamp(1.05rem, 4vw, 1.35rem);
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  line-height: 1.1;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
`;

const subscribeNoop = () => () => {};

/** Keep server and first-hydration output in UTC, then use the viewer's zone. */
export function formatLastUpdated(
  lastUpdated: string | null | undefined,
  isMounted: boolean,
): string | null {
  if (!lastUpdated) return null;
  const date = new Date(lastUpdated);
  return isMounted
    ? date.toLocaleString("en-US")
    : date.toLocaleString("en-US", { timeZone: "UTC" });
}

function formatJoined(createdAt: string | null | undefined): string | null {
  if (!createdAt) return null;
  const date = new Date(createdAt);
  if (Number.isNaN(date.getTime())) return null;

  return date.toLocaleDateString("en-US", {
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });
}

export function ProfileOverview({
  user,
  stats,
  lastUpdated,
  period = "all",
  className,
}: ProfileOverviewProps) {
  const [isEmbedDialogOpen, setIsEmbedDialogOpen] = useState(false);
  const isMounted = useSyncExternalStore(
    subscribeNoop,
    () => true,
    () => false,
  );
  const formattedLastUpdated = useMemo(
    () => formatLastUpdated(lastUpdated, isMounted),
    [isMounted, lastUpdated],
  );
  const joined = useMemo(() => formatJoined(user.createdAt), [user.createdAt]);
  const displayName = user.displayName || user.username;
  const avatarUrl = user.avatarUrl || `https://github.com/${user.username}.png`;

  const handleShareClick = async () => {
    try {
      await navigator.clipboard.writeText(window.location.href);
      toast.success("Link copied to clipboard!");
    } catch {
      toast.error("Failed to copy link");
    }
  };

  const headlineMetrics = [
    {
      label:
        period === "all"
          ? "All-time tokens"
          : period === "month"
            ? "30d tokens"
            : "7d tokens",
      value: formatNumber(stats.totalTokens),
      title: stats.totalTokens.toLocaleString("en-US"),
    },
    {
      label:
        period === "all"
          ? "All-time cost"
          : period === "month"
            ? "30d cost"
            : "7d cost",
      value: formatCurrency(stats.totalCost),
      title: stats.totalCost.toLocaleString("en-US", {
        style: "currency",
        currency: "USD",
      }),
    },
    {
      label:
        period === "all"
          ? "Active days (1y)"
          : period === "month"
            ? "Active days (30d)"
            : "Active days (7d)",
      value: stats.activeDays.toLocaleString("en-US"),
      title: stats.activeDays.toLocaleString("en-US"),
    },
    {
      label: "All submissions",
      value: (stats.submissionCount ?? 0).toLocaleString("en-US"),
      title: (stats.submissionCount ?? 0).toLocaleString("en-US"),
    },
  ];

  return (
    <OverviewPanel
      className={className}
      aria-labelledby="profile-overview-heading"
    >
      <OverviewHeader>
        <Identity>
          <Avatar>
            <AvatarImage
              src={avatarUrl}
              alt={`${displayName}'s avatar`}
              fill
              sizes="(min-width: 640px) 80px, 72px"
              priority
            />
          </Avatar>

          <IdentityCopy>
            <DisplayName id="profile-overview-heading">
              {displayName}
            </DisplayName>
            <Handle>@{user.username}</Handle>

            {(user.rank != null || joined || formattedLastUpdated) && (
              <Metadata aria-label="Profile details">
                {user.rank != null && (
                  <RankItem>
                    <span>Rank</span>
                    <strong>#{user.rank.toLocaleString("en-US")}</strong>
                  </RankItem>
                )}
                {joined && <MetadataItem>Joined {joined}</MetadataItem>}
                {formattedLastUpdated && (
                  <MetadataItem suppressHydrationWarning>
                    Updated {formattedLastUpdated}
                  </MetadataItem>
                )}
              </Metadata>
            )}
          </IdentityCopy>
        </Identity>

        <Actions aria-label="Profile actions">
          <PrimaryAction
            type="button"
            onClick={() => setIsEmbedDialogOpen(true)}
            aria-label={`Open GitHub README embed options for ${displayName}`}
          >
            Embed
          </PrimaryAction>
          <SecondaryAction
            type="button"
            onClick={handleShareClick}
            aria-label={`Share ${displayName}'s profile`}
          >
            <Image
              src="/icons/icon-share.svg"
              alt=""
              width={16}
              height={16}
              aria-hidden="true"
            />
            Share
          </SecondaryAction>
          <GhostAction
            href={`https://github.com/${user.username}`}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={`View ${user.username}'s GitHub profile (opens in new tab)`}
          >
            <Image
              src="/icons/icon-github.svg"
              alt=""
              width={16}
              height={16}
              aria-hidden="true"
            />
            GitHub
          </GhostAction>
        </Actions>
      </OverviewHeader>

      <Metrics aria-label="Profile summary">
        {headlineMetrics.map((metric) => (
          <Metric key={metric.label}>
            <MetricLabel>{metric.label}</MetricLabel>
            <MetricValue title={metric.title}>{metric.value}</MetricValue>
          </Metric>
        ))}
      </Metrics>

      <ProfileEmbedDialog
        open={isEmbedDialogOpen}
        username={user.username}
        displayName={user.displayName}
        onClose={() => setIsEmbedDialogOpen(false)}
      />
    </OverviewPanel>
  );
}
