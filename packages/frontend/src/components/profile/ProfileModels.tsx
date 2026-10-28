"use client";

import styled from "styled-components";
import { formatCurrency, formatNumber } from "@/lib/utils";
import { DitherMiniBar } from "./ditherControls";
import {
  ListBody,
  ListCaption,
  ListCard,
  ListCell,
  ListHead,
  ListHeaderCell,
  ListPrimaryCell,
  ListRow,
  ListTable,
  NumericValue,
} from "./listStyles";
import type { ModelUsage } from "./types";

export interface ProfileModelsProps {
  models: string[];
  modelUsage?: ModelUsage[];
  className?: string;
}

/** Ink for the leading model's share bar; grey for every other row. */
const LEAD_BAR_COLOR = "#f4f7fb";
const REST_BAR_COLOR = "#8a8c92";

const ModelsSection = styled.section`
  display: grid;
  min-width: 0;
  gap: 0.5rem;
`;

const SectionHeader = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.25rem 1rem;
`;

const SectionTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  letter-spacing: -0.01em;
  line-height: 1.25;
`;

const SectionNote = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.4;
`;

const ModelName = styled.span`
  display: block;
  overflow: hidden;
  color: var(--service-text);
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 639px) {
    overflow-wrap: anywhere;
    white-space: normal;
  }
`;

const ShareCell = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 0.5rem;
`;

const ModelsFallbackList = styled.ul`
  display: flex;
  flex-wrap: wrap;
  gap: 0.375rem;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const ModelTag = styled.li`
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 0.375rem;
  border: 1px solid var(--service-border);
  border-radius: var(--radius);
  padding: 0.3rem 0.5rem;
  color: var(--service-text);
  font-size: 0.75rem;
  line-height: 1.2;
`;

const FallbackCard = styled(ListCard)`
  padding: 0.75rem;
`;

export function ProfileModels({
  models,
  modelUsage,
  className,
}: ProfileModelsProps) {
  const filteredUsage = (modelUsage ?? [])
    .filter((usage) => usage.model !== "<synthetic>")
    .sort((a, b) => b.cost - a.cost);
  const filteredModels = Array.from(
    new Set(models.filter((model) => model !== "<synthetic>")),
  );

  if (filteredUsage.length > 0) {
    return (
      <ModelsSection
        className={className}
        aria-labelledby="profile-models-heading"
      >
        <SectionHeader>
          <SectionTitle id="profile-models-heading">Models</SectionTitle>
          <SectionNote>Tokens, cost and share by model, all-time.</SectionNote>
        </SectionHeader>
        <ListCard>
          <ListTable>
            <ListCaption>Model usage</ListCaption>
            <ListHead>
              <tr>
                <ListHeaderCell $width="52%">Model</ListHeaderCell>
                <ListHeaderCell $width="18%" $align="right">
                  Tokens
                </ListHeaderCell>
                <ListHeaderCell $width="17%" $align="right">
                  Cost
                </ListHeaderCell>
                <ListHeaderCell $width="13%" $align="right">
                  Share
                </ListHeaderCell>
              </tr>
            </ListHead>
            <ListBody>
              {filteredUsage.map((usage, index) => (
                <ListRow key={usage.model}>
                  <ListPrimaryCell scope="row">
                    <ModelName>{usage.model}</ModelName>
                  </ListPrimaryCell>
                  <ListCell data-label="Tokens" $align="right">
                    <NumericValue title={usage.tokens.toLocaleString("en-US")}>
                      {formatNumber(usage.tokens)}
                    </NumericValue>
                  </ListCell>
                  <ListCell data-label="Cost" $align="right">
                    <NumericValue $accent>
                      {formatCurrency(usage.cost)}
                    </NumericValue>
                  </ListCell>
                  <ListCell data-label="Share" $align="right">
                    <ShareCell>
                      <DitherMiniBar
                        ratio={Math.max(
                          0,
                          Math.min(1, usage.percentage / 100),
                        )}
                        color={index === 0 ? LEAD_BAR_COLOR : REST_BAR_COLOR}
                        width="4rem"
                      />
                      <NumericValue>
                        {usage.percentage.toFixed(1)}%
                      </NumericValue>
                    </ShareCell>
                  </ListCell>
                </ListRow>
              ))}
            </ListBody>
          </ListTable>
        </ListCard>
      </ModelsSection>
    );
  }

  if (filteredModels.length === 0) return null;

  return (
    <ModelsSection
      className={className}
      aria-labelledby="profile-models-heading"
    >
      <SectionHeader>
        <SectionTitle id="profile-models-heading">Models</SectionTitle>
        <SectionNote>
          Recorded by earlier submissions, without per-model totals.
        </SectionNote>
      </SectionHeader>
      <FallbackCard>
        <ModelsFallbackList aria-label="Models used">
          {filteredModels.map((model) => (
            <ModelTag key={model}>
              <ModelName>{model}</ModelName>
            </ModelTag>
          ))}
        </ModelsFallbackList>
      </FallbackCard>
    </ModelsSection>
  );
}
