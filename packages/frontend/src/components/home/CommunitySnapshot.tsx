"use client";

import Link from "next/link";
import styled, { css } from "styled-components";
import type { CommunitySnapshot as Snapshot, HumanRow, ModelUsageRow } from "@/lib/explore";
import { AgentMark } from "@/components/explore/ExploreUI";
import { ModelMark } from "@/components/explore/ModelMark";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { Arrow, focusRing, VisuallyHidden } from "./primitives";
import { compactCost, compactNumber, userChange, wholeNumber } from "./format";

interface CommunitySnapshotProps {
  snapshot: Snapshot | null;
}

export function CommunitySnapshot({ snapshot }: CommunitySnapshotProps) {
  const humans = snapshot?.humans.slice(0, 5) ?? [];
  const models = snapshot?.models.slice(0, 5) ?? [];

  return (
    <Panel aria-label="Community snapshot">
      <Columns>
        <Column aria-labelledby="snapshot-humans-title">
          <ColumnHead>
            <ColumnTitle id="snapshot-humans-title">Humans</ColumnTitle>
            <ColumnLink href="/leaderboard">
              All humans <Arrow />
            </ColumnLink>
          </ColumnHead>
          <HeaderRow $kind="humans" aria-hidden="true">
            <span>#</span>
            <span>Builder</span>
            <HideNarrow>Top model and agent</HideNarrow>
            <Right>Tokens</Right>
            <Right>Est. cost</Right>
          </HeaderRow>
          {humans.length > 0 ? (
            <List>
              {humans.map((human) => (
                <HumanItem key={human.username} human={human} />
              ))}
            </List>
          ) : (
            <Empty>No public usage in this period yet.</Empty>
          )}
        </Column>

        <Column aria-labelledby="snapshot-models-title">
          <ColumnHead>
            <ColumnTitle id="snapshot-models-title">Models</ColumnTitle>
            <ColumnLink href="/models">
              All models <Arrow />
            </ColumnLink>
          </ColumnHead>
          <HeaderRow $kind="models" aria-hidden="true">
            <span>#</span>
            <span>Model</span>
            <Right>Users</Right>
            <Right>vs prev.</Right>
            <Right>Est. cost</Right>
          </HeaderRow>
          {models.length > 0 ? (
            <List>
              {models.map((model) => (
                <ModelItem key={model.key} model={model} />
              ))}
            </List>
          ) : (
            <Empty>No model usage in this period yet.</Empty>
          )}
        </Column>
      </Columns>
    </Panel>
  );
}

function HumanItem({ human }: { human: HumanRow }) {
  const name = human.displayName || human.username;
  const topModel = human.topModels[0];
  const topAgent = human.topAgents[0];
  return (
    <li>
      <RowLink href={`/u/${encodeURIComponent(human.username)}`} $kind="humans">
        <Rank>{human.rank}</Rank>
        <Identity>
          <AvatarImg
            src={human.avatarUrl || `https://github.com/${encodeURIComponent(human.username)}.png?size=48`}
            alt=""
            width={22}
            height={22}
            loading="lazy"
          />
          {/* The handle is in the tooltip when the row shows a display name. */}
          <Name content={name !== human.username ? `@${human.username}` : null} describe={false}>
            {name}
          </Name>
          {human.displayName && human.displayName !== human.username ? (
            <VisuallyHidden> (@{human.username})</VisuallyHidden>
          ) : null}
        </Identity>
        <Stack>
          <VisuallyHidden>Top model and agent: </VisuallyHidden>
          {topModel ? (
            <>
              <ModelMark model={topModel} size={18} />
              <StackText>{topModel.name}</StackText>
            </>
          ) : (
            <StackText>—</StackText>
          )}
          {topAgent ? (
            <AgentChip content={topAgent.name} describe={false}>
              <AgentMark agent={topAgent} size={16} />
              <VisuallyHidden>, {topAgent.name}</VisuallyHidden>
            </AgentChip>
          ) : null}
        </Stack>
        <Value>
          <VisuallyHidden>Tokens: </VisuallyHidden>
          {compactNumber(human.tokens)}
        </Value>
        <Cost>
          <VisuallyHidden>Estimated cost: </VisuallyHidden>
          {compactCost(human.estimatedCost, human.tokens)}
        </Cost>
      </RowLink>
    </li>
  );
}

function ModelItem({ model }: { model: ModelUsageRow }) {
  const change = userChange(model);
  return (
    <li>
      <RowLink href={`/models/${encodeURIComponent(model.key)}`} $kind="models">
        <Rank>{model.rank}</Rank>
        <Identity>
          <ModelMark model={model} size={22} />
          {/* The lab is in the tooltip. */}
          <Name content={model.lab} describe={false}>
            {model.name}
          </Name>
          {model.lab ? <VisuallyHidden> by {model.lab}</VisuallyHidden> : null}
        </Identity>
        <Value>
          <VisuallyHidden>Active users: </VisuallyHidden>
          {wholeNumber(model.activeUsers)}
        </Value>
        <Change $kind={change.kind}>
          <VisuallyHidden>Change in active users against the previous period: </VisuallyHidden>
          {change.kind === "new" ? <NewBadge>New</NewBadge> : change.label}
        </Change>
        <Cost>
          <VisuallyHidden>Estimated cost: </VisuallyHidden>
          {compactCost(model.estimatedCost, model.tokens)}
        </Cost>
      </RowLink>
    </li>
  );
}

