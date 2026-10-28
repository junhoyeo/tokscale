"use client";

import Link from "next/link";
import styled from "styled-components";
import type { Coverage, ModelUsageRow } from "@/lib/explore";
import { ModelMark } from "@/components/explore/ModelMark";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { agentLogoUrl } from "./agents";
import { compactCost, compactNumber, percent, periodLabel, userChange, wholeNumber } from "./format";
import { Arrow, Container, Lead, Section, SectionTitle, TextLink, VisuallyHidden, focusRing } from "./primitives";
import { Reveal } from "./Reveal";

interface ModelsSectionProps {
  models: ModelUsageRow[];
  coverage: Coverage | null;
}

/** Cells in one half of a versus bar. */
const BAR_CELLS = 24;

/** A 4×4 Bayer threshold per column, read along the bar's length. */
const BAYER_ROW = [0, 8, 2, 10].map((v) => (v + 0.5) / 16);

interface Metric {
  label: string;
  a: number;
  b: number;
  formatA: string;
  formatB: string;
  noteA?: string;
  noteB?: string;
}

/**
 * The two most used models, head to head. Each fact is one row: model A's
 * figure at the left edge, model B's at the right, and between them two
 * dithered bars that meet at the fact's name. A bar's length is that model's
 * value against the larger of the two, so the longer side wins the row at a
 * glance; the winning bar is solid, the other a lighter dither.
 */
export function ModelsSection({ models, coverage }: ModelsSectionProps) {
  const [a, b] = models;
  const period = coverage ? periodLabel(coverage.period) : null;

  return (
    <Section aria-labelledby="home-models-title">
      <Container>
        <Head>
          <Reveal>
            <SectionTitle id="home-models-title">Compare models before you build.</SectionTitle>
          </Reveal>
          <HeadSide>
            <Reveal delay={70}>
              <Lead>
                How many people use a model, how many tokens it handles, and which agents carry it. Every number is
                from submitted usage{period ? `, ${period.toLowerCase()}` : ""}.
              </Lead>
            </Reveal>
            <Reveal delay={140}>
              <TextLink href="/models">
                Compare every model <Arrow />
              </TextLink>
            </Reveal>
          </HeadSide>
        </Head>

        {a && b && coverage ? (
          <Reveal delay={120}>
            <Versus a={a} b={b} accounts={coverage.accounts} period={period ?? ""} />
          </Reveal>
        ) : null}
      </Container>
    </Section>
  );
}

function Versus({
  a,
  b,
  accounts,
  period,
}: {
  a: ModelUsageRow;
  b: ModelUsageRow;
  accounts: number;
  period: string;
}) {
  const changeA = userChange(a);
  const changeB = userChange(b);
  const metrics: Metric[] = [
    {
      label: "Active users",
      a: a.activeUsers,
      b: b.activeUsers,
      formatA: wholeNumber(a.activeUsers),
      formatB: wholeNumber(b.activeUsers),
      noteA: changeA.kind === "new" ? "New" : changeA.kind === "none" ? undefined : `${changeA.label} vs previous`,
      noteB: changeB.kind === "new" ? "New" : changeB.kind === "none" ? undefined : `${changeB.label} vs previous`,
    },
    {
      label: `Adoption of ${wholeNumber(accounts)} accounts`,
      a: a.adoption,
      b: b.adoption,
      formatA: percent(a.adoption),
      formatB: percent(b.adoption),
    },
    {
      label: "Tokens",
      a: a.tokens,
      b: b.tokens,
      formatA: compactNumber(a.tokens),
      formatB: compactNumber(b.tokens),
    },
    {
      label: "Estimated cost",
      a: a.estimatedCost,
      b: b.estimatedCost,
      formatA: compactCost(a.estimatedCost, a.tokens),
      formatB: compactCost(b.estimatedCost, b.tokens),
    },
  ];

  return (
    <Panel data-ui aria-label={`${a.name} compared with ${b.name}, ${period.toLowerCase()}`}>
      <Identities>
        <Identity model={a} period={period} side="a" />
        <VsMark aria-hidden="true">vs</VsMark>
        <Identity model={b} period={period} side="b" />
      </Identities>

      <Rows>
        {metrics.map((metric) => (
          <MetricRow key={metric.label} metric={metric} nameA={a.name} nameB={b.name} />
        ))}
      </Rows>

      <Agents>
        <AgentColumn model={a} side="a" />
        <AgentsLabel>Top agents, share of the model&apos;s tokens</AgentsLabel>
        <AgentColumn model={b} side="b" />
      </Agents>

      <Foot>Costs are estimates from public list prices.</Foot>
    </Panel>
  );
}

