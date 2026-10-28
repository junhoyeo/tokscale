"use client";

import Link from "next/link";
import styled from "styled-components";
import { ModelMark } from "@/components/explore/ModelMark";
import type { HomeCapital, HomeCapitalLab } from "@/lib/capital/ledger/home";
import { coverageRangeLabel, wholeNumber } from "./format";
import { Arrow, Container, Lead, Section, SectionTitle, TextLink, VisuallyHidden, focusRing } from "./primitives";
import { DitherCells } from "./DitherCells";
import { Reveal } from "./Reveal";

interface CapitalPreviewProps {
  data: HomeCapital | null;
}

function sharePercent(share: number): string {
  if (!Number.isFinite(share) || share <= 0) return "0%";
  if (share < 0.001) return "<0.1%";
  return `${(share * 100).toFixed(1)}%`;
}

/**
 * The money side: four kinds of money, each stated and never summed, the labs
 * behind the models with their share of Tokscale tokens as a dither field, and
 * the latest moves with their sources.
 */
export function CapitalPreview({ data }: CapitalPreviewProps) {
  const leader =
    data && data.labs.length > 0
      ? data.labs.reduce((best, lab) => (lab.share > best.share ? lab : best), data.labs[0])
      : null;

  return (
    <Section aria-labelledby="home-capital-title">
      <Container>
        <Head>
          <Reveal>
            <SectionTitle id="home-capital-title">The money behind the models you use.</SectionTitle>
          </Reveal>
          <Reveal delay={70}>
            <Lead>
              Funding rounds, compute contracts and capital spending across AI labs, clouds and chips. Every number comes
              from a filing or a company announcement.
            </Lead>
          </Reveal>
        </Head>

        {data ? (
          <>
            <Reveal delay={120}>
              <Stats aria-label="Four money types">
                {data.stats.map((stat) => (
                  <Stat key={stat.label}>
                    <StatValue>{stat.value}</StatValue>
                    <StatLabel>{stat.label}</StatLabel>
                    <StatDetail>{stat.detail}</StatDetail>
                    <StatLink href={stat.href}>
                      {stat.linkLabel} <Arrow />
                    </StatLink>
                  </Stat>
                ))}
              </Stats>
            </Reveal>
            <StatsNote>
              These are four different money types. They are never added together.{" "}
              <InlineLink href="/capital/methodology">How we count</InlineLink>
            </StatsNote>
            <SectionLink>
              <TextLink href="/capital">
                Explore capital <Arrow />
              </TextLink>
            </SectionLink>

            <Panels>
              <Reveal>
                <Panel aria-labelledby="home-capital-labs-title" data-ui>
                  <PanelHead>
                    <div>
                      <PanelTitle id="home-capital-labs-title">Labs behind the models you use</PanelTitle>
                      {leader ? (
                        <PanelAnswer>
                          {leader.name} carries the largest share: {sharePercent(leader.share)} of the tokens with an
                          identified model.
                        </PanelAnswer>
                      ) : null}
                    </div>
                    <PanelLink href="/capital/companies">
                      All companies <Arrow />
                    </PanelLink>
                  </PanelHead>
                  {data.labs.length > 0 ? (
                    <>
                      <LabTable>
                        <thead>
                          <tr>
                            <th scope="col">Lab</th>
                            <th scope="col">Share of tokens</th>
                            <th scope="col">Top model</th>
                            <th scope="col">Last round</th>
                          </tr>
                        </thead>
                        <tbody>
                          {data.labs.map((lab) => (
                            <LabRow key={lab.slug} lab={lab} />
                          ))}
                        </tbody>
                      </LabTable>
                      <PanelNote>
                        Tokscale tokens: share of tokens from identified models among{" "}
                        {data.labsWindow
                          ? `${wholeNumber(data.labsWindow.accounts)} Tokscale accounts, ${coverageRangeLabel(data.labsWindow) ?? "last 30 days"}`
                          : "Tokscale accounts, last 30 days"}
                        . Not market share. Valuations and contracts are company-stated or from filings.
                      </PanelNote>
                    </>
                  ) : (
                    <Empty>Tokscale usage data is not available right now.</Empty>
                  )}
                </Panel>
              </Reveal>

              <Reveal delay={80}>
                <Panel aria-labelledby="home-capital-moves-title" data-ui>
                  <PanelHead>
                    <PanelTitle id="home-capital-moves-title">Latest moves</PanelTitle>
                    <PanelLink href="/capital/rounds">
                      All rounds <Arrow />
                    </PanelLink>
                  </PanelHead>
                  <MoveList>
                    {data.events.map((event) => (
                      <li key={event.id} data-row-link="">
                        <MoveMeta>
                          <span>{event.date}</span>
                          <MoveTag>{event.tag}</MoveTag>
                        </MoveMeta>
                        <MoveBody>
                          <MoveTitle href={event.href} data-row-primary="">
                            {event.title}
                          </MoveTitle>
                          <MoveSub>{event.parties}</MoveSub>
                        </MoveBody>
                        <MoveValue>
                          <span>{event.value ?? "—"}</span>
                          {event.valuation ? <MoveSub>at {event.valuation} post</MoveSub> : null}
                          {event.source ? (
                            <SourceLink href={event.source.url} target="_blank" rel="noopener noreferrer">
                              {event.source.label} ↗
                            </SourceLink>
                          ) : null}
                        </MoveValue>
                      </li>
                    ))}
                  </MoveList>
                </Panel>
              </Reveal>
            </Panels>
          </>
        ) : (
          <Empty>Capital data is not available right now.</Empty>
        )}
      </Container>
    </Section>
  );
}

