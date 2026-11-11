"use client";

import Link from "next/link";
import styled, { css } from "styled-components";
import { ModelMark } from "@/components/explore/ModelMark";
import type { HomeCapital, HomeCapitalLab } from "@/lib/capital/ledger/home";
import { coverageRangeLabel, wholeNumber } from "./format";
import { Arrow, Container, Eyebrow, focusRing, Lead, Section, SectionHeader, SectionTitle, TextLink, VisuallyHidden } from "./primitives";
import { titleCase } from "@/lib/text/titleCase";

interface CapitalPreviewProps {
  data: HomeCapital | null;
}

function sharePercent(share: number): string {
  if (!Number.isFinite(share) || share <= 0) return "0%";
  if (share < 0.001) return "<0.1%";
  return `${(share * 100).toFixed(1)}%`;
}

export function CapitalPreview({ data }: CapitalPreviewProps) {
  return (
    <Section aria-labelledby="home-capital-title">
      <Container>
        <Head>
          <SectionHeader>
            <Eyebrow>Capital</Eyebrow>
            <SectionTitle id="home-capital-title">The money behind the models you use.</SectionTitle>
            <Lead>
              Funding rounds, compute contracts and capital spending across AI labs, clouds and chips. Every number comes
              from a filing or a company announcement.
            </Lead>
          </SectionHeader>
          <TextLink href="/capital">
            Explore capital <Arrow />
          </TextLink>
        </Head>

        {data ? (
          <>
            <Stats aria-label="Four money types">
              {data.stats.map((stat) => (
                <Stat key={stat.label}>
                  <StatLabel>{titleCase(stat.label)}</StatLabel>
                  <StatValue>{stat.value}</StatValue>
                  <StatDetail>{stat.detail}</StatDetail>
                  <StatLink href={stat.href}>
                    {stat.linkLabel} <Arrow />
                  </StatLink>
                </Stat>
              ))}
            </Stats>
            <StatsNote>
              These are four different money types. They are never added together.{" "}
              <InlineLink href="/capital/methodology">How we count</InlineLink>
            </StatsNote>

            <Panels>
              <Panel aria-labelledby="home-capital-labs-title">
                <PanelHead>
                  <PanelTitle id="home-capital-labs-title">Labs behind the models you use</PanelTitle>
                  <PanelLink href="/capital/companies">
                    All companies <Arrow />
                  </PanelLink>
                </PanelHead>
                {data.labs.length > 0 ? (
                  <>
                    <LabHeader aria-hidden="true">
                      <span>Lab</span>
                      <Right>Tokscale tokens</Right>
                      <span>Top model</span>
                      <span>Last round</span>
                      <span>Largest compute deal</span>
                    </LabHeader>
                    <LabList>
                      {data.labs.map((lab) => (
                        <LabRow key={lab.slug} lab={lab} />
                      ))}
                    </LabList>
                    <PanelNote>
                      Tokscale tokens: share of tokens from identified models among{" "}
                      {data.labsWindow ? `${wholeNumber(data.labsWindow.accounts)} Tokscale accounts, ${coverageRangeLabel(data.labsWindow) ?? "last 30 days"}` : "Tokscale accounts, last 30 days"}
                      . Not market share. Valuations and contracts are company-stated or from filings.
                    </PanelNote>
                  </>
                ) : (
                  <Empty>Tokscale usage data is not available right now.</Empty>
                )}
              </Panel>

              <Panel aria-labelledby="home-capital-moves-title">
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
                        <Tag>{event.tag}</Tag>
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
    <LabItem data-row-link={lab.href ? "" : undefined}>
      <LabName>
        <OrgTile aria-hidden="true">
          {lab.logo ? (
            <OrgLogo src={lab.logo} alt="" width={16} height={16} loading="lazy" />
          ) : lab.avatar ? (
            <OrgAvatar src={lab.avatar} alt="" width={26} height={26} loading="lazy" />
          ) : (
            lab.monogram
          )}
        </OrgTile>
        {lab.href ? (
          <CellLink href={lab.href} data-row-primary="">
            {lab.name}
          </CellLink>
        ) : (
          <span>{lab.name}</span>
        )}
      </LabName>
      <Share>
        <VisuallyHidden>Share of Tokscale tokens: </VisuallyHidden>
        <ShareValue>{sharePercent(lab.share)}</ShareValue>
        <ShareTrack aria-hidden="true">
          <ShareFill style={{ width: `${Math.max(2, Math.min(100, lab.share * 100))}%` }} />
        </ShareTrack>
      </Share>
      <Cell>
        <VisuallyHidden>Top model: </VisuallyHidden>
        {lab.topModel ? (
          <ModelCell>
            <ModelMark model={lab.topModel} size={18} />
            <CellLink href={lab.topModel.href}>{lab.topModel.name}</CellLink>
          </ModelCell>
        ) : (
          "—"
        )}
      </Cell>
      <Cell>
        <VisuallyHidden>Last round: </VisuallyHidden>
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
      <Cell>
        <VisuallyHidden>Largest compute deal: </VisuallyHidden>
        <MobileLabel aria-hidden="true">Largest compute deal</MobileLabel>
        {lab.compute ? (
          <>
            {lab.compute.href ? <CellLink href={lab.compute.href}>{lab.compute.value}</CellLink> : <span>{lab.compute.value}</span>}
            {lab.compute.with ? <CellSub>with {lab.compute.with}</CellSub> : null}
          </>
        ) : (
          <CellSub>None tracked</CellSub>
        )}
      </Cell>
    </LabItem>
  );
}

const Head = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: 24px;
  margin-bottom: 32px;
`;

const Stats = styled.div`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  border: 1px solid var(--v2-rule-strong);
  border-radius: 12px;
  background: rgba(4, 5, 8, 0.86);

  @media (max-width: 900px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
`;

const Stat = styled.div`
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  padding: 18px 20px;

  & + & {
    border-left: 1px solid var(--v2-rule);
  }

  @media (max-width: 900px) {
    &:nth-child(3) {
      border-left: 0;
    }

    &:nth-child(n + 3) {
      border-top: 1px solid var(--v2-rule);
    }
  }

  @media (max-width: 520px) {
    padding: 14px 16px;
  }
`;

/* The shared eyebrow above each number. */
const StatLabel = styled(Eyebrow)``

const StatValue = styled.p`
  margin: 0;
  color: var(--v2-numeric);
  font-size: 28px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  line-height: 1.15;

  @media (max-width: 520px) {
    font-size: 24px;
  }
`;

const StatDetail = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 13px;
  line-height: 1.45;
`;

const StatLink = styled(Link)`
  display: inline-flex;
  gap: 6px;
  align-items: center;
  align-self: flex-start;
  min-height: 32px;
  margin-top: auto;
  border-radius: 4px;
  color: var(--v2-text);
  font-size: 13px;
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
  font-size: 12px;
`;

const InlineLink = styled(Link)`
  border-radius: 3px;
  color: var(--v2-text-muted);
  text-decoration: underline;
  text-decoration-color: var(--v2-rule-strong);
  text-underline-offset: 3px;
  ${focusRing}

  &:hover {
    color: var(--v2-text);
  }
`;

const Panels = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 20px;
  margin-top: 28px;
`;

/* Clips the row hover backgrounds to the rounded corners; row focus rings sit inside. */
const Panel = styled.section`
  --row-focus-offset: -2px;

  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--v2-rule-strong);
  border-radius: 12px;
  background: rgba(4, 5, 8, 0.86);
`;

const PanelHead = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 20px;
  border-bottom: 1px solid var(--v2-rule);

  @media (max-width: 520px) {
    padding: 12px 16px;
  }
`;

const PanelTitle = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-family: var(--v2-font-display);
  font-size: 20px;
  font-weight: 400;
  line-height: 1.2;
`;

const PanelLink = styled(Link)`
  display: inline-flex;
  flex-shrink: 0;
  gap: 6px;
  align-items: center;
  min-height: 32px;
  border-radius: 4px;
  color: var(--v2-text-muted);
  font-size: 13px;
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

const labGrid = css`
  display: grid;
  grid-template-columns: minmax(0, 1fr) 120px minmax(0, 1.2fr) minmax(0, 1.2fr) minmax(0, 1.1fr);
  gap: 20px;
  align-items: center;
`;

const LabHeader = styled.div`
  ${labGrid}
  padding: 10px 20px 4px;
  color: var(--v2-text-subtle);
  font-size: 12px;
  font-weight: 500;

  @media (max-width: 760px) {
    display: none;
  }
`;

const Right = styled.span`
  text-align: right;
`;

const LabList = styled.ol`
  margin: 0;
  padding: 0 0 4px;
  list-style: none;
`;

const LabItem = styled.li`
  ${labGrid}
  min-height: 56px;
  padding: 8px 20px;

  & + & {
    border-top: 1px solid var(--v2-rule);
  }

  @media (max-width: 760px) {
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 6px 12px;
    padding: 12px 16px;

    & > :nth-child(n + 3) {
      grid-column: 1 / -1;
    }

    /* Phones show the top three labs; All companies lists the rest. */
    &:nth-child(n + 4) {
      display: none;
    }
  }
`;

const LabName = styled.span`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 10px;
  font-size: 15px;
  font-weight: 500;
`;

const OrgTile = styled.span`
  display: inline-grid;
  flex: 0 0 auto;
  width: 26px;
  height: 26px;
  place-items: center;
  overflow: hidden;
  border: 1px solid var(--v2-rule-strong);
  border-radius: 6px;
  background: var(--v2-surface-raised);
  color: var(--v2-text-muted);
  font-size: 11px;
  font-weight: 600;
`;

const OrgLogo = styled.img`
  display: block;
  width: 16px;
  height: 16px;
`;

/* The company's own square picture fills the tile. */
const OrgAvatar = styled.img`
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
`;

const CellLink = styled(Link)`
  overflow: hidden;
  border-radius: 3px;
  color: var(--v2-text);
  text-decoration: none;
  text-overflow: ellipsis;
  white-space: nowrap;
  ${focusRing}

  &:hover {
    color: #8cc2ff;
  }
`;

const Share = styled.span`
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;

  @media (max-width: 760px) {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
`;

const ShareValue = styled.span`
  color: var(--v2-text);
  font-size: 14px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
`;

const ShareTrack = styled.span`
  display: block;
  width: 72px;
  height: 3px;
  overflow: hidden;
  border-radius: 2px;
  background: var(--v2-rule);

  @media (max-width: 760px) {
    width: 48px;
  }
`;

const ShareFill = styled.span`
  display: block;
  height: 100%;
  background: var(--v2-accent);
`;

const Cell = styled.span`
  display: flex;
  min-width: 0;
  flex-direction: column;
  color: var(--v2-text);
  font-size: 14px;

  @media (max-width: 760px) {
    flex-direction: row;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 8px;
  }
`;

/* Column names for the stacked phone rows (the header row is hidden there). */
const MobileLabel = styled.span`
  display: none;
  color: var(--v2-text-subtle);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;

  @media (max-width: 760px) {
    display: block;
    flex-basis: 100%;
  }
`;

const ModelCell = styled.span`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
`;

const CellSub = styled.span`
  overflow: hidden;
  color: var(--v2-text-subtle);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const PanelNote = styled.p`
  margin: 0;
  padding: 10px 20px 14px;
  border-top: 1px solid var(--v2-rule);
  color: var(--v2-text-subtle);
  font-size: 12px;
  line-height: 1.5;

  @media (max-width: 520px) {
    padding: 10px 16px 14px;
  }
`;

const MoveList = styled.ol`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin: 0;
  padding: 0;
  list-style: none;

  li {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 8px;
    padding: 14px 20px 16px;
  }

  li + li {
    border-left: 1px solid var(--v2-rule);
  }

  @media (max-width: 1100px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));

    li:nth-child(3) {
      border-left: 0;
    }

    li:nth-child(n + 3) {
      border-top: 1px solid var(--v2-rule);
    }
  }

  @media (max-width: 600px) {
    grid-template-columns: minmax(0, 1fr);

    li {
      padding: 12px 16px 14px;
    }

    li + li {
      border-left: 0;
      border-top: 1px solid var(--v2-rule);
    }
  }
`;

const MoveMeta = styled.span`
  display: flex;
  gap: 10px;
  align-items: baseline;
  color: var(--v2-text-subtle);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
`;

const Tag = styled.span`
  color: var(--v2-text-muted);
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
`;

const MoveBody = styled.span`
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
`;

const MoveTitle = styled(Link)`
  display: -webkit-box;
  overflow: hidden;
  border-radius: 3px;
  color: var(--v2-text);
  font-size: 14px;
  font-weight: 500;
  line-height: 1.35;
  text-decoration: none;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  ${focusRing}

  &:hover {
    color: #8cc2ff;
  }
`;

const MoveSub = styled.span`
  color: var(--v2-text-subtle);
  font-size: 12px;
`;

const MoveValue = styled.span`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 2px 10px;
  margin-top: auto;
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
`;

const SourceLink = styled.a`
  margin-left: auto;
  border-radius: 3px;
  color: var(--v2-text-muted);
  font-size: 12px;
  font-weight: 400;
  text-decoration: none;
  ${focusRing}

  &:hover {
    color: var(--v2-text);
  }
`;

const Empty = styled.p`
  margin: 0;
  padding: 20px;
  color: var(--v2-text-muted);
  font-size: 14px;
`;
