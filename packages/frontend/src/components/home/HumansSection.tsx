"use client";

import Link from "next/link";
import styled from "styled-components";
import type { Coverage, HumanRow } from "@/lib/explore";
import { ModelMark } from "@/components/explore/ModelMark";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { agentLogoUrl } from "./agents";
import {
  compactCost,
  compactNumber,
  coverageDays,
  coverageRangeLabel,
  periodLabel,
  uniqueByKey,
  wholeNumber,
} from "./format";
import { Arrow, Container, Eyebrow, Lead, Section, SectionHeader, SectionTitle, TextLink, VisuallyHidden } from "./primitives";

interface HumansSectionProps {
  human: HumanRow | null;
  coverage: Coverage | null;
}

export function HumansSection({ human, coverage }: HumansSectionProps) {
  return (
    <Section aria-labelledby="home-humans-title">
      <Container>
        <Layout>
          <SectionHeader>
            <Eyebrow>Humans</Eyebrow>
            <SectionTitle id="home-humans-title">Find people building with the tools you use.</SectionTitle>
            <Lead>
              Every public profile shows which models and agents a person uses, how many tokens they run, and how
              often they work with AI. Profiles come from usage people chose to submit.
            </Lead>
            <TextLink href="/leaderboard">
              Browse the humans leaderboard <Arrow />
            </TextLink>
          </SectionHeader>

          {human && coverage ? <FeaturedProfile human={human} coverage={coverage} /> : null}
        </Layout>
      </Container>
    </Section>
  );
}

function FeaturedProfile({ human, coverage }: { human: HumanRow; coverage: Coverage }) {
  const name = human.displayName || human.username;
  const range = coverageRangeLabel(coverage);
  const days = coverageDays(coverage);
  const profileHref = `/u/${encodeURIComponent(human.username)}`;
  const rankBasis = `by tokens, ${periodLabel(coverage.period).toLowerCase()}`;

  return (
    <Card aria-labelledby="featured-human-name">
      <Identity href={profileHref}>
        <Avatar
          src={human.avatarUrl || `https://github.com/${encodeURIComponent(human.username)}.png?size=128`}
          alt=""
          width={56}
          height={56}
          loading="lazy"
        />
        <div>
          <Name id="featured-human-name">
            {name}{" "}
            <Rank content={`#${human.rank} ${rankBasis}`} describe={false} openWithParentFocus>
              #{human.rank}
              <VisuallyHidden> {rankBasis}</VisuallyHidden>
            </Rank>
          </Name>
          <Handle>@{human.username}</Handle>
        </div>
      </Identity>

      <Block>
        <BlockLabel>
          Usage · {periodLabel(coverage.period)}
          {range ? <span> ({range})</span> : null}
        </BlockLabel>
        <Facts>
          <div>
            <dt>Tokens</dt>
            <dd>{compactNumber(human.tokens)}</dd>
          </div>
          <div>
            <dt>Est. cost</dt>
            <dd>{compactCost(human.estimatedCost, human.tokens)}</dd>
          </div>
          <div>
            <dt>Active days</dt>
            <dd>
              {wholeNumber(human.activeDays)}
              {days ? <small> of {days}</small> : null}
            </dd>
          </div>
        </Facts>
      </Block>

      <Block>
        <BlockLabel>Stack, by tokens in the period</BlockLabel>
        <StackList>
          <div>
            <dt>Models</dt>
            <dd>
              {uniqueByKey(human.topModels).length ? (
                <Chips>
                  {uniqueByKey(human.topModels).map((model) => (
                    <li key={model.key}>
                      <ModelMark model={model} size={16} />
                      {model.name}
                      {model.lab ? <span>{model.lab}</span> : null}
                    </li>
                  ))}
                </Chips>
              ) : (
                "—"
              )}
            </dd>
          </div>
          <div>
            <dt>Agents</dt>
            <dd>
              {uniqueByKey(human.topAgents).length ? (
                <Chips>
                  {uniqueByKey(human.topAgents).map((agent) => {
                    const logo = agentLogoUrl(agent);
                    return (
                      <li key={agent.key}>
                        {logo ? <ChipLogo src={logo} alt="" width={16} height={16} loading="lazy" /> : null}
                        {agent.name}
                      </li>
                    );
                  })}
                </Chips>
              ) : (
                "—"
              )}
            </dd>
          </div>
        </StackList>
      </Block>

    </Card>
  );
}

const Layout = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
  gap: 64px;
  align-items: start;

  @media (max-width: 960px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 40px;
  }
`;

const Card = styled.article`
  border: 1px solid var(--v2-rule-strong);
  border-radius: 12px;
  background: var(--v2-surface);
`;

/* The header row is the link to the profile. */
const Identity = styled(Link)`
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 20px 24px;
  border-radius: 11px 11px 0 0;
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
    padding: 16px;
  }
`

const Avatar = styled.img`
  flex-shrink: 0;
  width: 56px;
  height: 56px;
  border-radius: 50%;
  background: var(--v2-surface-raised);
  box-shadow: 0 0 0 1px var(--v2-rule-strong);
`;

const Name = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-family: var(--v2-font-display);
  font-size: 30px;
  font-weight: 400;
  line-height: 1.1;
  overflow-wrap: anywhere;
`;

/* Rank by tokens in the period; the tooltip spells it out. */
const Rank = styled(InstantTooltip)`
  color: var(--v2-accent);
  white-space: nowrap;
`;

const Handle = styled.p`
  margin: 2px 0 0;
  color: var(--v2-text-muted);
  font-size: 14px;
  overflow-wrap: anywhere;
`;

const Block = styled.div`
  padding: 16px 24px 20px;
  border-top: 1px solid var(--v2-rule);

  @media (max-width: 520px) {
    padding: 16px;
  }
`;

const BlockLabel = styled.p`
  margin: 0 0 12px;
  color: var(--v2-text-subtle);
  font-size: 13px;
  font-weight: 500;

  span {
    font-weight: 400;
  }
`;

const Facts = styled.dl`
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 16px;
  margin: 0;

  dt {
    color: var(--v2-text-muted);
    font-size: 13px;
  }

  dd {
    margin: 4px 0 0;
    color: var(--v2-text);
    font-size: 28px;
    font-weight: 500;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }

  small {
    color: var(--v2-text-subtle);
    font-size: 14px;
    font-weight: 400;
    letter-spacing: 0;
  }

  @media (max-width: 400px) {
    dd {
      font-size: 22px;
    }
  }
`;

const StackList = styled.dl`
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin: 0;

  > div {
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr);
    gap: 12px;
    align-items: baseline;
  }

  dt {
    color: var(--v2-text-muted);
    font-size: 13px;
  }

  dd {
    margin: 0;
    min-width: 0;
  }

  @media (max-width: 400px) {
    > div {
      grid-template-columns: minmax(0, 1fr);
      gap: 6px;
    }
  }
`;

const Chips = styled.ul`
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;

  li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 28px;
    padding: 0 10px;
    border: 1px solid var(--v2-rule-strong);
    border-radius: 999px;
    color: var(--v2-text);
    font-size: 13px;
  }

  li span {
    color: var(--v2-text-subtle);
  }

`;

const ChipLogo = styled.img`
  width: 16px;
  height: 16px;
  border-radius: 3px;
  object-fit: contain;
`;
