"use client";

import styled from "styled-components";
import { DitherGradient } from "dither-kit/DitherGradient";
import type { CommunitySnapshot as Snapshot, Coverage, HumanRow, ModelUsageRow } from "@/lib/explore";
import type { HomeCapital } from "@/lib/capital/ledger/home";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { CapitalPreview } from "./CapitalPreview";
import { CopyCommand } from "./CopyCommand";
import { HeroArt } from "./HeroArt";
import { CommunitySnapshot } from "./CommunitySnapshot";
import { HumansSection } from "./HumansSection";
import { ModelsSection } from "./ModelsSection";
import { InstagramSection } from "./InstagramSection";
import { TrustedBy } from "./TrustedBy";
import { DitherBand } from "./DitherCells";
import { PrimaryAction } from "./DitherButtonLink";
import { Arrow, Container, DisplayTitle, Lead, PageShell, TextLink } from "./primitives";
import { MotionStyles, Reveal } from "./Reveal";
import { wholeNumber } from "./format";

export interface HomePageProps {
  snapshot: Snapshot | null;
  featuredHuman: HumanRow | null;
  humansCoverage: Coverage | null;
  models: ModelUsageRow[];
  modelsCoverage: Coverage | null;
  capital: HomeCapital | null;
}

export function HomePage({ snapshot, featuredHuman, humansCoverage, models, modelsCoverage, capital }: HomePageProps) {
  const humans = snapshot?.humans.length ? snapshot.humans : featuredHuman ? [featuredHuman] : [];

  return (
    <PageShell>
      <MotionStyles />
      <main>
        <Hero aria-labelledby="home-title">
          <HeroGrid>
            <Card>
              <AgentStripe snapshot={snapshot} />
              <CardBody>
                  <Reveal delay={0}>
                    <DisplayTitle id="home-title">AI coding, counted in tokens.</DisplayTitle>
                  </Reveal>
                  <Reveal delay={70}>
                    <HeroLead>
                      Tokscale reads the logs your coding agents keep, counts the tokens, and publishes the leaderboards of
                      who uses what.
                    </HeroLead>
                  </Reveal>
                  <Reveal delay={140}>
                    <Actions>
                      <PrimaryAction href="/tracker">Track your usage</PrimaryAction>
                      <TextLink href="/leaderboard">
                        See the leaderboards <Arrow />
                      </TextLink>
                    </Actions>
                  </Reveal>
                  <Reveal delay={210}>
                    <Meta>
                      Public usage, last 30 days
                      {snapshot ? ` · ${wholeNumber(snapshot.coverage.accounts)} accounts` : ""} ·{" "}
                      <TextLink href="/methodology">How counts work</TextLink>
                    </Meta>
                  </Reveal>
                <Reveal delay={240}>
                  <CommunitySnapshot snapshot={snapshot} />
                </Reveal>
              </CardBody>
            </Card>

            <HeroArt />
          </HeroGrid>
        </Hero>

        <TrustedBy />

        <HumansSection humans={humans} coverage={humansCoverage} />
        <ModelsSection models={models} coverage={modelsCoverage} />
        <CapitalPreview data={capital} />
        <InstagramSection />

        <Closing aria-labelledby="home-closing-title">
          <Container>
            <ClosingPanel>
              <ClosingField aria-hidden="true">
                <DitherGradient from="blue" to="transparent" direction="left" cell={5} renderMode="static" className="field" />
              </ClosingField>
              <ClosingCopy>
                <ClosingTitle id="home-closing-title">Count your own tokens.</ClosingTitle>
                <ClosingNote>One command, no account, reads only your machine.</ClosingNote>
                <CopyCommand command="bunx tokscale@latest" label="Copy the command to run Tokscale" typed />
              </ClosingCopy>
            </ClosingPanel>
          </Container>
        </Closing>
      </main>
      <PublicFooter />
    </PageShell>
  );
}

/**
 * The stripe across the card's top: one block per top agent, its width its
 * share of those agents' tokens in the window. The only ornament on the card,
 * and it is data.
 */
function AgentStripe({ snapshot }: { snapshot: Snapshot | null }) {
  const agents = snapshot?.agents.slice(0, 6) ?? [];
  const total = agents.reduce((sum, agent) => sum + agent.tokens, 0);
  if (!agents.length || total <= 0) return <StripeBand aria-hidden="true" />;

  return (
    <StripeBand aria-hidden="true">
      <DitherBand weights={agents.map((agent) => agent.tokens)} cells={64} />
    </StripeBand>
  );
}

/* The first screen: a printed card on the left, the room on the right. */
const Hero = styled.section`
  position: relative;
  overflow: hidden;
  padding: 80px 0 56px;

  @media (max-width: 720px) {
    padding-top: 72px;
  }
`;

const HeroGrid = styled(Container)`
  position: relative;
  display: grid;
  /* The card carries the words and the live boards; the dithered pillars are
     the first screen's mass in the open right half. */
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  gap: 12px;
  align-items: stretch;

  @media (max-width: 980px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 0;
  }
`;

const Card = styled.div`
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);

  @media (max-width: 980px) {
    order: 2;
    margin-top: -24px;
  }
`;

/* The card's one ornament, and it is data: a full-width field of equal cells,
   denser where one agent's share of these tokens is larger. */
const StripeBand = styled.div`
  flex-shrink: 0;
  padding: 12px 12px 4px;
  background: var(--v2-surface);
`;

/* The card reads down: headline, lead, actions, then the live boards. */
const CardBody = styled.div`
  display: flex;
  flex-direction: column;
  gap: 20px;
  padding: 28px 24px 20px;

  @media (max-width: 520px) {
    padding: 24px 16px 16px;
  }
`;

const HeroLead = styled(Lead)`
  max-width: 32rem;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px 20px;
`;

const Meta = styled.p`
  margin: 10px 0 0;
  color: var(--v2-text-subtle);
  font-size: 13px;
`;


/* The closing panel: the page's one action again, on a dithered field that
   thickens toward the right edge — the motif as a quiet surface, not a word
   blown up past the viewport. */
const Closing = styled.section`
  margin-top: 128px;

  @media (max-width: 720px) {
    margin-top: 80px;
  }
`;

const ClosingPanel = styled.div`
  position: relative;
  display: flex;
  align-items: center;
  min-height: 320px;
  overflow: hidden;
  padding: 56px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);

  @media (max-width: 720px) {
    min-height: 0;
    padding: 32px 24px;
  }
`;

/* Right 60% of the panel; hidden on phones, where the copy stands alone. */
const ClosingField = styled.div`
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 60%;
  pointer-events: none;

  .field {
    position: absolute;
    inset: 0;
  }

  @media (max-width: 720px) {
    display: none;
  }
`;

const ClosingCopy = styled.div`
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 16px;
  max-width: 520px;
`;

const ClosingTitle = styled.h2`
  margin: 0;
  color: var(--v2-text);
  font-size: clamp(32px, 4vw, 56px);
  font-weight: 500;
  letter-spacing: -0.03em;
  line-height: 1.04;
  text-wrap: balance;
`;

const ClosingNote = styled.p`
  margin: 0 0 8px;
  color: var(--v2-text-muted);
  font-size: 17px;
  line-height: 1.5;
`;
