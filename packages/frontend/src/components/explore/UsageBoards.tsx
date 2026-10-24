"use client";

/**
 * Models and Agents rankings: dense tables on wide screens, and
 * information-complete stacked rows below them (no horizontal scrolling).
 */
import Link from "next/link";
import styled from "styled-components";
import type { AgentUsageRow, Coverage, ExplorePeriod, ExploreSort, ModelUsageRow } from "@/lib/explore";
import { modelPath } from "@/lib/seo/explore";
import {
  describeUsersChange,
  formatCount,
  estimatedCostPhrase,
  formatEstimatedCost,
  formatShare,
  formatTokens,
  periodLabel,
  adoptionNote,
  sortLabel,
} from "./format";
import { UNKNOWN_LAB_NAME } from "./catalog";
import { AgentMark, ChangeText, EmptyNote, InlineLink, ModelMark, ShareMeter, VisuallyHidden } from "./ExploreUI";

/* The model table needs about 800px; narrower screens show stacked rows instead. */
const TABLE_FROM = "840px";

// Agent names and their top-model lists need more room than the model board.
const AGENT_TABLE_FROM = "1100px";

const TableWrap = styled.div<{ $from?: string }>`
  display: none;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);

  @media (min-width: ${({ $from }) => $from ?? TABLE_FROM}) {
    display: block;
  }
`;

const Table = styled.table`
  width: 100%;
  border-collapse: collapse;
  table-layout: auto;
`;

const Th = styled.th<{ $align?: "right" }>`
  padding: 10px 12px;
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 500;
  text-align: ${({ $align }) => $align ?? "left"};
  white-space: nowrap;

  &[aria-sort] {
    color: var(--service-text);
  }

  &:first-child {
    padding-left: 0;
  }

  &:last-child {
    padding-right: 0;
  }
`;

const Td = styled.td<{ $align?: "right" }>`
  padding: 12px;
  vertical-align: middle;
  color: var(--service-text);
  font-size: 0.875rem;
  font-variant-numeric: tabular-nums;
  text-align: ${({ $align }) => $align ?? "left"};

  &:first-child {
    padding-left: 0;
  }

  &:last-child {
    padding-right: 0;
  }
`;

const Tr = styled.tr`
  &:not(:last-child) td {
    border-bottom: 1px solid var(--service-border);
  }

  /* Agent rows have no page: a light reading aid instead of the row-link hover. */
  &:not([data-row-link]):hover td {
    background: rgba(255, 255, 255, 0.02);
  }
`;

const Rank = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
`;

const ModelIdentity = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 10px;
`;

const Identity = styled.div`
  display: grid;
  min-width: 0;
  gap: 2px;
`;

const NameLink = styled(Link)`
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  text-decoration: none;
  text-overflow: ellipsis;
  white-space: nowrap;

  &:hover {
    color: var(--service-accent-hover);
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

const NameText = styled.span`
  display: grid;
  grid-template-columns: 20px minmax(0, 1fr);
  min-width: 0;
  align-items: center;
  gap: 8px;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  overflow-wrap: anywhere;
`;

const SubText = styled.span`
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const UsersCell = styled.div`
  display: grid;
  justify-items: end;
  gap: 2px;
`;

const UsersValue = styled.span`
  font-size: 0.9375rem;
  font-weight: 600;
`;

const MeterCell = styled.div`
  width: 150px;
`;

const TopList = styled.ul`
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const TopItem = styled.li`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--service-text);
  font-size: 0.8125rem;
  white-space: nowrap;
`;

const TopShare = styled.span`
  color: var(--service-text-muted);
  font-variant-numeric: tabular-nums;
`;

const TopModelLink = styled(Link)`
  color: var(--service-text);
  text-decoration: none;

  &:hover {
    color: var(--service-accent-hover);
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

const Footnote = styled.p`
  margin: 12px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

/* ----- Mobile rows ----- */

const MobileList = styled.ol<{ $until?: string }>`
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);

  @media (min-width: ${({ $until }) => $until ?? TABLE_FROM}) {
    display: none;
  }
