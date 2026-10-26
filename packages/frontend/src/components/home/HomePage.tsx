"use client";

import styled from "styled-components";
import type { CommunitySnapshot as Snapshot, Coverage, HumanRow, ModelUsageRow } from "@/lib/explore";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { CapitalPreview } from "./CapitalPreview";
import type { HomeCapital } from "@/lib/capital/ledger/home";
import { CommunitySnapshot } from "./CommunitySnapshot";
import { CopyCommand } from "./CopyCommand";
import { HumansSection } from "./HumansSection";
import { ModelsSection } from "./ModelsSection";
import { InstagramSection } from "./InstagramSection";
import { TrustedBy } from "./TrustedBy";
import { TwinkleArt } from "./TwinkleArt";
import { Container, DisplayTitle, Lead, PageShell, PrimaryButton, SecondaryButton, SectionTitle } from "./primitives";

export interface HomePageProps {
  snapshot: Snapshot | null;
  featuredHuman: HumanRow | null;
  humansCoverage: Coverage | null;
  models: ModelUsageRow[];
  modelsCoverage: Coverage | null;
  capital: HomeCapital | null;
}

export function HomePage({ snapshot, featuredHuman, humansCoverage, models, modelsCoverage, capital }: HomePageProps) {
  return (
    <PageShell>
      <main>
        <Hero aria-labelledby="home-title">
          <Container>
            <HeroCopy>
              <HeroTitle id="home-title">See how people build with AI.</HeroTitle>
              <HeroLead>
                Discover what builders use, compare AI models, and follow the companies and capital behind them.
              </HeroLead>
              <Actions>
                <PrimaryButton href="/leaderboard">Explore leaderboards</PrimaryButton>
                <SecondaryButton href="/tracker">Track your usage</SecondaryButton>
              </Actions>
            </HeroCopy>
          </Container>
        </Hero>

        <SnapshotStage>
          <Backdrop aria-hidden="true">
            <TwinkleArt
              src="/assets/v2/observatory-blue.webp"
              width={2472}
              height={1392}
              mobileSrc="/assets/v2/observatory-blue-960.webp"
              mobileWidth={1356}
              mobileHeight={768}
            />
          </Backdrop>
          <Container>
            <CommunitySnapshot snapshot={snapshot} />
            <TrustedBy />
          </Container>
        </SnapshotStage>

        <HumansSection human={featuredHuman} coverage={humansCoverage} />
        <ModelsSection models={models} coverage={modelsCoverage} />
        <CapitalPreview data={capital} />
        <InstagramSection />

        <Closing aria-labelledby="home-closing-title">
          <ClosingArt aria-hidden="true">
            <ClosingArtImage
              src="/assets/v2/instruments-blue.webp"
              width={2880}
              height={1620}
              mobileSrc="/assets/v2/instruments-blue-960.webp"
              mobileWidth={1248}
              mobileHeight={696}
              loading="lazy"
            />
          </ClosingArt>
          <Container>
            <ClosingInner>
              <SectionTitle id="home-closing-title">
                Track your usage. <Line>Join the community.</Line>
              </SectionTitle>
              <Lead>
                Tokscale reads the session logs your coding agents already keep, on your machine. Nothing leaves it
                until you submit.
              </Lead>
              <CopyCommand command="bunx tokscale@latest" label="Copy the command to run Tokscale" />
              <PrimaryButton href="/tracker">Start tracking</PrimaryButton>
            </ClosingInner>
          </Container>
        </Closing>
      </main>
      <PublicFooter />
    </PageShell>
  );
}

const Line = styled.span`
  @media (min-width: 721px) {
    display: block;
  }
`;

/* One line at 48px; narrow screens scale the size down so it never wraps (the line is ~9.6 em wide). */
const HeroTitle = styled(DisplayTitle)`
  font-size: 48px;
  line-height: 1.1;
  white-space: nowrap;

  @media (max-width: 520px) {
    font-size: min(48px, calc((100vw - 32px) * 0.104));
  }
`;

