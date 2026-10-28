"use client";

import Link from "next/link";
import styled from "styled-components";
import { DitherGradient } from "dither-kit/DitherGradient";
import { PublicFooter } from "@/components/layout/PublicFooter";
import type { SupportedAgent } from "@/components/home/agents";
import { CopyCommand } from "@/components/home/CopyCommand";
import {
  Container,
  DisplayTitle,
  Lead,
  PageShell,
  Section,
  SectionTitle,
  focusRing,
} from "@/components/home/primitives";
import { MotionStyles, Reveal } from "@/components/home/Reveal";
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
      <MotionStyles />
      <main>
        <Hero id="install" aria-labelledby="tracker-title">
          <Container>
            <HeroTop>
              <HeroCopy>
                <Reveal>
                  <DisplayTitle id="tracker-title">AI usage, from your terminal.</DisplayTitle>
                </Reveal>
                <Reveal delay={70}>
                  <HeroLead>
                    Tokscale reads the session logs your coding agents already keep and shows tokens, models, and
                    estimated cost on your machine. Submit when you want a public profile.
                  </HeroLead>
                </Reveal>
              </HeroCopy>
              <HeroAction>
                <Reveal delay={140}>
                  <CopyCommand command="bunx tokscale@latest" label="Copy the command to run Tokscale" typed />
                  <HeroLinks>
                    <a href={GITHUB_URL} target="_blank" rel="noopener noreferrer">
                      Read the docs on GitHub <span aria-hidden="true">↗</span>
                    </a>
                    <Link href="/local">Open the local viewer</Link>
                  </HeroLinks>
                </Reveal>
              </HeroAction>
            </HeroTop>
          </Container>

          <ShotWrap>
            <Container>
              <Reveal delay={120}>
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
              </Reveal>
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
          </ShotWrap>
        </Hero>

        <Section aria-labelledby="tracker-flow-title">
          <Container>
            <Header>
              <Reveal>
                <SectionTitle id="tracker-flow-title">From your logs to your profile.</SectionTitle>
              </Reveal>
              <Reveal delay={70}>
                <Lead>Tokscale runs on your machine. Only counts cross to tokscale.ai, and only when you submit.</Lead>
              </Reveal>
            </Header>
            <Reveal delay={120}>
              <FlowDiagram agents={agents} />
            </Reveal>
          </Container>
        </Section>

        <Section aria-labelledby="tracker-tui-title">
          <Container>
            <TuiShowcase
              intro={
                <div>
                  <Reveal>
                    <SectionTitle id="tracker-tui-title">Your usage, in your terminal.</SectionTitle>
                  </Reveal>
                  <Reveal delay={70}>
                    <TuiLead>Six of its views. In the app, switch views with the arrow keys or Tab.</TuiLead>
                  </Reveal>
                </div>
              }
            />
          </Container>
        </Section>

        <Section aria-labelledby="tracker-agents-title">
          <Container>
            <Header>
              <Reveal>
                <SectionTitle id="tracker-agents-title">Works with the agents you already use.</SectionTitle>
              </Reveal>
              <Reveal delay={70}>
                <Lead>Tokscale reads local logs from {agents.length} coding agents and AI clients.</Lead>
              </Reveal>
            </Header>
            <Reveal delay={120}>
              <AgentsGrid agents={agents} />
            </Reveal>
          </Container>
        </Section>

        {/* Hidden for now: the README embed section. To restore, uncomment this block and the EmbedShowcase import.
        <Section aria-labelledby="tracker-embed-title">
          <Container>
            <EmbedShowcase
              intro={
                <div>
                  <SectionTitle id="tracker-embed-title">Put your usage in your README.</SectionTitle>
                </div>
              }
            />
          </Container>
        </Section>
        */}

        <Section aria-labelledby="tracker-data-title">
          <Container>
            <Header>
              <Reveal>
                <SectionTitle id="tracker-data-title">Counts leave your machine. Content does not.</SectionTitle>
              </Reveal>
            </Header>
            <Reveal delay={100}>
              <Ledger>
                <LedgerSide aria-labelledby="tracker-data-sends">
                  <LedgerTitle id="tracker-data-sends">What tokscale submit sends</LedgerTitle>
                  <LedgerList>
                    <li>Daily token counts per client and model: input, output, cache, and reasoning</li>
                    <li>Estimated cost, from public list prices</li>
                    <li>Message counts and session timing, never message content</li>
                    <li>The names of MCP servers configured in your clients</li>
                    <li>Your CLI version and a device label</li>
                  </LedgerList>
                </LedgerSide>
                <LedgerSide aria-labelledby="tracker-data-never" $never>
                  <LedgerTitle id="tracker-data-never">What it never sends</LedgerTitle>
                  <LedgerList>
                    <li>Prompts, responses, or any conversation content</li>
                    <li>Source code, file contents, or file names</li>
                    <li>API keys or provider credentials</li>
                  </LedgerList>
                </LedgerSide>
              </Ledger>
            </Reveal>
            <ControlsRow aria-labelledby="tracker-data-controls">
              <ControlsTitle id="tracker-data-controls">Your controls</ControlsTitle>
              <ControlsList>
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
              </ControlsList>
            </ControlsRow>
            <DataNote>
              Full details are in the <Link href="/privacy">privacy policy</Link>.
            </DataNote>
          </Container>
        </Section>

        <Section aria-labelledby="tracker-faq-title">
          <Container>
            <FaqLayout>
              <FaqHeader>
                <Reveal>
                  <SectionTitle id="tracker-faq-title">Questions</SectionTitle>
                </Reveal>
              </FaqHeader>
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
          <Container>
            <ClosingPanel>
              <ClosingField aria-hidden="true">
                <DitherGradient from="blue" to="transparent" direction="left" cell={5} renderMode="static" className="field" />
              </ClosingField>
              <ClosingCopy>
                <ClosingTitle id="tracker-closing-title">Count your own tokens.</ClosingTitle>
                <ClosingNote>One command, no account. Reads local logs; submits only counts.</ClosingNote>
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

