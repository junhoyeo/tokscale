"use client";

import Image from "next/image";
import styled, { css } from "styled-components";
import { GraphContainer } from "@/components/GraphContainer";
import type { TokenContributionData } from "@/lib/types";
import { formatNumber, formatCurrency } from "@/lib/utils";

export interface ProfileUser {
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  rank: number | null;
}

export interface ProfileStatsData {
  totalTokens: number;
  totalCost: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  activeDays: number;
  submissionCount?: number;
}

export interface ProfileHeaderProps {
  user: ProfileUser;
  stats: ProfileStatsData;
  lastUpdated?: string;
}

const HeaderContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  border-radius: 1rem;
  border-width: 1px;
  border-style: solid;
  padding: 1rem;
  padding-bottom: 18px;
`;

const HeaderContent = styled.div`
  display: flex;
  flex-direction: column;
  gap: 1.5rem;

  @media (min-width: 1024px) {
    flex-direction: row;
    gap: 2.5rem;
  }
`;

const UserInfoCard = styled.div`
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 19px;
  border-radius: 20px;
  padding-top: 0.75rem;
  padding-bottom: 0.75rem;
  padding-left: 0.75rem;
  padding-right: 2rem;
  flex: 1;
`;

const AvatarContainer = styled.div`
  position: relative;
  width: 100px;
  height: 100px;
  border-radius: 7px;
  overflow: hidden;
  border-width: 2px;
  border-style: solid;
  flex-shrink: 0;
`;

const StyledAvatarImage = styled(Image)`
  object-fit: cover;
`;

const UserDetails = styled.div`
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  justify-content: flex-end;
  gap: 6px;
  padding-top: 0;
  padding-bottom: 0.25rem;
  height: 100px;
`;

const RankBadge = styled.div`
  width: 2rem;
  height: 2rem;
  border-radius: 0.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
`;

const RankText = styled.span`
  font-size: 1rem;
  font-weight: 500;
`;

const NameContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  justify-content: flex-end;
  min-width: 0;
`;

const NameHeading = styled.h1`
  font-size: 1.5rem;
  font-weight: 700;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  line-height: 1.2;
`;

const HandleText = styled.p`
  font-size: 0.875rem;
  font-weight: 700;
  line-height: 1;
`;

const StatsRow = styled.div`
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 1.75rem;
  height: 124px;
  flex: 1;
`;

const StatItem = styled.div`
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  min-width: 120px;
`;

const StatLabel = styled.span`
  font-size: 1rem;
  font-weight: 600;
  line-height: 1;
`;

const StatValue = styled.span`
  font-size: 27px;
  font-weight: 700;
  line-height: 1;
`;

const Divider = styled.div`
  width: 100%;
  height: 1px;
`;

const FooterRow = styled.div`
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: 0.75rem;
  align-items: flex-start;

  @media (min-width: 640px) {
    flex-direction: row;
    align-items: flex-end;
  }
`;

const LastUpdatedText = styled.span`
  font-size: 0.875rem;
  line-height: 1.21;
`;

const ActionButtons = styled.div`
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 6px;
`;

const actionButtonStyles = css`
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: center;
  gap: 6px;
  border-radius: 9999px;
  border-width: 1px;
  border-style: solid;
  padding-top: 9px;
  padding-bottom: 9px;
  padding-left: 10px;
  padding-right: 11px;
  transition: opacity 150ms ease-in-out;
  cursor: pointer;

  &:hover {
    opacity: 0.8;
  }
  
  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--color-bg-default), 0 0 0 4px #3b82f6;
  }
`;

const ActionButton = styled.button`
  ${actionButtonStyles}
`;

const ActionLink = styled.a`
  ${actionButtonStyles}
  text-decoration: none;
`;

const ActionText = styled.span`
  font-size: 0.875rem;
  line-height: 1;
`;

