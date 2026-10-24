"use client";

import Link from "next/link";
import styled from "styled-components";
import type { Coverage, ModelUsageRow } from "@/lib/explore";
import { ModelMark } from "@/components/explore/ModelMark";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { agentLogoUrl } from "./agents";
import {
  compactCost,
  compactNumber,
  percent,
  periodLabel,
  userChange,
  wholeNumber,
} from "./format";
import { Arrow, Container, Eyebrow, Lead, Section, SectionHeader, SectionTitle, TextLink, VisuallyHidden } from "./primitives";

interface ModelsSectionProps {
  models: ModelUsageRow[];
  coverage: Coverage | null;
}

export function ModelsSection({ models, coverage }: ModelsSectionProps) {
  const pair = models.slice(0, 2);

  return (
    <Section aria-labelledby="home-models-title">
      <Container>
        <Head>
          <SectionHeader>
            <Eyebrow>Models</Eyebrow>
            <SectionTitle id="home-models-title">Compare models before you build.</SectionTitle>
            <Lead>
              See how many people use a model, which agents they use it through, and how many tokens it handles.
            </Lead>
            <TextLink href="/models">
              Compare models <Arrow />
            </TextLink>
          </SectionHeader>
        </Head>

        {pair.length > 0 && coverage ? (
          <Comparison>
            <Pair>
              {pair.map((model) => (
                <ModelColumn
                  key={model.key}
                  model={model}
                  accounts={coverage.accounts}
                  period={periodLabel(coverage.period).toLowerCase()}
                />
              ))}
            </Pair>
          </Comparison>
        ) : null}
      </Container>
    </Section>
  );
}

function ModelColumn({ model, accounts, period }: { model: ModelUsageRow; accounts: number; period: string }) {
  const change = userChange(model);
  const titleId = `home-model-${model.key.replace(/[^a-z0-9-]/gi, "-")}`;
  return (
    <Column aria-labelledby={titleId}>
      <ModelHead href={`/models/${encodeURIComponent(model.key)}`}>
        <MarkSlot>
          <ModelMark model={model} size={40} />
        </MarkSlot>
        <ModelName id={titleId}>
          {model.name}{" "}
          <Rank content={`#${model.rank} by active users, ${period}`} describe={false} openWithParentFocus>
            #{model.rank}
            <VisuallyHidden> by active users, {period}</VisuallyHidden>
          </Rank>
        </ModelName>
        <ModelLab>{model.lab ?? "Unknown lab"}</ModelLab>
      </ModelHead>
      <Facts>
        <Fact>
          <dt>
            Active users
            <small>People who used it in the period</small>
          </dt>
          <dd>
            {wholeNumber(model.activeUsers)}
            {change.kind === "new" ? (
              <Delta>New this period</Delta>
            ) : change.kind === "none" ? null : (
              <Delta>{change.label} vs previous period</Delta>
            )}
          </dd>
        </Fact>
        <Fact>
          <dt>
            Adoption
            <small>Share of all {wholeNumber(accounts)} active accounts</small>
          </dt>
          <dd>{percent(model.adoption)}</dd>
        </Fact>
        <Fact>
          <dt>
            Tokens
            <small>Input, output, cache, and reasoning tokens</small>
          </dt>
          <dd>{compactNumber(model.tokens)}</dd>
        </Fact>
        <Fact>
          <dt>
            Est. cost
            <small>At public list prices, not what people paid</small>
          </dt>
          <dd>{compactCost(model.estimatedCost, model.tokens)}</dd>
        </Fact>
        <Fact $stack>
          <dt>
            Top agents
            <small>Share of this model&apos;s tokens</small>
          </dt>
          <dd>
            {model.topAgents.length ? (
              <AgentList>
                {model.topAgents.map((agent) => {
                  const logo = agentLogoUrl(agent);
                  return (
                    <li key={agent.key}>
                      {logo ? <AgentLogo src={logo} alt="" width={16} height={16} loading="lazy" /> : <i aria-hidden="true" />}
                      <span>{agent.name}</span>
                      <b>{percent(agent.share)}</b>
                    </li>
                  );
                })}
              </AgentList>
            ) : (
              "—"
            )}
          </dd>
        </Fact>
      </Facts>
    </Column>
  );
}

const Head = styled.div`
  margin-bottom: 40px;
`;

/* Clips the header rows' hover background to the rounded corners. */
const Comparison = styled.div`
  overflow: hidden;
  border: 1px solid var(--v2-rule-strong);
  border-radius: 12px;
  background: var(--v2-surface);
`;

const Pair = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);

  @media (max-width: 820px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const Column = styled.section`
  display: flex;
  flex-direction: column;
  min-width: 0;

  & + & {
    border-left: 1px solid var(--v2-rule);
  }

  @media (max-width: 820px) {
    & + & {
      border-top: 1px solid var(--v2-rule);
      border-left: 0;
    }
  }
`;

/* The header row is the link to the model page. */
const ModelHead = styled(Link)`
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  column-gap: 12px;
  padding: 24px 24px 16px;
  color: inherit;
  text-decoration: none;
  transition: background-color 120ms ease;

  &:hover {
    background: rgba(255, 255, 255, 0.04);
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: -2px;
  }

  @media (max-width: 520px) {
    padding: 20px 16px 12px;
  }
`

const MarkSlot = styled.span`
  display: flex;
  grid-row: span 2;
  align-self: center;
`;

const ModelName = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-family: var(--v2-font-display);
  font-size: 34px;
  font-weight: 400;
  line-height: 1.1;
`;

/* Rank by active users in the period, as on the featured human; the tooltip spells it out. */
const Rank = styled(InstantTooltip)`
  color: var(--v2-accent);
  white-space: nowrap;
`;

const ModelLab = styled.p`
  margin: 2px 0 0;
  color: var(--v2-text-muted);
  font-size: 14px;
`;

const Facts = styled.dl`
  flex: 1;
  margin: 0;
  padding: 0 24px;

  @media (max-width: 520px) {
    padding: 0 16px;
  }
`;

const Fact = styled.div<{ $stack?: boolean }>`
  display: grid;
  grid-template-columns: ${({ $stack }) => ($stack ? "minmax(0, 1fr)" : "minmax(0, 1fr) auto")};
  gap: ${({ $stack }) => ($stack ? "10px" : "16px")};
  align-items: baseline;
  padding: 14px 0;
  border-top: 1px solid var(--v2-rule);

  dt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    color: var(--v2-text);
    font-size: 14px;
    font-weight: 500;
  }

  dt small {
    color: var(--v2-text-subtle);
    font-size: 12px;
    font-weight: 400;
  }

  dd {
    display: flex;
    flex-direction: column;
    align-items: ${({ $stack }) => ($stack ? "stretch" : "flex-end")};
    margin: 0;
    color: var(--v2-text);
    font-size: 24px;
    font-weight: 500;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }
`;

const Delta = styled.span`
  color: var(--v2-numeric);
  font-size: 12px;
  font-weight: 500;
  letter-spacing: 0;
`;

const AgentList = styled.ul`
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: 14px;
  letter-spacing: 0;

  li {
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
  }

  span {
    grid-column: 2;
    color: var(--v2-text-muted);
    font-weight: 400;
  }

  b {
    color: var(--v2-text);
    font-weight: 500;
  }
`;

const AgentLogo = styled.img`
  grid-column: 1;
  width: 16px;
  height: 16px;
  border-radius: 3px;
  object-fit: contain;
`;
