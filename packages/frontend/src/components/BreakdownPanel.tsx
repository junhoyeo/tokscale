"use client";

import styled from "styled-components";
import type { DailyContribution, ClientType } from "@/lib/types";
import { formatCurrency, formatTokenCount, groupClientsByType, sortClientsByCost } from "@/lib/utils";
import { formatContributionDateFull } from "@/lib/date-utils";
import { SOURCE_DISPLAY_NAMES, SOURCE_COLORS, SOURCE_LOGOS } from "@/lib/constants";
import { SourceLogo } from "./SourceLogo";
import { Eyebrow as SharedEyebrow } from "@/components/ui/Eyebrow";

interface BreakdownPanelProps {
  day: DailyContribution | null;
  onClose: () => void;
}

const PanelContainer = styled.section`
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
  font-variant-numeric: tabular-nums;
`;

const PanelHeader = styled.header`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px 14px 20px;
  border-bottom: 1px solid var(--service-border);

  @media (max-width: 520px) {
    padding: 12px 10px 12px 14px;
  }
`;

const PanelHeading = styled.div`
  min-width: 0;
`;

/* The shared eyebrow (Title Case, no letter-spacing), tucked close to the title. */
const Eyebrow = styled(SharedEyebrow)`
  margin-bottom: 2px;
`;

const PanelTitle = styled.h3`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const CloseButton = styled.button`
  display: inline-grid;
  flex: 0 0 auto;
  width: 32px;
  height: 32px;
  place-items: center;
  border: 1px solid transparent;
  border-radius: 8px;
  color: var(--service-text-muted);

  &:hover {
    border-color: var(--service-border);
    background: var(--service-surface-muted);
    color: var(--service-text);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    width: 44px;
    height: 44px;
  }
`;

const PanelContent = styled.div`
  padding: 16px 20px 20px;

  @media (max-width: 520px) {
    padding: 14px;
  }
`;

const EmptyState = styled.p`
  margin: 0;
  padding: 24px 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  text-align: center;
`;

const SourceList = styled.div`
  display: grid;
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 10px;
`;

const SummaryFooter = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 4px 20px;
  margin-top: 16px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const SummaryItem = styled.div`
  font-weight: 500;
`;

const SummaryValue = styled.span`
  color: var(--service-text);
  font-weight: 600;
`;

const SummaryTotalValue = styled(SummaryValue)`
  color: var(--v2-numeric);
  font-size: 1rem;
`;

export function BreakdownPanel({ day, onClose }: BreakdownPanelProps) {
  if (!day) return null;

  const groupedClients = groupClientsByType(day.clients);
  const sortedClientTypes = Array.from(groupedClients.keys()).sort();

  return (
    <PanelContainer role="region" aria-label="Day breakdown">
      <PanelHeader>
        <PanelHeading>
          <Eyebrow as="div">Detailed Breakdown</Eyebrow>
          <PanelTitle>{formatContributionDateFull(day)}</PanelTitle>
        </PanelHeading>
        <CloseButton type="button" onClick={onClose} aria-label="Close breakdown panel">
          <svg aria-hidden="true" fill="none" height="14" viewBox="0 0 14 14" width="14">
            <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" strokeLinecap="round" strokeWidth="1.5" />
          </svg>
        </CloseButton>
      </PanelHeader>

      <PanelContent>
        {day.clients.length === 0 ? (
          <EmptyState>No activity on this day</EmptyState>
        ) : (
          <SourceList>
            {sortedClientTypes.map((clientType) => {
              const clients = sortClientsByCost(groupedClients.get(clientType) || []);
              const clientTotalCost = clients.reduce((sum, c) => sum + c.cost, 0);
              return <ClientSection key={clientType} clientType={clientType} clients={clients} totalCost={clientTotalCost} />;
            })}
          </SourceList>
        )}

        {day.clients.length > 0 && (
          <SummaryFooter>
            <SummaryItem>
              Total: <SummaryTotalValue>{formatCurrency(day.totals.cost)}</SummaryTotalValue>
            </SummaryItem>
            <SummaryItem>
              across <SummaryValue>{sortedClientTypes.length} client{sortedClientTypes.length !== 1 ? "s" : ""}</SummaryValue>
            </SummaryItem>
            <SummaryItem>
              <SummaryValue>
                {(() => {
                  const allModels = new Set<string>();
                  for (const c of day.clients) {
                    if (c.models) {
                      for (const modelId of Object.keys(c.models)) {
                        allModels.add(modelId);
                      }
                    } else if (c.modelId) {
                      allModels.add(c.modelId);
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

interface ClientSectionProps {
  clientType: ClientType;
  clients: DailyContribution["clients"];
  totalCost: number;
}

const Section = styled.div`
  min-width: 0;
  padding: 14px 16px;

  & + & {
    border-top: 1px solid var(--service-border);
  }

  @media (max-width: 520px) {
    padding: 12px;
  }
`;

const SectionHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 10px;
`;

const ClientName = styled.span`
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 600;

  img {
    border-radius: 4px;
  }
`;

const ClientDot = styled.span<{ $color: string }>`
  flex: 0 0 auto;
  width: 8px;
  height: 8px;
  border: 1px solid rgba(255, 255, 255, 0.28);
  border-radius: 999px;
  background: ${({ $color }) => $color};
`;

