"use client";

import { useMemo, useState, useSyncExternalStore } from "react";
import Image from "next/image";
import styled from "styled-components";
import { DitherAvatar } from "dither-kit/DitherAvatar";
import { DitherButton } from "dither-kit/DitherButton";
import { toast } from "react-toastify";
import { describeRankingWindow } from "@/lib/reportingPeriods";
import { formatCurrency, formatNumber } from "@/lib/utils";
import { DitherSelectionNav } from "./ditherControls";
import { ProfileEmbedDialog } from "./ProfileEmbedDialog";
import type { ProfileStatsData, ProfileUser } from "./types";

import { PROFILE_PERIODS, PROFILE_PERIOD_LABELS, type ProfilePeriod } from "@/lib/reportingPeriods";

export interface ProfileOverviewProps {
  user: ProfileUser;
  stats: ProfileStatsData;
  lastUpdated?: string | null;
  period?: ProfilePeriod;
  /**
   * Zone for the two absolute instants rendered here — "Updated" and "Joined".
   * Defaults to the viewer's browser zone. Never applies to contribution
   * dates: those are calendar buckets the submitting machine already resolved.
   */
  timeZone?: string;
  className?: string;
}

export function getProfileRankLabel(
  period: NonNullable<ProfileOverviewProps["period"]>,
): string {
  if (period === "month") return "30d rank (UTC)";
  if (period === "week") return "7d rank (UTC)";
  if (period === "this-month" || period === "last-month") {
    return `${PROFILE_PERIOD_LABELS[period]} rank (UTC)`;
  }
  return "All-time rank";
}

const OverviewPanel = styled.section`
  color: var(--service-text);
  container-type: inline-size;
`;

const IdentityRow = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem 1.25rem;

  @container (max-width: 34rem) {
    flex-direction: column;
    align-items: stretch;
  }
`;

const Identity = styled.div`
  display: flex;
  min-width: 0;
  flex: 1 1 19rem;
  align-items: center;
  gap: 1rem;

  @container (max-width: 34rem) {
    flex: 0 0 auto;
  }
`;

const Avatar = styled.div`
  position: relative;
  width: 72px;
  height: 72px;
  overflow: hidden;
  flex: 0 0 auto;
  border: 1px solid var(--service-border-strong);
  border-radius: var(--radius);
  background: var(--service-surface-muted);

  @container (min-width: 40rem) {
    width: 80px;
    height: 80px;
  }
`;

const AvatarImage = styled(Image)`
  object-fit: cover;
`;

const AvatarFallback = styled(DitherAvatar)`
  position: absolute;
  inset: 0;
`;

const IdentityCopy = styled.div`
  min-width: 0;
`;

const DisplayName = styled.h1`
  overflow: hidden;
  margin: 0;
  color: var(--service-text);
  font-size: clamp(1.25rem, 3vw, 1.5rem);
  font-weight: 600;
  letter-spacing: -0.02em;
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

const Metadata = styled.p`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.25rem 0.5rem;
  margin: 0.45rem 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.3;

  > span:not(:last-child)::after {
    margin-left: 0.5rem;
    color: var(--service-border-strong);
    content: "·";
  }
`;

const RankMark = styled.span`
  display: inline-flex;
  align-items: baseline;
  gap: 0.3rem;
  color: var(--service-text-muted);

  strong {
    color: var(--service-accent);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
`;

const Actions = styled.div`
  display: flex;
  flex: 0 0 auto;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.875rem;

  button.profile-primary-action {
    padding-right: 1rem;
    padding-left: 1rem;
    font-size: 0.875rem;
  }

  @container (max-width: 34rem) {
    width: 100%;
  }
`;

const TextAction = styled.button`
  display: inline-flex;
  min-height: 34px;
  align-items: center;
  padding: 0;
  color: var(--service-text);
  background: transparent;
  border: 0;
  font: inherit;
  font-size: 0.875rem;
  font-weight: 500;
  text-decoration: none;
  cursor: pointer;

  &:hover {
    text-decoration: underline;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 3px;
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }
`;

