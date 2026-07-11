"use client";

import styled from "styled-components";
import { formatCurrency, formatNumber } from "@/lib/utils";
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
import { getModelColor } from "./modelColors";
import type { ModelUsage } from "./types";

export interface ProfileModelsProps {
  models: string[];
  modelUsage?: ModelUsage[];
  className?: string;
}

const ModelIdentity = styled.span`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.5rem;
`;

const ModelMarker = styled.span`
  width: 8px;
  height: 8px;
  flex: 0 0 auto;
  border-radius: 50%;
`;

const ModelName = styled.span`
  overflow: hidden;
  color: var(--service-text);
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 639px) {
    overflow-wrap: anywhere;
    white-space: normal;
  }
`;

const ModelsFallback = styled(ListCard)`
  padding: 0.75rem;
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
  border-radius: 8px;
  padding: 0.35rem 0.5rem;
  background: var(--service-surface-muted);
  color: var(--service-text);
  font-size: 0.75rem;
  line-height: 1.2;
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
      <ListCard className={className}>
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
            {filteredUsage.map((usage) => (
              <ListRow key={usage.model}>
                <ListPrimaryCell scope="row">
                  <ModelIdentity>
                    <ModelMarker
                      aria-hidden="true"
                      style={{ backgroundColor: getModelColor(usage.model) }}
                    />
                    <ModelName>{usage.model}</ModelName>
                  </ModelIdentity>
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
                  <NumericValue>{usage.percentage.toFixed(1)}%</NumericValue>
                </ListCell>
              </ListRow>
            ))}
          </ListBody>
        </ListTable>
      </ListCard>
    );
  }

  if (filteredModels.length === 0) return null;

  return (
    <ModelsFallback className={className}>
      <ModelsFallbackList aria-label="Models used">
        {filteredModels.map((model) => (
          <ModelTag key={model}>
            <ModelMarker
              aria-hidden="true"
              style={{ backgroundColor: getModelColor(model) }}
            />
            <ModelName>{model}</ModelName>
          </ModelTag>
        ))}
      </ModelsFallbackList>
    </ModelsFallback>
  );
}
