"use client";

import Link from "next/link";
import styled, { css, keyframes } from "styled-components";
import { PublicFooter } from "@/components/layout/PublicFooter";
import type { SupportedAgent } from "@/components/home/agents";
import { CopyCommand } from "@/components/home/CopyCommand";
import {
  Container,
  DisplayTitle,
  Eyebrow,
  focusRing,
  Lead,
  PageShell,
  PrimaryButton,
  Section,
  SectionHeader,
  SectionTitle,
} from "@/components/home/primitives";
import { TwinkleArt } from "@/components/home/TwinkleArt";
import { compactNumber, wholeNumber } from "@/components/home/format";
import { AgentsGrid } from "./AgentsGrid";
// import { EmbedShowcase } from "./EmbedShowcase"; // README embed section is hidden for now (see below).
import { FlowDiagram } from "./FlowDiagram";
import { TuiShowcase } from "./TuiShowcase";

const GITHUB_URL = "https://github.com/junhoyeo/tokscale";

/** All-time totals from the public leaderboard (its "Ranked users" and "Tokens"). */
export interface TrackerStats {
  rankedUsers: number;
  tokens: number;
}

export interface TrackerFaq {
  question: string;
  answer: string;
}

export const TRACKER_FAQ: TrackerFaq[] = [
  {
    question: "Do I need an account to use Tokscale?",
    answer:
      "No. The terminal UI runs on your machine and reads local files. You only sign in with GitHub when you want to submit usage to a public profile.",
  },
  {
    question: "Does Tokscale read my prompts or code?",
    answer:
      "It reads your agents' local session files to count tokens, but it never sends prompts, responses, code, file names, or API keys. Run tokscale submit --dry-run to see exactly what a submission contains.",
  },
  {
    question: "How is cost estimated?",
    answer:
      "Tokscale multiplies token counts by public list prices from LiteLLM, with OpenRouter as a fallback. It is an estimate. Subscriptions, discounts, and free tiers are not reflected.",
  },
  {
    question: "Can I keep my profile up to date automatically?",
    answer:
      "Yes. tokscale autosubmit enable schedules the normal submit with your operating system scheduler, every 24 hours by default. Pass --interval to change it, and tokscale autosubmit disable to remove it.",
  },
  {
    question: "How do I remove my data?",
    answer:
      "Open Settings on tokscale.ai. You can delete your submitted data and keep the account, or delete the account and everything linked to it. Both take effect immediately.",
  },
];