const Panel = styled.section`
  position: relative;
  border: 1px solid var(--v2-rule-strong);
  border-radius: 12px;
  background: rgba(4, 5, 8, 0.86);
  backdrop-filter: blur(2px);
  -webkit-backdrop-filter: blur(2px);
`;

const Columns = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);

  @media (max-width: 960px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const Column = styled.section`
  min-width: 0;
  padding: 4px 0 8px;

  & + & {
    border-left: 1px solid var(--v2-rule);
  }

  @media (max-width: 960px) {
    & + & {
      border-top: 1px solid var(--v2-rule);
      border-left: 0;
    }
  }
`;

const ColumnHead = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 20px 2px;

  @media (max-width: 520px) {
    padding: 8px 16px 2px;
  }
`;

const ColumnTitle = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-family: var(--v2-font-display);
  font-size: 20px;
  font-weight: 400;
  line-height: 1.2;
`;

const ColumnLink = styled(Link)`
  display: inline-flex;
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

/* One line per row: rank, identity, (stack), and right-aligned numbers. */
const narrowHumans = "(max-width: 1100px) and (min-width: 961px), (max-width: 600px)";

const humansGrid = css`
  grid-template-columns: 16px minmax(0, 1fr) minmax(0, 1fr) 52px 56px;

  @media ${narrowHumans} {
    grid-template-columns: 16px minmax(0, 1fr) 52px 56px;
  }
`;

const modelsGrid = css`
  grid-template-columns: 16px minmax(0, 1fr) 48px 52px 56px;

  @media (max-width: 400px) {
    grid-template-columns: 14px minmax(0, 1fr) 34px 40px 58px;
  }
`;

const HeaderRow = styled.div<{ $kind: "humans" | "models" }>`
  display: grid;
  ${({ $kind }) => ($kind === "humans" ? humansGrid : modelsGrid)}
  gap: 10px;
  padding: 4px 20px;
  color: var(--v2-text-subtle);
  font-size: 12px;
  font-weight: 500;

  @media (max-width: 520px) {
    gap: 8px;
    padding: 4px 16px;
  }
`;

const HideNarrow = styled.span`
  @media ${narrowHumans} {
    display: none;
  }
`;

const Right = styled.span`
  text-align: right;
  white-space: nowrap;
`;

const List = styled.ol`
  margin: 0;
  padding: 0;
  list-style: none;

  li + li a {
    border-top: 1px solid var(--v2-rule);
  }
`;

const RowLink = styled(Link)<{ $kind: "humans" | "models" }>`
  display: grid;
  ${({ $kind }) => ($kind === "humans" ? humansGrid : modelsGrid)}
  gap: 10px;
  align-items: center;
  min-height: 44px;
  margin: 0 10px;
  padding: 6px 10px;
  border-radius: 6px;
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
    gap: 8px;
    margin: 0 4px;
    padding: 6px 12px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const Rank = styled.span`
  color: var(--v2-text-subtle);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
`;

const Identity = styled.span`
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
`;

const AvatarImg = styled.img`
  flex-shrink: 0;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--v2-surface-raised);
  box-shadow: 0 0 0 1px var(--v2-rule-strong);
`;

const ellipsis = css`
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const Name = styled(InstantTooltip)`
  ${ellipsis}
  min-width: 0;
  color: var(--v2-text);
  font-size: 14px;
  font-weight: 500;
`;

const Stack = styled.span`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 6px;
  color: var(--v2-text-muted);
  font-size: 13px;

  @media ${narrowHumans} {
    display: none;
  }
`;

const StackText = styled.span`
  ${ellipsis}
  min-width: 0;
`;

const AgentChip = styled(InstantTooltip)`
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  margin-left: 2px;
`;

const Value = styled.span`
  color: var(--v2-text);
  font-size: 14px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  text-align: right;
`;

const Cost = styled.span`
  color: var(--v2-text-muted);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  text-align: right;
`;

const Change = styled.span<{ $kind: string }>`
  color: ${({ $kind }) => ($kind === "up" ? "var(--v2-numeric)" : "var(--v2-text-muted)")};
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  text-align: right;
`;

const NewBadge = styled.span`
  display: inline-flex;
  align-items: center;
  height: 20px;
  padding: 0 7px;
  border: 1px solid rgba(0, 115, 255, 0.55);
  border-radius: 999px;
  background: var(--v2-accent-soft);
  color: #8cc2ff;
  font-size: 11px;
  font-weight: 600;
`;

const Empty = styled.p`
  margin: 0;
  padding: 20px;
  color: var(--v2-text-muted);
  font-size: 14px;
`;