const TextLink = styled.a`
  display: inline-flex;
  min-height: 34px;
  align-items: center;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
  text-decoration: none;

  &:hover {
    text-decoration: underline;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 3px;
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }
`;

const FactsHeader = styled.div`
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 0.75rem 1.25rem;
  margin-top: 1.25rem;
  padding-bottom: 0.5rem;
  border-bottom: 1px solid var(--service-border);

  @container (max-width: 30rem) {
    align-items: flex-start;
    flex-direction: column-reverse;
    gap: 0.5rem;
  }
`;

const PeriodNav = styled(DitherSelectionNav)``;

const Metrics = styled.dl`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin: 0.75rem 0 0;

  @container (max-width: 36rem) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    row-gap: 0.875rem;
  }
`;

const Metric = styled.div`
  min-width: 0;
  padding-right: 1rem;

  & + & {
    padding-left: 1rem;
    border-left: 1px solid var(--service-border);
  }

  @container (max-width: 36rem) {
    &:nth-child(odd) {
      padding-left: 0;
      border-left: 0;
    }

    &:nth-child(even) {
      padding-left: 1rem;
      border-left: 1px solid var(--service-border);
    }

    &:nth-child(n + 3) {
      margin-top: 0.875rem;
      padding-top: 0.75rem;
      border-top: 1px solid var(--service-border);
    }
  }
`;

const MetricLabel = styled.dt`
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  line-height: 1.2;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const MetricValue = styled.dd<{ $lead?: boolean }>`
  width: 100%;
  overflow: hidden;
  margin: 0.3rem 0 0;
  color: var(--service-text);
  font-size: ${(props) => (props.$lead ? "2rem" : "1.375rem")};
  font-variant-numeric: tabular-nums;
  font-weight: 500;
  letter-spacing: -0.01em;
  line-height: 1.1;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const subscribeNoop = () => () => {};

/** Keep server and first-hydration output in UTC, then use the viewer's zone. */
export function formatLastUpdated(
  lastUpdated: string | null | undefined,
  isMounted: boolean,
  timeZone?: string,
): string | null {
  if (!lastUpdated) return null;
  const date = new Date(lastUpdated);
  return isMounted
    ? date.toLocaleString("en-US", timeZone ? { timeZone } : undefined)
    : date.toLocaleString("en-US", { timeZone: "UTC" });
}

/**
 * "Joined" is an absolute instant (`users.createdAt`), so projecting it into the
 * viewer's chosen zone is meaningful and two viewers may legitimately read a
 * different month across a UTC boundary. Contribution dates never take this
 * path — see src/lib/timezone.ts.
 */
export function formatJoined(
  createdAt: string | null | undefined,
  timeZone?: string,
): string | null {
  if (!createdAt) return null;
  const date = new Date(createdAt);
  if (Number.isNaN(date.getTime())) return null;

  const options: Intl.DateTimeFormatOptions = {
    month: "short",
    year: "numeric",
  };
  if (timeZone) options.timeZone = timeZone;
  return date.toLocaleDateString("en-US", options);
}

function buildProfilePeriodHref(
  username: string,
  period: ProfilePeriod,
): string {
  const basePath = `/u/${encodeURIComponent(username)}`;
  return period === "all" ? basePath : `${basePath}?period=${period}`;
}

