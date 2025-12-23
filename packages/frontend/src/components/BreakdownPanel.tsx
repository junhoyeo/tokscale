"use client";

import styled from "styled-components";
import type { DailyContribution, GraphColorPalette, SourceType } from "@/lib/types";
import { formatDateFull, formatCurrency, formatTokenCount, groupSourcesByType, sortSourcesByCost } from "@/lib/utils";
import { SOURCE_DISPLAY_NAMES, SOURCE_COLORS } from "@/lib/constants";
import { SourceLogo } from "./SourceLogo";

interface BreakdownPanelProps {
  day: DailyContribution | null;
  onClose: () => void;
  palette: GraphColorPalette;
}

const PanelContainer = styled.div`
  margin-top: 2rem;
  border-radius: 1rem;
  border: 1px solid;
  overflow: hidden;
  box-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.05);
  transition: box-shadow 150ms;

  &:hover {
    box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1);
  }
`;

const PanelHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.5rem;
  border-bottom: 1px solid;
`;

const PanelTitle = styled.h3`
  font-weight: 700;
  font-size: 1.125rem;
`;

const CloseButton = styled.button`
  padding: 0.5rem;
  border-radius: 9999px;
  transition: all 200ms;

  &:hover {
    background-color: var(--color-btn-hover-bg);
    transform: scale(1.1);
  }

  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--color-bg-default, #fff), 0 0 0 4px #3b82f6;
  }
`;

const PanelContent = styled.div`
  padding: 1.5rem;
`;

const EmptyState = styled.p`
  text-align: center;
  padding-top: 2rem;
  padding-bottom: 2rem;
  font-size: 0.875rem;
  font-weight: 500;
`;

const SourceList = styled.div`
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
`;

const SummaryFooter = styled.div`
  margin-top: 1.5rem;
  padding-top: 1.5rem;
  border-top: 1px solid;
  display: flex;
  flex-wrap: wrap;
  gap: 1.5rem;
  font-size: 0.875rem;
`;

const SummaryItem = styled.div`
  font-weight: 500;
`;

const SummaryValue = styled.span`
  font-weight: 600;
`;

const SummaryTotalValue = styled(SummaryValue)`
  font-weight: 700;
  font-size: 1rem;
`;

export function BreakdownPanel({ day, onClose, palette }: BreakdownPanelProps) {
  if (!day) return null;

  const groupedSources = groupSourcesByType(day.sources);
  const sortedSourceTypes = Array.from(groupedSources.keys()).sort();

  return (
    <PanelContainer
      role="region"
      aria-label="Day breakdown"
      style={{ backgroundColor: "var(--color-card-bg)", borderColor: "var(--color-border-default)" }}
    >
      <PanelHeader style={{ borderColor: "var(--color-border-default)" }}>
        <PanelTitle style={{ color: "var(--color-fg-default)" }}>
          {formatDateFull(day.date)} - Detailed Breakdown
        </PanelTitle>
        <CloseButton
          onClick={onClose}
          aria-label="Close breakdown panel"
          style={{ color: "var(--color-fg-muted)" }}
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </CloseButton>
      </PanelHeader>

      <PanelContent>
        {day.sources.length === 0 ? (
          <EmptyState style={{ color: "var(--color-fg-muted)" }}>
            No activity on this day
          </EmptyState>
        ) : (
          <SourceList>
            {sortedSourceTypes.map((sourceType) => {
              const sources = sortSourcesByCost(groupedSources.get(sourceType) || []);
              const sourceTotalCost = sources.reduce((sum, s) => sum + s.cost, 0);
              return (
                <SourceSection key={sourceType} sourceType={sourceType} sources={sources} totalCost={sourceTotalCost} palette={palette} />
              );
            })}
          </SourceList>
        )}

        {day.sources.length > 0 && (
          <SummaryFooter style={{ borderColor: "var(--color-border-default)" }}>
            <SummaryItem style={{ color: "var(--color-fg-muted)" }}>
              Total: <SummaryTotalValue style={{ color: "var(--color-fg-default)" }}>{formatCurrency(day.totals.cost)}</SummaryTotalValue>
            </SummaryItem>
            <SummaryItem style={{ color: "var(--color-fg-muted)" }}>
              across <SummaryValue style={{ color: "var(--color-fg-default)" }}>{sortedSourceTypes.length} source{sortedSourceTypes.length !== 1 ? "s" : ""}</SummaryValue>
            </SummaryItem>
            <SummaryItem style={{ color: "var(--color-fg-muted)" }}>
              <SummaryValue style={{ color: "var(--color-fg-default)" }}>
                {(() => {
                  const allModels = new Set<string>();
                  for (const s of day.sources) {
                    if (s.models) {
                      for (const modelId of Object.keys(s.models)) {
                        allModels.add(modelId);
                      }
                    } else if (s.modelId) {
                      allModels.add(s.modelId);
                    }
                  }
                  const count = allModels.size;
                  return `${count} model${count !== 1 ? "s" : ""}`;
                })()}
              </SummaryValue>
            </SummaryItem>
          </SummaryFooter>
        )}
      </PanelContent>
    </PanelContainer>
  );
}

interface SourceSectionProps {
  sourceType: SourceType;
  sources: DailyContribution["sources"];
  totalCost: number;
  palette: GraphColorPalette;
}

const SectionHeader = styled.div`
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 0.75rem;
`;

const SourceBadge = styled.span`
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem 0.75rem;
  border-radius: 9999px;
  font-size: 0.75rem;
  font-weight: 600;
  transition: transform 150ms;

  &:hover {
    transform: scale(1.05);
  }
`;

const SectionTotal = styled.span`
  font-size: 0.875rem;
  font-weight: 700;
`;

const ModelsList = styled.div`
  margin-left: 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