`;

const MobileItem = styled.li`
  list-style: none;

  &:not(:last-child) {
    border-bottom: 1px solid var(--service-border);
  }
`;

const MobileRow = styled.div`
  position: relative;
  display: grid;
  grid-template-columns: 32px minmax(0, 1fr) auto;
  gap: 4px 10px;
  align-items: start;
  min-height: 88px;
  padding: 14px 4px;

  &:has(a:active) {
    background: rgba(255, 255, 255, 0.03);
  }
`;

/**
 * The model name is the row's only link. It is the row's primary link (row
 * links, globals.css): its ::after covers the whole row, so the touch target
 * is the full row (88px+) while the link's accessible name stays just the
 * model name.
 */
const MobileNameLink = styled(Link)`
  overflow: hidden;
  color: var(--service-text);
  text-decoration: none;
  text-overflow: ellipsis;
`;

const MobileRank = styled(Rank)`
  grid-row: 1 / span 4;
  padding-top: 2px;
  font-size: 0.875rem;
`;

const MobileName = styled.span`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
  overflow: hidden;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const MobileNameText = styled.span`
  overflow: hidden;
  text-overflow: ellipsis;
`;

const MobilePrimary = styled.span`
  display: grid;
  justify-items: end;
  grid-row: 1 / span 2;
  grid-column: 3;
`;

const MobilePrimaryValue = styled.span`
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const MobilePrimaryLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.75rem;
  white-space: nowrap;
`;

const MobileSub = styled.span`
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px 8px;
  grid-column: 2;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const MobileWide = styled.span`
  display: block;
  grid-column: 2 / 4;
  min-width: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
  line-height: 1.45;
`;

const MobileMeterLabel = styled.span`
  display: flex;
  align-items: center;
  gap: 10px;

  > span:first-child {
    flex: 0 0 auto;
  }