function Identity({ model, period, side }: { model: ModelUsageRow; period: string; side: "a" | "b" }) {
  return (
    <IdentityLink href={`/models/${encodeURIComponent(model.key)}`} $side={side}>
      <ModelMark model={model} size={32} />
      <IdentityText $side={side}>
        <ModelName>{model.name}</ModelName>
        <ModelMeta>
          {model.lab ?? "Unknown lab"} ·{" "}
          <InstantTooltip as="span" content={`#${model.rank} by active users, ${period.toLowerCase()}`} describe={false} openWithParentFocus>
            #{model.rank}
            <VisuallyHidden> by active users, {period.toLowerCase()}</VisuallyHidden>
          </InstantTooltip>
        </ModelMeta>
      </IdentityText>
    </IdentityLink>
  );
}

function MetricRow({ metric, nameA, nameB }: { metric: Metric; nameA: string; nameB: string }) {
  const max = Math.max(metric.a, metric.b, 0);
  const fracA = max > 0 ? metric.a / max : 0;
  const fracB = max > 0 ? metric.b / max : 0;
  return (
    <Row>
      <Value $side="a">
        <Figure>{metric.formatA}</Figure>
        {metric.noteA ? <Note>{metric.noteA}</Note> : null}
      </Value>
      <VersusBar frac={fracA} lead={metric.a >= metric.b} side="a" label={`${nameA}: ${metric.formatA}`} />
      <RowLabel>{metric.label}</RowLabel>
      <VersusBar frac={fracB} lead={metric.b >= metric.a} side="b" label={`${nameB}: ${metric.formatB}`} />
      <Value $side="b">
        <Figure>{metric.formatB}</Figure>
        {metric.noteB ? <Note>{metric.noteB}</Note> : null}
      </Value>
    </Row>
  );
}

/* One half of a versus row: cells fill from the centre outward, so both bars
   start at the shared label. The leading side is solid; the other side's
   cells are thinned by the Bayer row, the motif's "less". */
function VersusBar({ frac, lead, side, label }: { frac: number; lead: boolean; side: "a" | "b"; label: string }) {
  const lit = Math.round(Math.min(1, Math.max(0, frac)) * BAR_CELLS);
  return (
    <Bar role="img" aria-label={label} $side={side}>
      {Array.from({ length: BAR_CELLS }, (_, i) => {
        const on = i < lit;
        const tone = !on ? "off" : lead || 0.62 > BAYER_ROW[i % 4] ? "on" : "thin";
        return <BarCell key={i} $tone={tone} />;
      })}
    </Bar>
  );
}

function AgentColumn({ model, side }: { model: ModelUsageRow; side: "a" | "b" }) {
  if (!model.topAgents.length) return <AgentList $side={side}><li><Muted>—</Muted></li></AgentList>;
  return (
    <AgentList $side={side} aria-label={`Top agents for ${model.name}`}>
      {model.topAgents.map((agent) => {
        const logo = agentLogoUrl(agent);
        return (
          <AgentItem key={agent.key} $side={side}>
            {logo ? <AgentLogo src={logo} alt="" width={16} height={16} loading="lazy" /> : <AgentDot aria-hidden="true" />}
            <AgentName>{agent.name}</AgentName>
            <AgentShare>{percent(agent.share)}</AgentShare>
          </AgentItem>
        );
      })}
    </AgentList>
  );
}

const Head = styled.header`
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  align-items: end;
  gap: 24px 48px;
  margin-bottom: 40px;

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const HeadSide = styled.div`
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
`;

const Panel = styled.section`
  display: flex;
  flex-direction: column;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
`;

/* Every row shares one five-track grid: figure | bar | label | bar | figure.
   The label track is fixed, so both bars always meet on the panel's centre. */
const rowGrid = `
  display: grid;
  grid-template-columns: minmax(96px, 1fr) minmax(0, 2fr) 168px minmax(0, 2fr) minmax(96px, 1fr);
  align-items: center;
  column-gap: 20px;
  padding: 0 28px;

  @media (max-width: 760px) {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    row-gap: 10px;
    padding: 0 16px;
  }
`;

const Identities = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  align-items: center;
  gap: 16px;
  padding: 24px 28px;

  @media (max-width: 520px) {
    padding: 20px 16px;
  }
`;

const VsMark = styled.span`
  color: var(--v2-text-muted);
  font-size: 14px;
  font-weight: 500;
`;

const IdentityLink = styled(Link)<{ $side: "a" | "b" }>`
  display: flex;
  flex-direction: ${({ $side }) => ($side === "a" ? "row" : "row-reverse")};
  align-items: center;
  gap: 12px;
  min-width: 0;
  border-radius: var(--radius, 2px);
  color: inherit;
  text-decoration: none;
  ${focusRing}

  &:hover h3 {
    text-decoration: underline;
    text-decoration-color: var(--v2-accent);
    text-underline-offset: 4px;
  }
`;

