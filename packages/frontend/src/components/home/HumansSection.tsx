"use client";

import { useId, useState } from "react";
import styled from "styled-components";
import { DitherAvatar } from "dither-kit/DitherAvatar";
import { DitherImage } from "dither-kit/DitherImage";
import { DitherReveal } from "dither-kit/DitherReveal";
import type { Coverage, HumanRow } from "@/lib/explore";
import { ModelMark } from "@/components/explore/ModelMark";
import { agentLogoUrl } from "./agents";
import { compactNumber, coverageDays, coverageRangeLabel, periodLabel, uniqueByKey, wholeNumber } from "./format";
import { Arrow, Container, Lead, Section, SectionTitle, TextLink, VisuallyHidden, focusRing } from "./primitives";
import { Reveal } from "./Reveal";

interface HumansSectionProps {
  humans: HumanRow[];
  coverage: Coverage | null;
}

/**
 * The people on Tokscale. A name is chosen from the list; the square dither
 * field beside it is seeded by that name and crossfades to the next person's,
 * with the real GitHub avatar standing in front of it.
 */
export function HumansSection({ humans, coverage }: HumansSectionProps) {
  const [active, setActive] = useState(0);
  const listId = useId();
  const rows = humans.slice(0, 5);
  const current = rows[Math.min(active, rows.length - 1)] ?? null;

  return (
    <Section aria-labelledby="home-humans-title">
      <Container>
        <Layout>
          <Copy>
            <Reveal>
              <SectionTitle id="home-humans-title">Find people building with the tools you use.</SectionTitle>
            </Reveal>
            <Reveal delay={70}>
              <Lead>
                Every public profile shows which models and agents a person uses, how many tokens they run, and how
                often they work with AI. Profiles come from usage people chose to submit.
              </Lead>
            </Reveal>
            {rows.length > 0 ? (
              <Reveal delay={140}>
                <People id={listId} aria-label="Top builders by tokens, last 30 days">
                  {rows.map((human, index) => (
                    <li key={human.username}>
                      <NameButton
                        type="button"
                        $active={index === Math.min(active, rows.length - 1)}
                        aria-current={index === Math.min(active, rows.length - 1) ? "true" : undefined}
                        onClick={() => setActive(index)}
                      >
                        <NameText>{human.displayName || human.username}</NameText>
                        <NameValue>{compactNumber(human.tokens)}</NameValue>
                      </NameButton>
                    </li>
                  ))}
                </People>
              </Reveal>
            ) : null}
            <Reveal delay={210}>
              <TextLink href="/leaderboard">
                Browse the humans leaderboard <Arrow />
              </TextLink>
            </Reveal>
          </Copy>

          {current ? <Figure human={current} coverage={coverage} /> : null}
        </Layout>
      </Container>
    </Section>
  );
}

