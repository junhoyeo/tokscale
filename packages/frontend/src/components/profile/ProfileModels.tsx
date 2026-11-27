"use client";

import { useMemo, useState } from "react";
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
import {
  DEFAULT_MODEL_RANK_BY,
  rankModelUsage,
  type ModelRankBy,
} from "./modelRanking";
import { ModelRankToggle } from "./ModelRankToggle";
import type { ModelUsage } from "./types";

export interface ProfileModelsProps {
  models: string[];
  modelUsage?: ModelUsage[];
  rankBy?: ModelRankBy;
  onRankByChange?: (next: ModelRankBy) => void;
  className?: string;
}

const ModelsSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
`;

const ModelsToolbar = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem 1rem;
`;

const ModelsSummary = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
`;

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
  rankBy,
  onRankByChange,
  className,
}: ProfileModelsProps) {
  const [uncontrolledRankBy, setUncontrolledRankBy] = useState<ModelRankBy>(
    DEFAULT_MODEL_RANK_BY,
  );
  const effectiveRankBy = rankBy ?? uncontrolledRankBy;
  const handleRankByChange = (next: ModelRankBy) => {
    if (onRankByChange) {
      onRankByChange(next);
    } else {
      setUncontrolledRankBy(next);
    }
  };

  const rankedUsage = useMemo(
    () => rankModelUsage(modelUsage, effectiveRankBy),
    [modelUsage, effectiveRankBy],
  );
  const filteredModels = Array.from(
    new Set(models.filter((model) => model !== "<synthetic>")),
  );

  if (rankedUsage.length > 0) {
    const shareTitle =
      effectiveRankBy === "tokens" ? "Share of total tokens" : "Share of total cost";

    return (
      <ModelsSection className={className}>
        <ModelsToolbar>
          <ModelsSummary>
            {rankedUsage.length.toLocaleString("en-US")}{" "}
            {rankedUsage.length === 1 ? "model" : "models"} · ranked by{" "}
            {effectiveRankBy === "tokens" ? "tokens" : "cost"}
          </ModelsSummary>
          <ModelRankToggle
            label="Rank by"
            ariaLabel="Rank models by"
            value={effectiveRankBy}
            onChange={handleRankByChange}
          />
        </ModelsToolbar>

        <ListCard>
          <ListTable>
            <ListCaption>
              Model usage ranked by{" "}
              {effectiveRankBy === "tokens" ? "tokens" : "cost"}
            </ListCaption>
            <ListHead>
              <tr>
                <ListHeaderCell $width="52%">Model</ListHeaderCell>
                <ListHeaderCell
                  $width="18%"
                  $align="right"
                  aria-sort={
                    effectiveRankBy === "tokens" ? "descending" : "none"
                  }
                >
                  Tokens
                </ListHeaderCell>
                <ListHeaderCell
                  $width="17%"
                  $align="right"
                  aria-sort={effectiveRankBy === "cost" ? "descending" : "none"}
                >
                  Cost
                </ListHeaderCell>
                <ListHeaderCell $width="13%" $align="right" title={shareTitle}>
                  Share
                </ListHeaderCell>
              </tr>
            </ListHead>
            <ListBody>
              {rankedUsage.map((usage) => {
                const formattedShare =
                  usage.share != null ? `${usage.share.toFixed(1)}%` : "0.0%";
                return (
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
                      <NumericValue
                        $accent={effectiveRankBy === "tokens"}
                        title={usage.tokens.toLocaleString("en-US")}
                      >
                        {formatNumber(usage.tokens)}
                      </NumericValue>
                    </ListCell>
                    <ListCell data-label="Cost" $align="right">
                      <NumericValue
                        $accent={effectiveRankBy === "cost"}
                        title={usage.cost.toLocaleString("en-US", {
                          style: "currency",
                          currency: "USD",
                        })}
                      >
                        {formatCurrency(usage.cost)}
                      </NumericValue>
                    </ListCell>
                    <ListCell data-label="Share" $align="right">
                      <NumericValue title={shareTitle}>
                        {formattedShare}
                      </NumericValue>
                    </ListCell>
                  </ListRow>
                );
              })}
            </ListBody>
          </ListTable>
        </ListCard>
      </ModelsSection>
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