`;

interface BoardProps<Row> {
  rows: Row[];
  coverage: Coverage;
  period: ExplorePeriod;
  sort: ExploreSort;
}

function caption(kind: string, sort: ExploreSort, period: ExplorePeriod) {
  return `${kind} ranked by ${sortLabel(sort).toLowerCase()}, ${periodLabel(period).toLowerCase()}`;
}

function UsersHeader({ sort }: { sort: ExploreSort }) {
  return (
    <Th scope="col" $align="right" aria-sort={sort === "users" ? "descending" : undefined}>
      Active users
    </Th>
  );
}

function TokensHeader({ sort }: { sort: ExploreSort }) {
  return (
    <Th scope="col" $align="right" aria-sort={sort === "tokens" ? "descending" : undefined}>
      Tokens
    </Th>
  );
}

export function ModelBoard({ rows, coverage, period, sort }: BoardProps<ModelUsageRow>) {
  if (rows.length === 0) {
    return <EmptyNote>No model usage in this period yet.</EmptyNote>;
  }

  return (
    <>
      <TableWrap>
        <Table>
          <caption>
            <VisuallyHidden>{caption("Models", sort, period)}</VisuallyHidden>
          </caption>
          <thead>
            <tr>
              <Th scope="col">#</Th>
              <Th scope="col">Model</Th>
              <UsersHeader sort={sort} />
              <Th scope="col">Adoption</Th>
              <TokensHeader sort={sort} />
              <Th scope="col" $align="right">Est. cost</Th>
              <Th scope="col">Top agents</Th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const change = describeUsersChange(row.activeUsers, row.previousActiveUsers, row.isNew, period);
              return (
                <Tr key={row.key} data-row-link="">
                  <Td>
                    <Rank>{row.rank}</Rank>
                  </Td>
                  <Td>
                    <ModelIdentity>
                      <ModelMark model={row} size={28} />
                      <Identity>
                        <NameLink href={modelPath(row.key)} data-row-primary="">
                          {row.name}
                        </NameLink>
                        <SubText>{row.lab ?? UNKNOWN_LAB_NAME}</SubText>
                      </Identity>
                    </ModelIdentity>
                  </Td>
                  <Td $align="right">
                    <UsersCell>
                      <UsersValue>{formatCount(row.activeUsers)}</UsersValue>
                      <ChangeText change={change} />
                    </UsersCell>
                  </Td>
                  <Td>
                    <MeterCell>
                      <ShareMeter share={row.adoption} label="of active accounts" />
                    </MeterCell>
                  </Td>
                  <Td $align="right">{formatTokens(row.tokens)}</Td>
                  <Td $align="right">{formatEstimatedCost(row.estimatedCost, row.tokens)}</Td>
                  <Td>
                    <TopList aria-label={`Top agents for ${row.name}`}>
                      {row.topAgents.map((agent) => (
                        <TopItem key={agent.key}>
                          <AgentMark agent={agent} size={16} />
                          {agent.name}
                          <TopShare>{formatShare(agent.share)}</TopShare>
                        </TopItem>
                      ))}
                    </TopList>
                  </Td>
                </Tr>
              );
            })}
          </tbody>
        </Table>
      </TableWrap>

      <MobileList aria-label={caption("Models", sort, period)}>
        {rows.map((row) => {
          const change = describeUsersChange(row.activeUsers, row.previousActiveUsers, row.isNew, period);
          const primaryIsUsers = sort === "users";
          return (
            <MobileItem key={row.key}>
              <MobileRow data-row-link="">
                <MobileRank>{row.rank}</MobileRank>
                <MobileName>
                  <ModelMark model={row} size={20} />
                  <MobileNameLink href={modelPath(row.key)} data-row-primary="">
                    {row.name}
                  </MobileNameLink>
                </MobileName>
                <MobilePrimary>
                  <MobilePrimaryValue>
                    {primaryIsUsers ? formatCount(row.activeUsers) : formatTokens(row.tokens)}
                  </MobilePrimaryValue>
                  <MobilePrimaryLabel>{primaryIsUsers ? "active users" : "tokens"}</MobilePrimaryLabel>
                </MobilePrimary>
                <MobileSub>
                  <span>{row.lab ?? UNKNOWN_LAB_NAME}</span>
                  <ChangeText change={change} />
                </MobileSub>
                <MobileWide>
                  <MobileMeterLabel>
                    <span>Adoption</span>
                    <ShareMeter share={row.adoption} label="of active accounts" />
                  </MobileMeterLabel>
                </MobileWide>
                <MobileWide>
                  {primaryIsUsers
                    ? `${formatTokens(row.tokens)} tokens`
                    : `${formatCount(row.activeUsers)} active users`}
                  {" · "}
                  {estimatedCostPhrase(row.estimatedCost, row.tokens)}
                </MobileWide>
                {row.topAgents.length > 0 ? (
                  <MobileWide>
                    Top agents: {row.topAgents.map((agent) => `${agent.name} ${formatShare(agent.share)}`).join(", ")}
                  </MobileWide>
                ) : null}
              </MobileRow>
            </MobileItem>
          );
        })}
      </MobileList>

      <Footnote>
        {adoptionNote(rows.map((row) => row.adoption), coverage.accounts, "model")}{" "}
        Top agent percentages are each agent&apos;s share of the model&apos;s tokens. Cost is estimated from public list prices.{" "}
        <InlineLink href="/methodology">Methodology</InlineLink>
      </Footnote>
    </>
  );
}

export function AgentBoard({ rows, coverage, period, sort }: BoardProps<AgentUsageRow>) {
  if (rows.length === 0) {
    return <EmptyNote>No agent usage in this period yet.</EmptyNote>;
  }

  return (
    <>
      <TableWrap $from={AGENT_TABLE_FROM}>
        <Table>
          <caption>
            <VisuallyHidden>{caption("Agents", sort, period)}</VisuallyHidden>
          </caption>
          <thead>
            <tr>
              <Th scope="col">#</Th>
              <Th scope="col">Agent</Th>
              <UsersHeader sort={sort} />
              <Th scope="col">Adoption</Th>
              <TokensHeader sort={sort} />
              <Th scope="col" $align="right">Est. cost</Th>
              <Th scope="col">Top models</Th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const change = describeUsersChange(row.activeUsers, row.previousActiveUsers, row.isNew, period);
              return (
                <Tr key={row.key}>
                  <Td>
                    <Rank>{row.rank}</Rank>
                  </Td>
                  <Td>
                    <Identity>
                      <NameText>
                        <AgentMark agent={row} size={20} />
                        <span>{row.name}</span>
                      </NameText>
                      <SubText>
                        Client id <code>{row.key}</code>
                      </SubText>
                    </Identity>
                  </Td>
                  <Td $align="right">
                    <UsersCell>
                      <UsersValue>{formatCount(row.activeUsers)}</UsersValue>
                      <ChangeText change={change} />
                    </UsersCell>
                  </Td>
                  <Td>
                    <MeterCell>
                      <ShareMeter share={row.adoption} label="of active accounts" />
                    </MeterCell>
                  </Td>
                  <Td $align="right">{formatTokens(row.tokens)}</Td>
                  <Td $align="right">{formatEstimatedCost(row.estimatedCost, row.tokens)}</Td>
                  <Td>
                    <TopList aria-label={`Top models in ${row.name}`}>
                      {row.topModels.map((model) => (
                        <TopItem key={model.key}>
                          <ModelMark model={model} size={16} />
                          <TopModelLink href={modelPath(model.key)}>{model.name}</TopModelLink>
                          <TopShare>{formatShare(model.share)}</TopShare>
                        </TopItem>
                      ))}
                    </TopList>
                  </Td>
                </Tr>
              );
            })}
          </tbody>
        </Table>
      </TableWrap>

      <MobileList $until={AGENT_TABLE_FROM} aria-label={caption("Agents", sort, period)}>
        {rows.map((row) => {
          const change = describeUsersChange(row.activeUsers, row.previousActiveUsers, row.isNew, period);
          const primaryIsUsers = sort === "users";
          return (
            <MobileItem key={row.key}>
              <MobileRow>
                <MobileRank>{row.rank}</MobileRank>
                <MobileName>
                  <AgentMark agent={row} size={20} />
                  <MobileNameText>{row.name}</MobileNameText>
                </MobileName>
                <MobilePrimary>
                  <MobilePrimaryValue>
                    {primaryIsUsers ? formatCount(row.activeUsers) : formatTokens(row.tokens)}
                  </MobilePrimaryValue>
                  <MobilePrimaryLabel>{primaryIsUsers ? "active users" : "tokens"}</MobilePrimaryLabel>
                </MobilePrimary>
                <MobileSub>
                  <ChangeText change={change} />
                </MobileSub>
                <MobileWide>
                  <MobileMeterLabel>
                    <span>Adoption</span>
                    <ShareMeter share={row.adoption} label="of active accounts" />
                  </MobileMeterLabel>
                </MobileWide>
                <MobileWide>
                  {primaryIsUsers
                    ? `${formatTokens(row.tokens)} tokens`
                    : `${formatCount(row.activeUsers)} active users`}
                  {" · "}
                  {estimatedCostPhrase(row.estimatedCost, row.tokens)}
                </MobileWide>
                {row.topModels.length > 0 ? (
                  <MobileWide>
                    Top models: {row.topModels.map((model, index) => (
                      <span key={model.key}>
                        {index > 0 ? ", " : ""}
                        <TopModelLink href={modelPath(model.key)}>{model.name}</TopModelLink>{" "}
                        {formatShare(model.share)}
                      </span>
                    ))}
                  </MobileWide>
                ) : null}
              </MobileRow>
            </MobileItem>
          );
        })}
      </MobileList>

      <Footnote>
        {adoptionNote(rows.map((row) => row.adoption), coverage.accounts, "agent")}{" "}
        Top model percentages are each model&apos;s share of the agent&apos;s tokens. Cost is estimated from public list prices.{" "}
        <InlineLink href="/methodology">Methodology</InlineLink>
      </Footnote>
    </>
  );
}