function Figure({ human, coverage }: { human: HumanRow; coverage: Coverage | null }) {
  const name = human.displayName || human.username;
  const days = coverage ? coverageDays(coverage) : null;
  const period = coverage ? periodLabel(coverage.period).toLowerCase() : "last 30 days";
  const range = coverage ? coverageRangeLabel(coverage) : null;
  const profileHref = `/u/${encodeURIComponent(human.username)}`;
  const stack = uniqueByKey(human.topModels).slice(0, 3);
  // The portrait is the person, dithered into the same square cells as the art.
  // GitHub's avatar host allows canvas reads; any other host keeps the seeded
  // pattern rather than risking a blank canvas.
  const portrait = human.avatarUrl || `https://github.com/${encodeURIComponent(human.username)}.png?size=256`;
  const canDither = /^https?:\/\/(avatars\.githubusercontent\.com|github\.com)\//i.test(portrait);

  return (
    <Stage>
      <Reveal delay={80}>
        <FieldWrap>
          <DitherReveal key={human.username} direction="down" cell={10} duration={0.9}>
            {canDither ? (
              <DitherImage key={human.username} src={portrait} cell={3} renderMode="live" />
            ) : (
              <DitherAvatar name={name} size={undefined} grid={42} color="grey" animate={false} />
            )}
          </DitherReveal>
        </FieldWrap>
      </Reveal>
      <Fact>
        <FactName href={profileHref}>
          {name} #{human.rank}
          <VisuallyHidden> by tokens, {period}</VisuallyHidden>
        </FactName>
        <FactLine>
          {compactNumber(human.tokens)} tokens · {wholeNumber(human.activeDays)}
          {days ? ` of ${days}` : ""} days active
          {range ? ` · ${range}` : ""}
        </FactLine>
        {stack.length > 0 ? (
          <Stack>
            <VisuallyHidden>Top models: </VisuallyHidden>
            {stack.map((model) => (
              <li key={model.key}>
                <ModelMark model={model} size={16} />
                {model.name}
              </li>
            ))}
            {uniqueByKey(human.topAgents).slice(0, 2).map((agent) => {
              const logo = agentLogoUrl(agent);
              return (
                <li key={agent.key}>
                  {logo ? <AgentLogo src={logo} alt="" width={16} height={16} loading="lazy" /> : null}
                  {agent.name}
                </li>
              );
            })}
          </Stack>
        ) : null}
      </Fact>
    </Stage>
  );
}

const Layout = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
  gap: 64px;
  align-items: center;

  @media (max-width: 960px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 40px;
  }
`;

const Copy = styled.div`
  display: flex;
  flex-direction: column;
  gap: 20px;
  min-width: 0;
`;

const People = styled.ul`
  display: flex;
  flex-direction: column;
  margin: 4px 0 0;
  padding: 0;
  list-style: none;
`;

const NameButton = styled.button<{ $active: boolean }>`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  width: 100%;
  min-height: 44px;
  padding: 6px 0;
  border: 0;
  background: none;
  color: ${({ $active }) => ($active ? "var(--v2-text)" : "var(--v2-text-muted)")};
  text-align: left;
  cursor: pointer;
  ${focusRing}

  @media (hover: hover) {
    &:hover {
      color: var(--v2-text);
    }
  }
`;

const NameText = styled.span`
  font-size: clamp(26px, 2.9vw, 40px);
  font-weight: 500;
  line-height: 1.12;
  letter-spacing: -0.02em;
`;

const NameValue = styled.span`
  flex-shrink: 0;
  color: var(--v2-text-subtle);
  font-size: 15px;
  font-variant-numeric: tabular-nums;
`;

const Stage = styled.div`
  display: flex;
  flex-direction: column;
  gap: 20px;
  min-width: 0;
`;

/* The square dither field, one instrument scale up: the motif as a portrait. */
const FieldWrap = styled.div`
  position: relative;
  display: grid;
  aspect-ratio: 1;
  width: 100%;
  max-width: 520px;
  place-items: center;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);

  > div,
  > div > div {
    position: absolute;
    inset: 0;
  }

  canvas {
    width: 100%;
    height: 100%;
  }
`;

const Fact = styled.div`
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
`;

const FactName = styled.a`
  align-self: flex-start;
  border-radius: var(--radius, 2px);
  color: var(--v2-text);
  font-size: 17px;
  font-weight: 600;
  text-decoration: underline;
  text-decoration-color: transparent;
  text-underline-offset: 0.18em;
  transition: text-decoration-color 160ms ease;
  ${focusRing}

  @media (hover: hover) {
    &:hover {
      text-decoration-color: currentColor;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const FactLine = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 15px;
  font-variant-numeric: tabular-nums;
`;

const Stack = styled.ul`
  display: flex;
  flex-wrap: wrap;
  gap: 6px 18px;
  margin: 4px 0 0;
  padding: 0;
  color: var(--v2-text-muted);
  font-size: 15px;
  list-style: none;

  li {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
`;

const AgentLogo = styled.img`
  flex-shrink: 0;
  border-radius: var(--radius, 2px);
`;