`;

function SourceSection({ sourceType, sources, totalCost, palette }: SourceSectionProps) {
  const sourceColor = SOURCE_COLORS[sourceType] || palette.grade3;

  const modelEntries: Array<{ modelId: string; cost: number; messages: number; tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; reasoning: number } }> = [];
  for (const source of sources) {
    if (source.models && Object.keys(source.models).length > 0) {
      for (const [modelId, data] of Object.entries(source.models)) {
        modelEntries.push({
          modelId,
          cost: data.cost || 0,
          messages: data.messages || 0,
          tokens: {
            input: data.input || 0,
            output: data.output || 0,
            cacheRead: data.cacheRead || 0,
            cacheWrite: data.cacheWrite || 0,
            reasoning: data.reasoning || 0,
          },
        });
      }
    } else if (source.modelId) {
      modelEntries.push({
        modelId: source.modelId,
        cost: source.cost,
        messages: source.messages,
        tokens: source.tokens,
      });
    }
  }

  const sortedModels = modelEntries.sort((a, b) => b.cost - a.cost);

  return (
    <div>
      <SectionHeader>
        <SourceBadge
          style={{ backgroundColor: `${sourceColor}20`, color: sourceColor }}
        >
          <SourceLogo sourceId={sourceType} height={14} />
          {SOURCE_DISPLAY_NAMES[sourceType] || sourceType}
        </SourceBadge>
        <SectionTotal style={{ color: "var(--color-fg-default)" }}>{formatCurrency(totalCost)}</SectionTotal>
      </SectionHeader>

      <ModelsList>
        {sortedModels.map((model, index) => (
          <ModelRow key={`${model.modelId}-${index}`} model={model} isLast={index === sortedModels.length - 1} palette={palette} />
        ))}
      </ModelsList>
    </div>
  );
}

interface ModelRowProps {
  model: {
    modelId: string;
    cost: number;
    messages: number;
    tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; reasoning: number };
  };
  isLast: boolean;
  palette: GraphColorPalette;
}

const ModelRowContainer = styled.div`
  position: relative;
`;

const TreeLineContainer = styled.div`
  position: absolute;
  left: 0;
  top: 0;
  width: 1rem;
  height: 100%;
`;

const TreeLineBranch = styled.span`
  position: absolute;
  left: 0;
  top: 0.75rem;
  width: 0.75rem;
  border-top: 1px solid;
`;

const TreeLineVertical = styled.span`
  position: absolute;
  left: 0;
  top: 0;
  height: 100%;
  border-left: 1px solid;
`;

const ModelContent = styled.div`
  margin-left: 1.5rem;
`;

const ModelHeader = styled.div`
  display: flex;
  align-items: center;
  gap: 0.75rem;
  flex-wrap: wrap;
`;

const ModelName = styled.span`
  font-family: var(--font-mono);
  font-size: 0.875rem;
  font-weight: 600;
`;

const ModelCost = styled.span`
  font-weight: 700;
  font-size: 0.875rem;
`;

const TokenBadgesGrid = styled.div`
  margin-top: 0.5rem;
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  column-gap: 1rem;
  row-gap: 0.5rem;
  font-size: 0.75rem;

  @media (min-width: 640px) {
    grid-template-columns: repeat(3, 1fr);
  }

  @media (min-width: 768px) {
    grid-template-columns: repeat(5, 1fr);
  }
`;

const MessageCount = styled.div`
  margin-top: 0.5rem;
  font-size: 0.75rem;
  font-weight: 500;
`;

function ModelRow({ model, isLast, palette }: ModelRowProps) {
  const { modelId, tokens, cost, messages } = model;

  return (
    <ModelRowContainer>
      <TreeLineContainer style={{ color: "var(--color-fg-muted)" }}>
        <TreeLineBranch style={{ opacity: 0.2, borderColor: "var(--color-fg-muted)" }} />
        {!isLast && <TreeLineVertical style={{ opacity: 0.2, borderColor: "var(--color-fg-muted)" }} />}
      </TreeLineContainer>

      <ModelContent>
        <ModelHeader>
          <ModelName style={{ color: "var(--color-fg-default)" }}>{modelId}</ModelName>
          <ModelCost style={{ color: palette.grade1 }}>{formatCurrency(cost)}</ModelCost>
        </ModelHeader>

        <TokenBadgesGrid>
          {tokens.input > 0 && <TokenBadge label="Input" value={tokens.input} />}
          {tokens.output > 0 && <TokenBadge label="Output" value={tokens.output} />}
          {tokens.cacheRead > 0 && <TokenBadge label="Cache Read" value={tokens.cacheRead} />}
          {tokens.cacheWrite > 0 && <TokenBadge label="Cache Write" value={tokens.cacheWrite} />}
          {tokens.reasoning > 0 && <TokenBadge label="Reasoning" value={tokens.reasoning} />}
        </TokenBadgesGrid>

        <MessageCount style={{ color: "var(--color-fg-muted)" }}>
          {messages.toLocaleString()} message{messages !== 1 ? "s" : ""}
        </MessageCount>
      </ModelContent>
    </ModelRowContainer>
  );
}

const BadgeContainer = styled.div`
  display: flex;
  align-items: center;
  gap: 0.375rem;
`;

const BadgeLabel = styled.span`
  font-weight: 500;
`;

const BadgeValue = styled.span`
  font-family: var(--font-mono);
  font-weight: 600;
`;

function TokenBadge({ label, value }: { label: string; value: number }) {
  return (
    <BadgeContainer>
      <BadgeLabel style={{ color: "var(--color-fg-muted)" }}>{label}:</BadgeLabel>
      <BadgeValue style={{ color: "var(--color-fg-default)" }}>{formatTokenCount(value)}</BadgeValue>
    </BadgeContainer>
  );
}