const IdentityText = styled.span<{ $side: "a" | "b" }>`
  display: flex;
  flex-direction: column;
  min-width: 0;
  text-align: ${({ $side }) => ($side === "a" ? "left" : "right")};
`;

const ModelName = styled.h3`
  margin: 0;
  overflow: hidden;
  color: var(--v2-text);
  font-size: 22px;
  font-weight: 600;
  letter-spacing: -0.015em;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 520px) {
    font-size: 17px;
  }
`;

const ModelMeta = styled.span`
  color: var(--v2-text-muted);
  font-size: 14px;
  font-variant-numeric: tabular-nums;
`;

const Rows = styled.div`
  display: flex;
  flex-direction: column;
`;

const Row = styled.div`
  ${rowGrid}
  min-height: 72px;
  padding-top: 14px;
  padding-bottom: 14px;
  border-top: 1px solid var(--v2-rule); /* taste-skill-ignore: table rows inside the data-ui comparison fragment */

  @media (max-width: 760px) {
    /* Phone: the label on top, then each side's figure over its bar. */
    & > :nth-child(3) {
      grid-column: 1 / -1;
      grid-row: 1;
      text-align: left;
    }
    & > :nth-child(1) { grid-column: 1; grid-row: 2; }
    & > :nth-child(5) { grid-column: 2; grid-row: 2; }
    & > :nth-child(2) { grid-column: 1; grid-row: 3; }
    & > :nth-child(4) { grid-column: 2; grid-row: 3; }
  }
`;

const Value = styled.div<{ $side: "a" | "b" }>`
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  align-items: ${({ $side }) => ($side === "a" ? "flex-start" : "flex-end")};
  text-align: ${({ $side }) => ($side === "a" ? "left" : "right")};
`;

const Figure = styled.span`
  color: var(--v2-text);
  font-size: 26px;
  font-weight: 500;
  letter-spacing: -0.01em;
  line-height: 1.1;
  font-variant-numeric: tabular-nums;

  @media (max-width: 520px) {
    font-size: 22px;
  }
`;

const Note = styled.span`
  color: var(--v2-text-muted);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
`;

const RowLabel = styled.span`
  color: var(--v2-text-muted);
  font-size: 14px;
  text-align: center;
`;

const Bar = styled.span<{ $side: "a" | "b" }>`
  display: flex;
  flex-direction: ${({ $side }) => ($side === "a" ? "row-reverse" : "row")};
  gap: 2px;
  height: 14px;
`;

const BarCell = styled.i<{ $tone: "on" | "thin" | "off" }>`
  flex: 1 1 0;
  min-width: 0;
  background: ${({ $tone }) =>
    $tone === "on" ? "var(--v2-accent)" : $tone === "thin" ? "#1d4f99" : "var(--v2-surface-raised)"};
`;

const Agents = styled.div`
  ${rowGrid}
  grid-template-columns: minmax(0, 3fr) 168px minmax(0, 3fr);
  align-items: start;
  padding-top: 20px;
  padding-bottom: 20px;
  border-top: 1px solid var(--v2-rule); /* taste-skill-ignore: table rows inside the data-ui comparison fragment */

  @media (max-width: 760px) {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);

    & > :nth-child(2) {
      grid-column: 1 / -1;
      grid-row: 1;
      text-align: left;
    }
  }
`;

const AgentsLabel = styled.span`
  padding-top: 2px;
  color: var(--v2-text-muted);
  font-size: 14px;
  line-height: 1.4;
  text-align: center;
`;

const AgentList = styled.ul<{ $side: "a" | "b" }>`
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const AgentItem = styled.li<{ $side: "a" | "b" }>`
  display: flex;
  flex-direction: ${({ $side }) => ($side === "a" ? "row" : "row-reverse")};
  align-items: center;
  gap: 8px;
  min-width: 0;
`;

const AgentLogo = styled.img`
  flex-shrink: 0;
  border-radius: var(--radius, 2px);
`;

const AgentDot = styled.span`
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  background: var(--v2-surface-raised);
`;

const AgentName = styled.span`
  overflow: hidden;
  color: var(--v2-text);
  font-size: 15px;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const AgentShare = styled.span`
  color: var(--v2-text-muted);
  font-size: 14px;
  font-variant-numeric: tabular-nums;
`;

const Muted = styled.span`
  color: var(--v2-text-muted);
`;

const Foot = styled.p`
  margin: 0;
  padding: 14px 28px;
  border-top: 1px solid var(--v2-rule); /* taste-skill-ignore: table rows inside the data-ui comparison fragment */
  color: var(--v2-text-muted);
  font-size: 13px;

  @media (max-width: 520px) {
    padding: 12px 16px;
  }
`;
