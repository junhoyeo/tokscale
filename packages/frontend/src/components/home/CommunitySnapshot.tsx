"use client";

import Link from "next/link";
import styled, { css } from "styled-components";
import type { CommunitySnapshot as Snapshot, HumanRow, ModelUsageRow } from "@/lib/explore";
import { ModelMark } from "@/components/explore/ModelMark";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { Arrow, focusRing, VisuallyHidden } from "./primitives";
import { compactCost, compactNumber, userChange, wholeNumber } from "./format";

interface CommunitySnapshotProps {
  snapshot: Snapshot | null;
}

/**
 * The live leaderboard, as one cropped fragment of the product (data-ui):
 * builders by tokens and models by active users, from the last 30 days of
 * submitted usage. Rows sit on a tone step, never on rules.
 */
export function CommunitySnapshot({ snapshot }: CommunitySnapshotProps) {
  const humans = snapshot?.humans.slice(0, 5) ?? [];
  const models = snapshot?.models.slice(0, 5) ?? [];

  return (
    <Board data-ui aria-label="Community snapshot">
      <Group aria-labelledby="snapshot-humans-title">
        <GroupHead>
          <GroupTitle id="snapshot-humans-title">Builders</GroupTitle>
          <GroupLink href="/leaderboard">
            All <Arrow />
          </GroupLink>
        </GroupHead>
        <HeadRow $kind="humans" aria-hidden="true">
          <span />
          <span>Builder</span>
          <Right>Tokens</Right>
          <Right $cost>Est. cost</Right>
        </HeadRow>
        {humans.length > 0 ? (
          <List>
            {humans.map((human) => (
              <HumanItem key={human.username} human={human} />
            ))}
          </List>
        ) : (
          <Empty>No public usage in this period yet.</Empty>
        )}
      </Group>

      <Group aria-labelledby="snapshot-models-title">
        <GroupHead>
          <GroupTitle id="snapshot-models-title">Models</GroupTitle>
          <GroupLink href="/models">
            All <Arrow />
          </GroupLink>
        </GroupHead>
        <HeadRow $kind="models" aria-hidden="true">
          <span />
          <span>Model</span>
          <Right>Users</Right>
          <Right>vs prev.</Right>
        </HeadRow>
        {models.length > 0 ? (
          <List>
            {models.map((model) => (
              <ModelItem key={model.key} model={model} />
            ))}
          </List>
        ) : (
          <Empty>No model usage in this period yet.</Empty>
        )}
      </Group>
    </Board>
  );
}

function HumanItem({ human }: { human: HumanRow }) {
  const name = human.displayName || human.username;
  return (
    <li>
      <Row href={`/u/${encodeURIComponent(human.username)}`} $kind="humans">
        <Rank>{human.rank}</Rank>
        <Identity>
          <Avatar
            src={human.avatarUrl || `https://github.com/${encodeURIComponent(human.username)}.png?size=48`}
            alt=""
            width={22}
            height={22}
            loading="lazy"
          />
          <Name content={name !== human.username ? `@${human.username}` : null} describe={false}>
            {name}
          </Name>
          {human.displayName && human.displayName !== human.username ? (
            <VisuallyHidden> (@{human.username})</VisuallyHidden>
          ) : null}
        </Identity>
        <Value>
          <VisuallyHidden>Tokens: </VisuallyHidden>
          {compactNumber(human.tokens)}
        </Value>
        <Cost $cost>
          <VisuallyHidden>Estimated cost: </VisuallyHidden>
          {compactCost(human.estimatedCost, human.tokens)}
        </Cost>
      </Row>
    </li>
  );
}

function ModelItem({ model }: { model: ModelUsageRow }) {
  const change = userChange(model);
  return (
    <li>
      <Row href={`/models/${encodeURIComponent(model.key)}`} $kind="models">
        <Rank>{model.rank}</Rank>
        <Identity>
          <ModelMark model={model} size={20} />
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
      </Row>
    </li>
  );
}

/* The boards stack: builders, then models, inside the hero card's column. */
const Board = styled.section`
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
`;

const Group = styled.div`
  min-width: 0;
  padding: 8px 6px 10px;
`;

const GroupHead = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 0 8px 4px;
`;

const GroupTitle = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 600;
  letter-spacing: -0.005em;
`;

const GroupLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 28px;
  border-radius: 2px;
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

const humansGrid = css`
  grid-template-columns: 14px minmax(0, 1fr) 52px 56px;

  @media (max-width: 560px) {
    grid-template-columns: 14px minmax(0, 1fr) 52px;
  }
`;

const modelsGrid = css`
  grid-template-columns: 14px minmax(0, 1fr) 44px 48px;
`;

const HeadRow = styled.div<{ $kind: "humans" | "models" }>`
  display: grid;
  ${({ $kind }) => ($kind === "humans" ? humansGrid : modelsGrid)}
  gap: 8px;
  padding: 2px 8px 4px;
  color: var(--v2-text-subtle);
  font-size: 12px;
  font-weight: 500;
`;

const Right = styled.span<{ $cost?: boolean }>`
  text-align: right;
  white-space: nowrap;

  @media (max-width: 560px) {
    display: ${({ $cost }) => ($cost ? "none" : "inline")};
  }
`;

const List = styled.ol`
  margin: 0;
  padding: 0;
  list-style: none;

  li + li {
    margin-top: 2px;
  }
`;

const Row = styled(Link)<{ $kind: "humans" | "models" }>`
  display: grid;
  ${({ $kind }) => ($kind === "humans" ? humansGrid : modelsGrid)}
  gap: 8px;
  align-items: center;
  min-height: 32px;
  padding: 3px 8px;
  border-radius: var(--radius, 2px);
  color: inherit;
  text-decoration: none;
  transition: background-color 140ms ease;

  @media (hover: hover) {
    &:hover {
      background: var(--v2-row-hover);
    }
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: -2px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const Rank = styled.span`
  color: var(--v2-text-subtle);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
`;

const Identity = styled.span`
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
`;

/* Real GitHub avatars stay; square, like everything else on the page. */
const Avatar = styled.img`
  flex-shrink: 0;
  width: 22px;
  height: 22px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface-raised);
  object-fit: cover;
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
  font-size: 15px;
  font-weight: 500;
`;

const Value = styled.span`
  color: var(--v2-text);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  text-align: right;
`;

const Cost = styled.span<{ $cost?: boolean }>`
  color: var(--v2-text-muted);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  text-align: right;

  @media (max-width: 560px) {
    display: none;
  }
`;

const Change = styled.span<{ $kind: string }>`
  color: ${({ $kind }) => ($kind === "up" ? "var(--v2-text)" : "var(--v2-text-muted)")};
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  text-align: right;
`;

const NewBadge = styled.span`
  display: inline-flex;
  align-items: center;
  height: 20px;
  padding: 0 6px;
  border-radius: var(--radius, 2px);
  background: var(--v2-accent-soft);
  color: #8cc2ff;
  font-size: 12px;
  font-weight: 600;
`;

const Empty = styled.p`
  margin: 0;
  padding: 12px 8px;
  color: var(--v2-text-muted);
  font-size: 15px;
`;