export function ProfileOverview({
  user,
  stats,
  lastUpdated,
  period = "all",
  timeZone,
  className,
}: ProfileOverviewProps) {
  const [isEmbedDialogOpen, setIsEmbedDialogOpen] = useState(false);
  const [avatarFailed, setAvatarFailed] = useState(false);
  const isMounted = useSyncExternalStore(
    subscribeNoop,
    () => true,
    () => false,
  );
  const formattedLastUpdated = useMemo(
    () => formatLastUpdated(lastUpdated, isMounted, timeZone),
    [isMounted, lastUpdated, timeZone],
  );
  const joined = useMemo(
    () => formatJoined(user.createdAt, isMounted ? timeZone : "UTC"),
    [user.createdAt, isMounted, timeZone],
  );
  const displayName = user.displayName || user.username;
  const avatarUrl = user.avatarUrl;
  const showAvatar = Boolean(avatarUrl) && !avatarFailed;
  const periodOptions = useMemo(
    () =>
      PROFILE_PERIODS.map((value) => ({
        value,
        label: PROFILE_PERIOD_LABELS[value],
        href: buildProfilePeriodHref(user.username, value),
      })),
    [user.username],
  );

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
            : `${PROFILE_PERIOD_LABELS[period]} tokens`,
      value: formatNumber(stats.totalTokens),
      title: stats.totalTokens.toLocaleString("en-US"),
      lead: true,
    },
    {
      label:
        period === "all"
          ? "All-time cost"
          : period === "month"
            ? "30d cost"
            : `${PROFILE_PERIOD_LABELS[period]} cost`,
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
            : `Active days (${PROFILE_PERIOD_LABELS[period]})`,
      value: stats.activeDays.toLocaleString("en-US"),
      title: stats.activeDays.toLocaleString("en-US"),
    },
    {
      label: "Submissions (all-time)",
      value: (stats.submissionCount ?? 0).toLocaleString("en-US"),
      title: (stats.submissionCount ?? 0).toLocaleString("en-US"),
    },
  ];

  return (
    <OverviewPanel
      className={className}
      aria-labelledby="profile-overview-heading"
    >
      <IdentityRow>
        <Identity>
          <Avatar>
            {showAvatar ? (
              <AvatarImage
                src={avatarUrl as string}
                alt={`${displayName}'s avatar`}
                fill
                sizes="(min-width: 640px) 80px, 72px"
                priority
                onError={() => setAvatarFailed(true)}
              />
            ) : (
              <AvatarFallback
                name={user.username}
                color="blue"
                grid={8}
                cellPx={5}
              />
            )}
          </Avatar>

          <IdentityCopy>
            <DisplayName id="profile-overview-heading">
              {displayName}
            </DisplayName>
            <Handle>@{user.username}</Handle>

            {(user.rank != null || joined || formattedLastUpdated) && (
              <Metadata>
                {user.rank != null && (
                  <RankMark title={describeRankingWindow(user.rankDateRange)}>
                    <span>{getProfileRankLabel(period)}</span>
                    <strong>#{user.rank.toLocaleString("en-US")}</strong>
                  </RankMark>
                )}
                {joined && <span>Joined {joined}</span>}
                {formattedLastUpdated && (
                  <span suppressHydrationWarning>
                    Updated {formattedLastUpdated}
                  </span>
                )}
              </Metadata>
            )}
          </IdentityCopy>
        </Identity>

        <Actions aria-label="Profile actions">
          <DitherButton
            type="button"
            color="blue"
            variant="solid"
            className="profile-primary-action"
            onClick={() => setIsEmbedDialogOpen(true)}
            aria-label={`Open GitHub README embed options for ${displayName}`}
          >
            Embed
          </DitherButton>
          <TextAction
            type="button"
            onClick={handleShareClick}
            aria-label={`Share ${displayName}'s profile`}
          >
            Share
          </TextAction>
          <TextLink
            href={`https://github.com/${user.username}`}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={`View ${user.username}'s GitHub profile (opens in new tab)`}
          >
            GitHub
          </TextLink>
        </Actions>
      </IdentityRow>

      <FactsHeader>
        <PeriodNav
          label="Usage period"
          value={period}
          options={periodOptions}
        />
      </FactsHeader>

      <Metrics aria-label="Profile summary">
        {headlineMetrics.map((metric) => (
          <Metric key={metric.label}>
            <MetricLabel>{metric.label}</MetricLabel>
            <MetricValue $lead={metric.lead} title={metric.title}>
              {metric.value}
            </MetricValue>
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