/* Above the snapshot backdrop, which reaches up behind the hero's lower edge. */
const Hero = styled.section`
  position: relative;
  z-index: 1;
  padding: 128px 0 36px;
  /* A faint blue glow behind the title, matching the blue backdrop below. */
  background: var(--v2-glow);

  @media (max-width: 720px) {
    padding: 112px 0 28px;
  }
`;

const HeroCopy = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  max-width: 860px;
  margin: 0 auto;
  text-align: center;
`;

const HeroLead = styled(Lead)`
  max-width: 34rem;
  font-size: 18px;

  @media (max-width: 520px) {
    font-size: 16px;
  }
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 6px;
  margin-top: 8px;

  @media (max-width: 400px) {
    flex-direction: column;
    align-self: stretch;
  }
`;

const SnapshotStage = styled.div`
  position: relative;
  isolation: isolate;
  /*
   * Room under the card and the logo row for the observatory engraving. The
   * padding tracks the engraving height, so the telescope always starts below
   * the logo row.
   */
  padding-bottom: min(440px, 42vw);

  @media (max-width: 720px) {
    padding-bottom: 64vw;
  }
`;

const Backdrop = styled.div`
  position: absolute;
  z-index: -1;
  bottom: -24px;
  left: 50%;
  width: min(1240px, 118vw);
  transform: translateX(-50%);
  pointer-events: none;
  opacity: 0.92;
  /*
   * Blue dot-matrix rendering of the observatory engraving (ordered dither).
   * Sits below the snapshot card: the card covers only the sky, and the dome
   * and telescope show in the space under it. Fade the sides, the top (behind
   * the card), and the base so it runs softly into the next section.
   */
  -webkit-mask-image:
    radial-gradient(ellipse 66% 90% at 50% 58%, #000 50%, transparent 100%),
    linear-gradient(to bottom, transparent 0%, #000 30%, #000 74%, transparent 100%);
  -webkit-mask-composite: source-in;
  mask-image:
    radial-gradient(ellipse 66% 90% at 50% 58%, #000 50%, transparent 100%),
    linear-gradient(to bottom, transparent 0%, #000 30%, #000 74%, transparent 100%);
  mask-composite: intersect;

  img {
    display: block;
    width: 100%;
    height: auto;
  }

  @media (max-width: 720px) {
    bottom: -12px;
    width: 175vw;
    opacity: 0.85;
  }
`;

const Closing = styled.section`
  position: relative;
  isolation: isolate;
  margin-top: 120px;
  padding: 120px 0 240px;
  border-top: 1px solid var(--v2-rule);

  @media (max-width: 720px) {
    margin-top: 80px;
    padding: 80px 0 160px;
  }
`;

const ClosingArt = styled.div`
  position: absolute;
  z-index: -1;
  inset: 0;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  overflow: hidden;
  pointer-events: none;
  opacity: 0.85;
  -webkit-mask-image: linear-gradient(to bottom, transparent 0%, #000 55%);
  mask-image: linear-gradient(to bottom, transparent 0%, #000 55%);
`;

/* Blue dot-matrix rendering of the instruments engraving, with twinkling dots. */
const ClosingArtImage = styled(TwinkleArt)`
  flex-shrink: 0;
  width: min(1440px, 160vw);
`;

const ClosingInner = styled.div`
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
  max-width: 640px;
  margin: 0 auto;
  text-align: center;

  > p {
    max-width: 30rem;
  }

  /* Keep the copy readable over the instruments engraving. */
  &::before {
    position: absolute;
    z-index: -1;
    inset: -80px -160px;
    background: radial-gradient(closest-side, rgba(0, 0, 0, 0.92) 55%, rgba(0, 0, 0, 0) 100%);
    content: "";
    pointer-events: none;
  }
`;