const SectionTotal = styled.span`
  flex: 0 0 auto;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 600;
`;

const ModelsList = styled.div`
  display: grid;
  gap: 12px;
  margin-left: 4px;
`;

function ClientSection({ clientType, clients, totalCost }: ClientSectionProps) {
  const hasLogo = Object.prototype.hasOwnProperty.call(SOURCE_LOGOS, String(clientType).toLowerCase());

  const modelEntries: Array<{ modelId: string; cost: number; messages: number; tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; reasoning: number } }> = [];
  for (const client_contrib of clients) {
    if (client_contrib.models && Object.keys(client_contrib.models).length > 0) {
      for (const [modelId, data] of Object.entries(client_contrib.models)) {
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
    } else if (client_contrib.modelId) {
      modelEntries.push({
        modelId: client_contrib.modelId,
        cost: client_contrib.cost,
        messages: client_contrib.messages,
        tokens: client_contrib.tokens,
      });
    }
  }

  const sortedModels = modelEntries.sort((a, b) => b.cost - a.cost);

  return (
    <Section>
      <SectionHeader>
        <ClientName>
          {hasLogo ? (
            <SourceLogo sourceId={clientType} height={16} decorative />
          ) : (
            <ClientDot $color={SOURCE_COLORS[clientType] || "var(--service-text-muted)"} aria-hidden="true" />
          )}
          {SOURCE_DISPLAY_NAMES[clientType] || clientType}
        </ClientName>
        <SectionTotal>{formatCurrency(totalCost)}</SectionTotal>
      </SectionHeader>

      <ModelsList>
        {sortedModels.map((model, index) => (
          <ModelRow key={`${model.modelId}-${index}`} model={model} isLast={index === sortedModels.length - 1} />
        ))}
      </ModelsList>
    </Section>
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
}

const ModelRowContainer = styled.div`
  position: relative;
`;

const TreeLineContainer = styled.div`
  position: absolute;
  top: 0;
  left: 0;
  width: 1rem;
  height: 100%;
`;

const TreeLineBranch = styled.span`
  position: absolute;
  top: 0.75rem;
  left: 0;
  width: 0.75rem;
  border-top: 1px solid var(--service-border-strong);
`;

const TreeLineVertical = styled.span`
  position: absolute;
  top: 0;
  left: 0;
  height: calc(100% + 12px);
  border-left: 1px solid var(--service-border-strong);
`;

const ModelContent = styled.div`
  min-width: 0;
  margin-left: 1.5rem;
`;

const ModelHeader = styled.div`
  display: flex;
  min-width: 0;
  align-items: baseline;
  gap: 12px;
`;

const ModelName = styled.span`
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  color: var(--service-text);
  font-family: var(--v2-font-mono);
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const ModelCost = styled.span`
  flex: 0 0 auto;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 600;
`;

const TokenBadgesGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 6px 16px;
  margin-top: 8px;
  font-size: 0.75rem;

  @media (max-width: 400px) {
    grid-template-columns: minmax(0, 1fr);
  }

  @media (min-width: 640px) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  @media (min-width: 768px) {
    grid-template-columns: repeat(5, minmax(0, 1fr));
  }
`;

const MessageCount = styled.div`
  margin-top: 6px;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

function ModelRow({ model, isLast }: ModelRowProps) {
  const { modelId, tokens, cost, messages } = model;

  return (
    <ModelRowContainer>
      <TreeLineContainer aria-hidden="true">
        <TreeLineBranch />
        {!isLast && <TreeLineVertical />}
      </TreeLineContainer>

      <ModelContent>
        <ModelHeader>
          <ModelName title={modelId}>{modelId}</ModelName>
          <ModelCost>{formatCurrency(cost)}</ModelCost>
        </ModelHeader>

        <TokenBadgesGrid>
          {tokens.input > 0 && <TokenBadge label="Input" value={tokens.input} />}
          {tokens.output > 0 && <TokenBadge label="Output" value={tokens.output} />}
          {tokens.cacheRead > 0 && <TokenBadge label="Cache read" value={tokens.cacheRead} />}
          {tokens.cacheWrite > 0 && <TokenBadge label="Cache write" value={tokens.cacheWrite} />}
          {tokens.reasoning > 0 && <TokenBadge label="Reasoning" value={tokens.reasoning} />}
        </TokenBadgesGrid>

        <MessageCount>
          {messages.toLocaleString()} message{messages !== 1 ? "s" : ""}
        </MessageCount>
      </ModelContent>
    </ModelRowContainer>
  );
}

const BadgeContainer = styled.div`
  display: flex;
  min-width: 0;
  align-items: baseline;
  gap: 6px;
`;

const BadgeLabel = styled.span`
  color: var(--service-text-muted);
`;

const BadgeValue = styled.span`
  color: var(--service-text);
  font-weight: 600;
`;

function TokenBadge({ label, value }: { label: string; value: number }) {
  return (
    <BadgeContainer>
      <BadgeLabel>{label}:</BadgeLabel>
      <BadgeValue>{formatTokenCount(value)}</BadgeValue>
    </BadgeContainer>
  );
}