export function ProfileHeader({ user, stats, lastUpdated }: ProfileHeaderProps) {
  const avatarUrl = user.avatarUrl || `https://github.com/${user.username}.png`;

  return (
    <HeaderContainer
      style={{ backgroundColor: "#141A21", borderColor: "var(--color-border-default)" }}
    >
      <HeaderContent>
        <UserInfoCard
          style={{ backgroundColor: "var(--color-bg-darkest)" }}
        >
          <AvatarContainer
            style={{ borderColor: "var(--color-border-default)" }}
          >
            <StyledAvatarImage
              src={avatarUrl}
              alt={user.username}
              fill
            />
          </AvatarContainer>

          <UserDetails>
            {user.rank && (
              <RankBadge
                style={{
                  background: "linear-gradient(135deg, var(--color-bg-darkest) 0%, color-mix(in srgb, var(--color-accent-blue) 20%, var(--color-bg-darkest)) 50%, color-mix(in srgb, var(--color-accent-blue) 35%, var(--color-bg-darkest)) 100%)",
                  border: "1px solid var(--color-border-default)",
                }}
              >
                <RankText
                  style={{ color: "var(--color-accent-blue)" }}
                >
                  #{user.rank}
                </RankText>
              </RankBadge>
            )}

            <NameContainer>
              <NameHeading
                style={{ color: "var(--color-fg-default)" }}
              >
                {user.displayName || user.username}
              </NameHeading>
              <HandleText
                style={{ color: "var(--color-fg-muted)" }}
              >
                @{user.username}
              </HandleText>
            </NameContainer>
          </UserDetails>
        </UserInfoCard>

        <StatsRow>
          <StatItem>
            <StatLabel
              style={{ color: "var(--color-accent-blue)" }}
            >
              Total Tokens
            </StatLabel>
            <StatValue
              style={{
                background: "linear-gradient(117deg, #169AFF 0%, #9FD4FB 26%, #B9DFF8 52%)",
                WebkitBackgroundClip: "text",
                WebkitTextFillColor: "transparent",
                backgroundClip: "text",
                textDecoration: "none",
              }}
              title={stats.totalTokens.toLocaleString()}
            >
              {formatNumber(stats.totalTokens)}
            </StatValue>
          </StatItem>

          <StatItem>
            <StatLabel
              style={{ color: "var(--color-fg-default)" }}
            >
              Total Cost
            </StatLabel>
            <StatValue
              style={{ color: "var(--color-fg-default)", textDecoration: "none" }}
              title={stats.totalCost.toLocaleString('en-US', { style: 'currency', currency: 'USD' })}
            >
              {formatCurrency(stats.totalCost)}
            </StatValue>
          </StatItem>
        </StatsRow>
      </HeaderContent>

      <Divider style={{ backgroundColor: "var(--color-border-default)" }} />

      <FooterRow>
        {lastUpdated && (
          <LastUpdatedText
            style={{ color: "var(--color-fg-muted)" }}
          >
            Last Updated: {new Date(lastUpdated).toLocaleString()}
          </LastUpdatedText>
        )}

        <ActionButtons>
          <ActionButton
            aria-label={`Share ${user.displayName || user.username}'s profile`}
            style={{ backgroundColor: "var(--color-btn-bg)", borderColor: "var(--color-border-default)" }}
          >
            <Image src="/icons/icon-share.svg" alt="" width={20} height={20} aria-hidden="true" />
            <ActionText
              style={{ color: "var(--color-fg-default)" }}
            >
              Share
            </ActionText>
          </ActionButton>

          <ActionLink
            href={`https://github.com/${user.username}`}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={`View ${user.username}'s GitHub profile (opens in new tab)`}
            style={{ backgroundColor: "var(--color-btn-bg)", borderColor: "var(--color-border-default)" }}
          >
            <Image src="/icons/icon-github.svg" alt="" width={20} height={20} aria-hidden="true" />
            <ActionText
              style={{ color: "var(--color-fg-default)" }}
            >
              GitHub
            </ActionText>
          </ActionLink>
        </ActionButtons>
      </FooterRow>
    </HeaderContainer>
  );
}

export type ProfileTab = "activity" | "breakdown" | "models";

export interface ProfileTabBarProps {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
}

const TabBarContainer = styled.div`
  display: inline-flex;
  flex-direction: row;
  align-items: center;
  border-radius: 25px;
  border-width: 1px;
  border-style: solid;
  padding: 6px;
  width: fit-content;
`;

const TabButton = styled.button`
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 25px;
  padding-left: 1.25rem;
  padding-right: 1.25rem;
  padding-top: 10px;
  padding-bottom: 10px;
  transition: background-color 150ms;
  cursor: pointer;
  border: none;

  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--color-bg-elevated), 0 0 0 4px #3b82f6;
  }
`;

const TabText = styled.span`
  font-size: 1.125rem;
  font-weight: 600;
  line-height: 1;
  white-space: nowrap;
`;