const Hero = styled.section`
  position: relative;
  padding: 88px 0 0;

  @media (max-width: 720px) {
    padding-top: 76px;
  }
`;

/* The headline on the left, the command on the right: opposite ends of one
   baseline, not a centred stack. */
const HeroTop = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 7fr) minmax(0, 5fr);
  gap: 48px;
  align-items: end;

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 28px;
  }
`;

const HeroCopy = styled.div`
  display: flex;
  flex-direction: column;
  gap: 18px;
  min-width: 0;
`;

const HeroLead = styled(Lead)`
  max-width: 40rem;
`;

const HeroAction = styled.div`
  display: flex;
  flex-direction: column;
  gap: 12px;
  justify-self: end;
  width: min(100%, 420px);

  @media (max-width: 900px) {
    justify-self: start;
  }
`;

const HeroLinks = styled.p`
  display: flex;
  flex-wrap: wrap;
  gap: 8px 22px;
  margin: 0;

  a {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: 32px;
    border-radius: var(--radius, 2px);
    color: var(--v2-text-muted);
    font-size: 15px;
    text-decoration: underline;
    text-decoration-thickness: 0.08em;
    text-underline-offset: 0.18em;
    ${focusRing}

    &:hover {
      color: var(--v2-text);
    }
  }
`;

const ShotWrap = styled.div`
  margin-top: 44px;

  @media (max-width: 720px) {
    margin-top: 28px;
  }
`;

/* The terminal is the hero's product shot: its own window chrome, no frame. */
const TerminalShot = styled.figure`
  margin: 0;

  img {
    display: block;
    width: 100%;
    height: auto;
    border-radius: var(--radius, 2px);
  }
`;

const StatStrip = styled.ul`
  display: flex;
  flex-wrap: wrap;
  gap: 10px 36px;
  margin: 18px 0 0;
  padding: 0;
  color: var(--v2-text-muted);
  font-size: 16px;
  list-style: none;

  b {
    color: var(--v2-text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
`;

const Header = styled.header`
  display: flex;
  flex-direction: column;
  gap: 16px;
  max-width: 640px;
  margin-bottom: 40px;
`;

const TuiLead = styled(Lead)`
  margin-top: 14px;
`;

const Ledger = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 2px;
  background: var(--v2-canvas);

  @media (max-width: 760px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const LedgerSide = styled.div<{ $never?: boolean }>`
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 24px;
  border-radius: var(--radius, 2px);
  background: ${({ $never }) => ($never ? "var(--v2-surface-raised)" : "var(--v2-surface)")};

  @media (max-width: 520px) {
    padding: 20px 16px;
  }
`;

const LedgerTitle = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-size: 20px;
  font-weight: 500;
  letter-spacing: -0.01em;
`;

const LedgerList = styled.ul`
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 0;
  padding: 0;
  color: var(--v2-text);
  font-size: 16px;
  line-height: 1.5;
  list-style: none;

  li {
    padding-left: 18px;
    position: relative;
  }

  /* The motif at text size: a 2px cell in the data blue. */
  li::before {
    position: absolute;
    top: 0.6em;
    left: 0;
    width: 6px;
    height: 6px;
    background: var(--v2-accent);
    content: "";
  }
`;

const ControlsRow = styled.section`
  display: flex;
  flex-direction: column;
  gap: 14px;
  margin-top: 32px;
`;

const ControlsTitle = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-size: 20px;
  font-weight: 500;
  letter-spacing: -0.01em;
`;

const ControlsList = styled.ul`
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 0;
  padding: 0;
  color: var(--v2-text);
  font-size: 16px;
  line-height: 1.55;
  list-style: none;

  a {
    color: var(--v2-text);
    text-decoration: underline;
    text-decoration-thickness: 0.08em;
    text-underline-offset: 0.18em;
    ${focusRing}
  }
`;

const DataNote = styled.p`
  margin: 20px 0 0;
  color: var(--v2-text-subtle);
  font-size: 14px;

  a {
    color: var(--v2-text-muted);
    text-decoration: underline;
    text-underline-offset: 0.18em;
  }
`;

const FaqLayout = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 4fr) minmax(0, 8fr);
  gap: 48px;
  align-items: start;

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 20px;
  }
`;

const FaqHeader = styled.div`
  position: sticky;
  top: 96px;

  @media (max-width: 900px) {
    position: static;
  }
`;

const Faq = styled.div`
  display: flex;
  flex-direction: column;

  details {
    padding: 6px 0;
  }

  summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: 44px;
    padding: 6px 12px;
    border-radius: var(--radius, 2px);
    color: var(--v2-text);
    font-size: 17px;
    font-weight: 500;
    cursor: pointer;
    list-style: none;
    ${focusRing}

    &::-webkit-details-marker {
      display: none;
    }

    &::after {
      flex-shrink: 0;
      color: var(--v2-text-muted);
      content: "+";
      font-size: 20px;
      line-height: 1;
    }

    &:hover {
      background: var(--v2-surface);
    }
  }

  details[open] summary::after {
    content: "−";
  }

  p {
    max-width: 62ch;
    margin: 4px 0 10px;
    padding: 0 12px;
    color: var(--v2-text-muted);
    font-size: 16px;
    line-height: 1.6;
  }
`;

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