export function TrackerPageClient({ agents, stats }: { agents: SupportedAgent[]; stats: TrackerStats | null }) {
  return (
    <PageShell>
      <main>
        <Hero aria-labelledby="tracker-title">
          <Container>
            <HeroCopy>
              <Eyebrow>Tracker</Eyebrow>
              <DisplayTitle id="tracker-title">
                Track your AI usage <Line>from the terminal.</Line>
              </DisplayTitle>
              <HeroLead>
                Tokscale reads the session logs your coding agents already keep and shows tokens, models, and
                estimated cost on your machine. Submit when you want a public profile.
              </HeroLead>
              <CopyCommand command="bunx tokscale@latest" label="Copy the command to run Tokscale" typed />
              <HeroLinks>
                <a href={GITHUB_URL} target="_blank" rel="noopener noreferrer">
                  Read the docs on GitHub <span aria-hidden="true">↗</span>
                </a>
                <Link href="/local">Open the local viewer</Link>
              </HeroLinks>
            </HeroCopy>
          </Container>
        </Hero>

        <ShotStage>
          <HeroArt aria-hidden="true">
            <HeroArtImage
              src="/assets/v2/ledger-blue.webp"
              width={2472}
              height={1392}
              mobileSrc="/assets/v2/ledger-blue-960.webp"
              mobileWidth={1356}
              mobileHeight={768}
            />
          </HeroArt>
          <Container>
            <TerminalShot>
              {/* eslint-disable-next-line @next/next/no-img-element -- static screenshot with its own window chrome */}
              <img
                src="/assets/v2/tui/tui-overview.webp"
                alt="The Tokscale terminal UI, Overview view: tokens per day by model, and every model ranked by estimated cost."
                width={1800}
                height={1270}
                fetchPriority="high"
              />
            </TerminalShot>
            <StatStrip>
              {stats ? (
                <>
                  <li>
                    <b>{wholeNumber(stats.rankedUsers)}</b> people on the leaderboard
                  </li>
                  <li>
                    <b>{compactNumber(stats.tokens)}</b> tokens, all time
                  </li>
                </>
              ) : null}
              <li>
                <b>{agents.length}</b> agents supported
              </li>
            </StatStrip>
          </Container>
        </ShotStage>

        <Section aria-labelledby="tracker-flow-title">
          <Container>
            <Header>
              <Eyebrow>How It Works</Eyebrow>
              <SectionTitle id="tracker-flow-title">From your logs to your profile.</SectionTitle>
              <Lead>Tokscale runs on your machine. Only counts cross to tokscale.ai, and only when you submit.</Lead>
            </Header>
            <FlowDiagram agents={agents} />
          </Container>
        </Section>

        <Section aria-labelledby="tracker-tui-title">
          <Container>
            <TuiShowcase
              intro={
                <SectionHeader>
                  <Eyebrow>Terminal UI</Eyebrow>
                  <SectionTitle id="tracker-tui-title">Your usage, in your terminal.</SectionTitle>
                  <Lead>Six of its views. In the app, switch views with the arrow keys or Tab.</Lead>
                </SectionHeader>
              }
            />
          </Container>
        </Section>

        <Section aria-labelledby="tracker-agents-title">
          <Container>
            <Header>
              <Eyebrow>Supported Agents</Eyebrow>
              <SectionTitle id="tracker-agents-title">Works with the agents you already use.</SectionTitle>
              <Lead>
                Tokscale reads local logs from {agents.length} coding agents and AI clients.
              </Lead>
            </Header>
            <AgentsGrid agents={agents} />
          </Container>
        </Section>

        {/* Hidden for now: the README embed section. To restore, uncomment this block and the EmbedShowcase import.
        <Section aria-labelledby="tracker-embed-title">
          <Container>
            <EmbedShowcase
              intro={
                <SectionHeader>
                  <Eyebrow>README Embed</Eyebrow>
                  <SectionTitle id="tracker-embed-title">Put your usage in your README.</SectionTitle>
                  <Lead>
                    Pick one of eight cards. Each card renders from your latest submitted data. More options, like
                    theme and period, are in the Embed dialog on your profile.
                  </Lead>
                </SectionHeader>
              }
            />
          </Container>
        </Section>
        */}

        <Section aria-labelledby="tracker-data-title">
          <Container>
            <Header>
              <Eyebrow>Your Data</Eyebrow>
              <SectionTitle id="tracker-data-title">Counts leave your machine. Content does not.</SectionTitle>
            </Header>
            <Ledger>
              <LedgerSide aria-labelledby="tracker-data-sends">
                <h3 id="tracker-data-sends">What tokscale submit sends</h3>
                <ul>
                  <li>Daily token counts per client and model: input, output, cache, and reasoning</li>
                  <li>Estimated cost, from public list prices</li>
                  <li>Message counts and session timing, never message content</li>
                  <li>The names of MCP servers configured in your clients</li>
                  <li>Your CLI version and a device label</li>
                </ul>
              </LedgerSide>
              <LedgerSide aria-labelledby="tracker-data-never" $never>
                <h3 id="tracker-data-never">What it never sends</h3>
                <ul>
                  <li>Prompts, responses, or any conversation content</li>
                  <li>Source code, file contents, or file names</li>
                  <li>API keys or provider credentials</li>
                </ul>
              </LedgerSide>
            </Ledger>
            <ControlsRow aria-labelledby="tracker-data-controls">
              <h3 id="tracker-data-controls">Your controls</h3>
              <ul>
                <li>
                  Inspect an export in your browser with the <Link href="/local">local viewer</Link>. Nothing is
                  uploaded.
                </li>
                <li>
                  Delete submitted data or your whole account in <Link href="/settings">Settings</Link>.
                </li>
                <li>
                  The CLI is MIT licensed and the web app is AGPL-3.0.{" "}
                  <a href={GITHUB_URL} target="_blank" rel="noopener noreferrer">
                    Read the source
                  </a>{" "}
                  or self-host with Docker Compose.
                </li>
              </ul>
            </ControlsRow>
            <DataNote>
              Full details are in the <Link href="/privacy">privacy policy</Link>.
            </DataNote>
          </Container>
        </Section>

        <Section aria-labelledby="tracker-faq-title">
          <Container>
            <FaqLayout>
              <Header>
                <Eyebrow>FAQ</Eyebrow>
                <SectionTitle id="tracker-faq-title">Questions</SectionTitle>
              </Header>
              <Faq>
                {TRACKER_FAQ.map((item) => (
                  <details key={item.question}>
                    <summary>{item.question}</summary>
                    <p>{item.answer}</p>
                  </details>
                ))}
              </Faq>
            </FaqLayout>
          </Container>
        </Section>

        <Closing aria-labelledby="tracker-closing-title">
          <ClosingArt aria-hidden="true">
            <ClosingArtImage
              src="/assets/v2/recorder-blue.webp"
              width={2880}
              height={1620}
              mobileSrc="/assets/v2/recorder-blue-960.webp"
              mobileWidth={1248}
              mobileHeight={696}
              loading="lazy"
            />
          </ClosingArt>
          <Container>
            <ClosingInner>
              <SectionTitle id="tracker-closing-title">Start with one command.</SectionTitle>
              <Lead>No account needed to look at your own usage.</Lead>
              <CopyCommand command="bunx tokscale@latest" label="Copy the command to run Tokscale" />
              <PrimaryButton href="/leaderboard">See who else is building</PrimaryButton>
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

const HeroCopy = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
  max-width: 860px;
  margin: 0 auto;
  text-align: center;
`;

const HeroLead = styled(Lead)`
  max-width: 36rem;
  font-size: 18px;

  @media (max-width: 520px) {
    font-size: 16px;
  }
`;

const HeroLinks = styled.p`
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px 24px;
  margin: 0;

  a {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: 32px;
    border-radius: 4px;
    color: var(--v2-text-muted);
    font-size: 14px;
    text-decoration: underline;
    text-decoration-color: var(--v2-rule-strong);
    text-underline-offset: 3px;
    ${focusRing}
  }

  a:hover {
    color: var(--v2-text);
  }
`;

const Header = styled(SectionHeader)`
  margin-bottom: 40px;
`;

const DataNote = styled.p`
  margin: 24px 0 0;
  color: var(--v2-text-subtle);
  font-size: 14px;

  a {
    color: var(--v2-text-muted);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
`;

const FaqLayout = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 4fr) minmax(0, 8fr);
  gap: 40px;

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 0;
  }
`;

const Faq = styled.div`
  border-top: 1px solid var(--v2-rule-strong);

  details {
    border-bottom: 1px solid var(--v2-rule);
  }

  summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: 56px;
    padding: 12px 0;
    color: var(--v2-text);
    font-size: 16px;
    font-weight: 500;
    cursor: pointer;
    list-style: none;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary::after {
    flex-shrink: 0;
    color: var(--v2-text-subtle);
    font-size: 20px;
    font-weight: 300;
    content: "+";
  }

  details[open] summary::after {
    content: "−";
  }

  summary:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }

  p {
    max-width: 44rem;
    margin: 0;
    padding: 0 0 20px;
    color: var(--v2-text-muted);
    font-size: 15px;
    line-height: 1.6;
  }
`;

const rise = keyframes`
  from {
    opacity: 0;
    transform: translateY(28px);
  }
  to {
    opacity: 1;
    transform: none;
  }
`;

const Hero = styled.section`
  position: relative;
  z-index: 1;
  padding: 128px 0 56px;
  /* The same faint blue glow as the home hero. */
  background: var(--v2-glow);

  @media (max-width: 720px) {
    padding: 112px 0 40px;
  }
`;

/* The terminal is the hero's product shot; the ledger engraving shows on either side of it. */
const ShotStage = styled.div`
  position: relative;
  isolation: isolate;
  padding-bottom: 72px;
`;

const HeroArt = styled.div`
  position: absolute;
  z-index: -1;
  inset: -160px 0 0;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  overflow: hidden;
  pointer-events: none;
  opacity: 0.9;
  -webkit-mask-image:
    radial-gradient(ellipse 72% 92% at 50% 90%, #000 55%, transparent 100%),
    linear-gradient(to bottom, transparent 0%, #000 34%, #000 72%, transparent 100%);
  -webkit-mask-composite: source-in;
  mask-image:
    radial-gradient(ellipse 72% 92% at 50% 90%, #000 55%, transparent 100%),
    linear-gradient(to bottom, transparent 0%, #000 34%, #000 72%, transparent 100%);
  mask-composite: intersect;
`;

const HeroArtImage = styled(TwinkleArt)`
  flex-shrink: 0;
  width: min(1680px, 150vw);
  margin-bottom: -40px;

  @media (max-width: 720px) {
    width: 190vw;
  }
`;

const TerminalShot = styled.figure`
  max-width: 940px;
  margin: 0 auto;
  border-radius: 12px;
  box-shadow:
    0 0 0 1px rgba(255, 255, 255, 0.1),
    0 40px 120px -32px rgba(0, 0, 0, 0.95),
    0 0 80px -20px rgba(22, 64, 160, 0.45);

  img {
    display: block;
    width: 100%;
    height: auto;
    border-radius: 12px;
  }

  @media (prefers-reduced-motion: no-preference) {
    ${css`
      animation: ${rise} 0.7s cubic-bezier(0.2, 0.7, 0.2, 1) 0.35s both;
    `}
  }
`;

/* A status bar under the terminal, dark enough to read over the dots. */
const StatStrip = styled.ul`
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 6px 28px;
  width: fit-content;
  max-width: 100%;
  margin: 28px auto 0;
  padding: 10px 22px;
  border: 1px solid var(--v2-rule);
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.82);
  color: var(--v2-text-muted);
  font-size: 14px;
  list-style: none;

  @media (max-width: 520px) {
    border-radius: 16px;
  }

  b {
    color: var(--v2-numeric);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
`;

/* Two sides of one ledger: what leaves the machine on submit, and what never does. */
const Ledger = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  border: 1px solid var(--v2-rule-strong);
  border-radius: 12px;
  background: var(--v2-surface);

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const LedgerSide = styled.section<{ $never?: boolean }>`
  min-width: 0;
  padding: 24px 28px 12px;

  & + & {
    border-left: 1px solid var(--v2-rule);
  }

  h3 {
    margin: 0 0 8px;
    color: var(--v2-text);
    font-family: var(--v2-font-display);
    font-size: 26px;
    font-weight: 400;
    line-height: 1.2;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    position: relative;
    padding: 12px 0 12px 22px;
    color: var(--v2-text-muted);
    font-size: 15px;
    line-height: 1.55;
  }

  li + li {
    border-top: 1px solid var(--v2-rule);
  }

  /* A blue dot for what is sent, a muted cross for what never is. */
  li::before {
    position: absolute;
    top: 12px;
    left: 0;
    width: 12px;
    color: ${({ $never }) => ($never ? "var(--v2-text-subtle)" : "#4f8ae6")};
    font-size: ${({ $never }) => ($never ? "15px" : "10px")};
    line-height: ${({ $never }) => ($never ? "1.55" : "2.3")};
    text-align: center;
    content: "${({ $never }) => ($never ? "×" : "●")}";
  }

  @media (max-width: 900px) {
    & + & {
      border-top: 1px solid var(--v2-rule);
      border-left: 0;
    }
  }

  @media (max-width: 520px) {
    padding: 20px 16px 8px;
  }
`;

const ControlsRow = styled.section`
  display: grid;
  grid-template-columns: minmax(0, 3fr) minmax(0, 9fr);
  gap: 16px 24px;
  margin-top: 28px;

  h3 {
    margin: 0;
    color: var(--v2-text);
    font-size: 16px;
    font-weight: 600;
  }

  ul {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px 24px;
    margin: 0;
    padding: 0;
    color: var(--v2-text-muted);
    font-size: 15px;
    line-height: 1.55;
    list-style: none;
  }

  a {
    border-radius: 3px;
    color: var(--v2-text);
    text-decoration: underline;
    text-decoration-color: var(--v2-rule-strong);
    text-underline-offset: 3px;
    ${focusRing}
  }

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);

    ul {
      grid-template-columns: minmax(0, 1fr);
    }
  }
`;

const Closing = styled.section`
  position: relative;
  isolation: isolate;
  margin-top: 120px;
  padding: 120px 0 260px;
  border-top: 1px solid var(--v2-rule);

  @media (max-width: 720px) {
    margin-top: 80px;
    padding: 80px 0 170px;
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

/* Blue dot-matrix drum recorder, with twinkling dots. */
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

  /* Keep the copy readable over the engraving. */
  &::before {
    position: absolute;
    z-index: -1;
    inset: -80px -160px;
    background: radial-gradient(closest-side, rgba(0, 0, 0, 0.92) 55%, rgba(0, 0, 0, 0) 100%);
    content: "";
    pointer-events: none;
  }
`;