export function ProfileTabBar({ activeTab, onTabChange }: ProfileTabBarProps) {
  const tabs: { id: ProfileTab; label: string }[] = [
    { id: "activity", label: "Activity" },
    { id: "breakdown", label: "Token Breakdown" },
    { id: "models", label: "Models Used" },
  ];

  const handleKeyDown = (e: React.KeyboardEvent, currentIndex: number) => {
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      const nextIndex = (currentIndex + 1) % tabs.length;
      onTabChange(tabs[nextIndex].id);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      const prevIndex = (currentIndex - 1 + tabs.length) % tabs.length;
      onTabChange(tabs[prevIndex].id);
    } else if (e.key === "Home") {
      e.preventDefault();
      onTabChange(tabs[0].id);
    } else if (e.key === "End") {
      e.preventDefault();
      onTabChange(tabs[tabs.length - 1].id);
    }
  };

  return (
    <TabBarContainer
      role="tablist"
      aria-label="Profile tabs"
      style={{
        backgroundColor: "var(--color-bg-elevated)",
        borderColor: "var(--color-border-default)",
      }}
    >
      {tabs.map((tab, index) => {
        const isActive = activeTab === tab.id;
        return (
          <TabButton
            key={tab.id}
            role="tab"
            aria-selected={isActive}
            aria-controls={`tabpanel-${tab.id}`}
            tabIndex={isActive ? 0 : -1}
            onClick={() => onTabChange(tab.id)}
            onKeyDown={(e) => handleKeyDown(e, index)}
            style={{
              backgroundColor: isActive ? "var(--color-bg-active)" : "transparent",
            }}
          >
            <TabText
              style={{
                color: isActive ? "var(--color-fg-default)" : "color-mix(in srgb, var(--color-fg-default) 50%, transparent)",
              }}
            >
              {tab.label}
            </TabText>
          </TabButton>
        );
      })}
    </TabBarContainer>
  );
}

export interface TokenBreakdownProps {
  stats: ProfileStatsData;
}

const BreakdownContainer = styled.div`
  border-radius: 1rem;
  border-width: 1px;
  border-style: solid;
  padding: 1rem;

  @media (min-width: 640px) {
    padding: 1.5rem;
  }
`;

const ProgressBarWrapper = styled.div`
  margin-bottom: 1.5rem;
`;

const ProgressBar = styled.div`
  height: 0.75rem;
  border-radius: 9999px;
  overflow: hidden;
  display: flex;
`;

const LegendGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;

  @media (min-width: 768px) {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
`;

const LegendItem = styled.div`
  display: flex;
  align-items: center;
  gap: 0.75rem;
`;

const LegendDot = styled.div`
  width: 0.75rem;
  height: 0.75rem;
  border-radius: 9999px;
  flex-shrink: 0;
`;

const LegendInfo = styled.div`
  min-width: 0;
`;

const LegendHeader = styled.div`
  display: flex;
  align-items: center;
  gap: 0.5rem;
`;

const LegendLabel = styled.p`
  font-size: 0.75rem;
`;

const LegendPercentage = styled.span`
  font-size: 0.75rem;
`;

const LegendValue = styled.p`
  font-size: 1rem;
  font-weight: 600;

  @media (min-width: 640px) {
    font-size: 1.125rem;
  }
`;

export function TokenBreakdown({ stats }: TokenBreakdownProps) {
  const { totalTokens, inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens } = stats;

  const tokenTypes = [
    { label: "Input", value: inputTokens, color: "#006edb", percentage: totalTokens > 0 ? (inputTokens / totalTokens) * 100 : 0 },
    { label: "Output", value: outputTokens, color: "#894ceb", percentage: totalTokens > 0 ? (outputTokens / totalTokens) * 100 : 0 },
    { label: "Cache Read", value: cacheReadTokens, color: "#30a147", percentage: totalTokens > 0 ? (cacheReadTokens / totalTokens) * 100 : 0 },
    { label: "Cache Write", value: cacheWriteTokens, color: "#eb670f", percentage: totalTokens > 0 ? (cacheWriteTokens / totalTokens) * 100 : 0 },
  ];

  return (
    <BreakdownContainer
      style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
    >
      {totalTokens > 0 && (
        <ProgressBarWrapper>
          <ProgressBar
            style={{ backgroundColor: "var(--color-bg-subtle)" }}
          >
            {tokenTypes.map((type) => (
              <div
                key={type.label}
                style={{
                  width: `${type.percentage}%`,
                  backgroundColor: type.color,
                }}
                title={`${type.label}: ${formatNumber(type.value)}`}
              />
            ))}
          </ProgressBar>
        </ProgressBarWrapper>
      )}

      <LegendGrid>
        {tokenTypes.map((type) => (
          <LegendItem key={type.label}>
            <LegendDot style={{ backgroundColor: type.color }} />
            <LegendInfo>
              <LegendHeader>
                <LegendLabel style={{ color: "var(--color-fg-muted)" }}>{type.label}</LegendLabel>
                {type.percentage > 0 && (
                  <LegendPercentage style={{ color: "var(--color-fg-subtle)" }}>
                    {type.percentage.toFixed(1)}%
                  </LegendPercentage>
                )}
              </LegendHeader>
              <LegendValue
                style={{ color: "var(--color-fg-default)" }}
              >
                {formatNumber(type.value)}
              </LegendValue>
            </LegendInfo>
          </LegendItem>
        ))}
      </LegendGrid>
    </BreakdownContainer>
  );
}

export interface ProfileStatsProps {
  stats: ProfileStatsData;
  favoriteModel?: string;
}

const StatsContainer = styled(BreakdownContainer)``;

const StatsGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;

  @media (min-width: 640px) {
    gap: 1.5rem;
  }
  
  @media (min-width: 768px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
`;

const StatsItem = styled.div`
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
`;

const StatsLabel = styled.p`
  font-size: 0.75rem;
  
  @media (min-width: 640px) {
    font-size: 0.875rem;
  }
`;

const StatsValue = styled.p`
  font-size: 1.125rem;
  font-weight: 700;

  @media (min-width: 640px) {
    font-size: 1.25rem;
  }
`;

export function ProfileStats({ stats, favoriteModel }: ProfileStatsProps) {
  const statItems = [
    { label: "Submits", value: (stats.submissionCount ?? 0).toString(), color: "var(--color-primary)" },
    { label: "Favorite Model", value: favoriteModel ?? "N/A", color: "var(--color-primary)" },
  ];

  return (
    <StatsContainer
      style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
    >
      <StatsGrid>
        {statItems.map((item) => (
          <StatsItem key={item.label}>
            <StatsLabel style={{ color: "var(--color-fg-muted)" }}>{item.label}</StatsLabel>
            <StatsValue
              style={{ color: item.color }}
            >
              {item.value}
            </StatsValue>
          </StatsItem>
        ))}
      </StatsGrid>
    </StatsContainer>
  );
}

const MODEL_COLORS: Record<string, string> = {
  "claude": "#D97706",
  "sonnet": "#D97706",
  "opus": "#DC2626",
  "haiku": "#059669",
  "gpt": "#10B981",
  "o1": "#6366F1",
  "o3": "#8B5CF6",
  "gemini": "#3B82F6",
  "deepseek": "#06B6D4",
  "codex": "#F59E0B",
};

function getModelColor(modelName: string): string {
  const lowerName = modelName.toLowerCase();
  for (const [key, color] of Object.entries(MODEL_COLORS)) {
    if (lowerName.includes(key)) return color;
  }
  return "#6B7280";
}

export interface ModelUsage {
  model: string;
  tokens: number;
  cost: number;
  percentage: number;
}

export interface ProfileModelsProps {
  models: string[];
  modelUsage?: ModelUsage[];
}

const ModelsListContainer = styled.div`
  border-radius: 1rem;
  border-width: 1px;
  border-style: solid;
  overflow: hidden;
`;

const ModelsListHeader = styled.div`
  display: grid;
  grid-template-columns: 1fr auto auto auto;
  gap: 1rem;
  padding-left: 1rem;
  padding-right: 1rem;
  padding-top: 0.75rem;
  padding-bottom: 0.75rem;
  font-size: 0.75rem;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom-width: 1px;
  border-bottom-style: solid;

  @media (min-width: 640px) {
    padding-left: 1.5rem;
    padding-right: 1.5rem;
  }
`;

const ModelsListRow = styled.div`
  display: grid;
  grid-template-columns: 1fr auto auto auto;
  gap: 1rem;
  padding-left: 1rem;
  padding-right: 1rem;
  padding-top: 0.75rem;
  padding-bottom: 0.75rem;
  align-items: center;

  @media (min-width: 640px) {
    padding-left: 1.5rem;
    padding-right: 1.5rem;
  }
`;

const ModelNameCell = styled.div`
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
`;

const ModelColorDot = styled.div`
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 9999px;
  flex-shrink: 0;
`;

const ModelNameText = styled.span`
  font-size: 0.875rem;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const ModelMetricCell = styled.div<{ $width: string; $smWidth: string }>`
  text-align: right;
  width: ${props => props.$width};

  @media (min-width: 640px) {
    width: ${props => props.$smWidth};
  }
`;

const MetricText = styled.span`
  font-size: 0.875rem;
`;

const CostText = styled.span`
  font-size: 0.875rem;
  font-weight: 500;
`;

const ModelsTagsContainer = styled(BreakdownContainer)``;

const ModelsTagsWrapper = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
`;

const ModelTag = styled.span`
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding-left: 0.75rem;
  padding-right: 0.75rem;
  padding-top: 0.375rem;
  padding-bottom: 0.375rem;
  border-radius: 9999px;
  font-size: 0.875rem;
  font-weight: 500;
`;

export function ProfileModels({ models, modelUsage }: ProfileModelsProps) {
  const filteredModels = models.filter((m) => m !== "<synthetic>");

  if (filteredModels.length === 0) return null;

  if (modelUsage && modelUsage.length > 0) {
    const sortedUsage = [...modelUsage].sort((a, b) => b.cost - a.cost);

    return (
      <ModelsListContainer
        style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
      >
        <ModelsListHeader
          style={{ backgroundColor: "var(--color-bg-elevated)", borderColor: "var(--color-border-default)", color: "var(--color-fg-muted)" }}
        >
          <div>Model</div>
          <ModelMetricCell $width="5rem" $smWidth="6rem">Tokens</ModelMetricCell>
          <ModelMetricCell $width="4rem" $smWidth="5rem">Cost</ModelMetricCell>
          <ModelMetricCell $width="3rem" $smWidth="4rem">%</ModelMetricCell>
        </ModelsListHeader>

        <div>
          {sortedUsage.map((usage, index) => (
            <ModelsListRow
              key={usage.model}
              style={{
                backgroundColor: index % 2 === 1 ? "var(--color-bg-elevated)" : "transparent",
                borderTop: index > 0 ? "1px solid var(--color-border-default)" : undefined,
              }}
            >
              <ModelNameCell>
                <ModelColorDot style={{ backgroundColor: getModelColor(usage.model) }} />
                <ModelNameText style={{ color: "var(--color-fg-default)" }}>
                  {usage.model}
                </ModelNameText>
              </ModelNameCell>
              <ModelMetricCell $width="5rem" $smWidth="6rem">
                <MetricText style={{ color: "var(--color-fg-default)" }}>
                  {formatNumber(usage.tokens)}
                </MetricText>
              </ModelMetricCell>
              <ModelMetricCell $width="4rem" $smWidth="5rem">
                <CostText style={{ color: "var(--color-primary)" }}>
                  {formatCurrency(usage.cost)}
                </CostText>
              </ModelMetricCell>
              <ModelMetricCell $width="3rem" $smWidth="4rem">
                <MetricText style={{ color: "var(--color-fg-muted)" }}>
                  {usage.percentage.toFixed(1)}%
                </MetricText>
              </ModelMetricCell>
            </ModelsListRow>
          ))}
        </div>
      </ModelsListContainer>
    );
  }

  return (
    <ModelsTagsContainer
      style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
    >
      <ModelsTagsWrapper>
        {filteredModels.map((model) => (
          <ModelTag
            key={model}
            style={{ backgroundColor: "var(--color-bg-subtle)", color: "var(--color-fg-default)" }}
          >
            <ModelColorDot style={{ backgroundColor: getModelColor(model) }} />
            {model}
          </ModelTag>
        ))}
      </ModelsTagsWrapper>
    </ModelsTagsContainer>
  );
}

export interface ProfileActivityProps {
  data: TokenContributionData;
}

const ActivityContainer = styled.div`
  overflow-x: auto;
  margin-left: -1rem;
  margin-right: -1rem;
  padding-left: 1rem;
  padding-right: 1rem;

  @media (min-width: 640px) {
    margin-left: 0;
    margin-right: 0;
    padding-left: 0;
    padding-right: 0;
  }
`;

const ActivityInner = styled.div`
  min-width: 600px;
  
  @media (min-width: 640px) {
    min-width: 0;
  }
`;

export function ProfileActivity({ data }: ProfileActivityProps) {
  return (
    <ActivityContainer>
      <ActivityInner>
        <GraphContainer data={data} />
      </ActivityInner>
    </ActivityContainer>
  );
}

const EmptyActivityContainer = styled.div`
  border-radius: 1rem;
  border-width: 1px;
  border-style: solid;
  padding: 1.5rem;
  text-align: center;

  @media (min-width: 640px) {
    padding: 2rem;
  }
`;

const EmptyActivityText = styled.p`
  font-size: 0.875rem;

  @media (min-width: 640px) {
    font-size: 1rem;
  }
`;

export function ProfileEmptyActivity() {
  return (
    <EmptyActivityContainer
      style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
    >
      <EmptyActivityText style={{ color: "var(--color-fg-muted)" }}>
        No contribution data available yet.
      </EmptyActivityText>
    </EmptyActivityContainer>
  );
}