function LabRow({ lab }: { lab: HomeCapitalLab }) {
  return (
    <tr data-row-link={lab.href ? "" : undefined}>
      <LabCell>
        <LabName>
          {lab.logo ? (
            <LabLogo src={lab.logo} alt="" width={16} height={16} loading="lazy" />
          ) : lab.avatar ? (
            <LabLogo src={lab.avatar} alt="" width={20} height={20} loading="lazy" />
          ) : null}
          {lab.href ? (
            <CellLink href={lab.href} data-row-primary="">
              {lab.name}
            </CellLink>
          ) : (
            <span>{lab.name}</span>
          )}
        </LabName>
      </LabCell>
      <ShareCell>
        <ShareLine>
          <DitherCells
            value={lab.share}
            max={1}
            cols={26}
            rows={1}
            cell={5}
            label={`${lab.name}: ${sharePercent(lab.share)} of identified-model tokens`}
          />
          <ShareValue>
            <VisuallyHidden>Share of Tokscale tokens: </VisuallyHidden>
            {sharePercent(lab.share)}
          </ShareValue>
        </ShareLine>
      </ShareCell>
      <Cell>
        <MobileLabel aria-hidden="true">Top model</MobileLabel>
        {lab.topModel ? (
          <ModelCell>
            <ModelMark model={lab.topModel} size={16} />
            <CellLink href={lab.topModel.href}>{lab.topModel.name}</CellLink>
          </ModelCell>
        ) : (
          "—"
        )}
      </Cell>
      <Cell>
        <MobileLabel aria-hidden="true">Last round</MobileLabel>
        {lab.round ? (
          <>
            <CellLink href={lab.round.href}>{lab.round.label}</CellLink>
            <CellSub>
              {lab.round.valuation ? `${lab.round.valuation} post · ` : ""}
              {lab.round.date}
            </CellSub>
          </>
        ) : (
          <CellSub>{lab.status}</CellSub>
        )}
      </Cell>
    </tr>
  );
}

const Head = styled.header`
  display: flex;
  flex-direction: column;
  gap: 16px;
  max-width: 640px;
  margin-bottom: 36px;
`;

const Stats = styled.dl`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 2px;
  margin: 0;
  background: var(--v2-canvas);

  @media (max-width: 900px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
`;

const Stat = styled.div`
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  padding: 20px;
  background: var(--v2-surface);

  @media (max-width: 520px) {
    padding: 16px;
  }
`;

const StatValue = styled.dt`
  color: var(--v2-text);
  font-size: 28px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  line-height: 1.05;
  letter-spacing: -0.02em;

  @media (max-width: 520px) {
    font-size: 24px;
  }
`;

const StatLabel = styled.dd`
  margin: 0;
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 500;
  line-height: 1.35;
`;

const StatDetail = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 13px;
  line-height: 1.45;
`;

const StatLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  align-self: flex-start;
  min-height: 32px;
  margin-top: auto;
  border-radius: var(--radius, 2px);
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 500;
  text-decoration: none;
  ${focusRing}

  span[aria-hidden] {
    color: var(--v2-accent);
  }
`;

const StatsNote = styled.p`
  margin: 10px 0 0;
  color: var(--v2-text-subtle);
  font-size: 13px;
`;

const SectionLink = styled.p`
  margin: 18px 0 0;
`;

const InlineLink = styled(Link)`
  border-radius: var(--radius, 2px);
  color: var(--v2-text-muted);
  text-decoration: underline;
  text-decoration-thickness: 0.08em;
  text-underline-offset: 0.18em;
  ${focusRing}

  &:hover {
    color: var(--v2-text);
  }
`;

const Panels = styled.div`
  display: flex;
  flex-direction: column;
  gap: 32px;
  margin-top: 32px;
`;

const Panel = styled.section`
  --row-focus-offset: -2px;

  min-width: 0;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
`;

const PanelHead = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px 24px;
  padding: 20px 20px 12px;
`;

const PanelAnswer = styled.p`
  max-width: 60ch;
  margin: 6px 0 0;
  color: var(--v2-text);
  font-size: 16px;
  line-height: 1.45;
`;

const PanelTitle = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-size: 20px;
  font-weight: 500;
  letter-spacing: -0.01em;
`;

const PanelLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  border-radius: var(--radius, 2px);
  color: var(--v2-text-muted);
  font-size: 15px;
  font-weight: 500;
  text-decoration: none;
  ${focusRing}

  span[aria-hidden] {
    color: var(--v2-accent);
  }

  &:hover {
    color: var(--v2-text);
  }
`;

/* A real table on wide screens (quiet rules are the table's own convention). */
const LabTable = styled.table`
  width: 100%;
  border-collapse: collapse;
  margin-top: 4px;

  th {
    padding: 10px 16px;
    color: var(--v2-text-muted);
    font-size: 14px;
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
  }

  th:nth-child(2) {
    text-align: right;
  }

  td {
    padding: 14px 16px;
    border-top: 1px solid var(--v2-rule);
    color: var(--v2-text);
    font-size: 16px;
    vertical-align: middle;
  }

  tbody tr:first-child td {
    border-top: 0;
  }

  tbody tr:hover {
    background: var(--v2-row-hover);
  }

  @media (max-width: 980px) {
    thead {
      display: none;
    }

    tbody,
    tr,
    td {
      display: block;
      width: 100%;
    }

    tr {
      padding: 12px 16px;
    }

    tr + tr {
      border-top: 1px solid var(--v2-rule);
    }

    td {
      padding: 2px 0;
      border-top: 0;
    }
  }
`;

const LabCell = styled.td`
  width: 24%;
`;

const ShareCell = styled.td`
  width: 28%;
`;

/* The one accent on the row: the dither field, with its figure beside it. */
const ShareLine = styled.span`
  display: flex;
  align-items: center;
  gap: 12px;

  > span[role="img"] {
    flex-shrink: 0;
  }
`;

const LabName = styled.span`
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
`;

const LabLogo = styled.img`
  flex-shrink: 0;
  border-radius: var(--radius, 2px);
  object-fit: contain;
`;

const ShareValue = styled.span`
  color: var(--v2-text);
  font-size: 16px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const Cell = styled.td`
  @media (min-width: 981px) {
    &.wide {
      width: 22%;
    }
  }
`;

const MobileLabel = styled.span`
  display: block;
  color: var(--v2-text-muted);
  font-size: 13px;

  @media (min-width: 981px) {
    display: none;
  }
`;

const CellLink = styled(Link)`
  border-radius: var(--radius, 2px);
  color: var(--v2-text);
  font-weight: 500;
  text-decoration: underline;
  text-decoration-color: transparent;
  text-decoration-thickness: 0.08em;
  text-underline-offset: 0.18em;
  ${focusRing}

  &:hover {
    text-decoration-color: currentColor;
  }
`;

const CellSub = styled.span`
  display: block;
  color: var(--v2-text-muted);
  font-size: 13px;
`;

const ModelCell = styled.span`
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
`;

const PanelNote = styled.p`
  margin: 0;
  padding: 14px 20px 18px;
  color: var(--v2-text-muted);
  font-size: 13px;
  line-height: 1.5;
`;

const MoveList = styled.ul`
  margin: 0;
  padding: 0 8px 8px;
  list-style: none;

  li {
    display: grid;
    grid-template-columns: 116px minmax(0, 1fr) auto;
    gap: 16px;
    align-items: start;
    padding: 14px 12px;
    border-radius: var(--radius, 2px);
  }

  li + li {
    margin-top: 2px;
  }

  li:hover {
    background: var(--v2-row-hover);
  }

  @media (max-width: 760px) {
    li {
      grid-template-columns: minmax(0, 1fr) auto;
      grid-template-areas:
        "meta value"
        "body body";
      gap: 4px 12px;
    }
  }
`;

const MoveMeta = styled.div`
  display: flex;
  flex-direction: column;
  gap: 2px;
  color: var(--v2-text-muted);
  font-size: 13px;

  @media (max-width: 760px) {
    grid-area: meta;
  }
`;

const MoveTag = styled.span`
  color: var(--v2-text-subtle);
  font-size: 13px;
`;

const MoveBody = styled.div`
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;

  @media (max-width: 760px) {
    grid-area: body;
  }
`;

const MoveTitle = styled(Link)`
  border-radius: var(--radius, 2px);
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 500;
  text-decoration: none;
  ${focusRing}
`;

const MoveSub = styled.span`
  color: var(--v2-text-muted);
  font-size: 13px;
`;

const MoveValue = styled.div`
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  color: var(--v2-text);
  font-size: 15px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;

  @media (max-width: 760px) {
    grid-area: value;
  }
`;

const SourceLink = styled.a`
  border-radius: var(--radius, 2px);
  color: var(--v2-text-muted);
  font-size: 13px;
  text-decoration: underline;
  text-decoration-thickness: 0.08em;
  text-underline-offset: 0.18em;
  ${focusRing}

  &:hover {
    color: var(--v2-text);
  }
`;

const Empty = styled.p`
  margin: 0;
  padding: 20px;
  color: var(--v2-text-muted);
  font-size: 15px;
`;
