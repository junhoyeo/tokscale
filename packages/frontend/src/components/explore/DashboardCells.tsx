"use client";

/**
 * Cell renderers for the /models dashboard: usage, price estimates and
 * specs. Scores use ScoreValue. Missing values say what is missing.
 */
import styled from "styled-components";
import type { DashboardRow, PriceCell } from "@/lib/benchmarks/dashboard";
import { formatContext, formatIsoDay, formatPricePerMillion } from "@/lib/benchmarks/format";
import type { EffectivePrice, MixCounts } from "@/lib/benchmarks/price";
import type { ExplorePeriod } from "@/lib/explore";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { AgentMark, ChangeText, VisuallyHidden } from "./ExploreUI";
import { describeUsersChange, formatCount, formatShare, formatTokens, TOKEN_MIX_LABELS } from "./format";
import { InfoPopover, PopoverFacts, PopoverNote, PopoverTitle } from "./InfoPopover";

export const Dash = styled(InstantTooltip)`
  color: var(--service-text-muted);
`;

/** An em dash with a spoken and hover label, for values that do not exist. */
export function Missing({ label }: { label: string }) {
  return (
    <Dash content={label} describe={false}>
      <span aria-hidden="true">{"\u2014"}</span>
      <VisuallyHidden>{label}</VisuallyHidden>
    </Dash>
  );
}

const Stack = styled.span<{ $align: "start" | "end" }>`
  display: inline-grid;
  justify-items: ${({ $align }) => ($align === "end" ? "end" : "start")};
  min-width: 0;
  font-variant-numeric: tabular-nums;
`;

const Sub = styled.span`
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  white-space: nowrap;
`;

const Inline = styled.span`
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 4px;
`;

const InlineTip = styled(InstantTooltip)`
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 4px;
`;

export function UsersCell({ row, period, align = "end" }: { row: DashboardRow; period: ExplorePeriod; align?: "start" | "end" }) {
  if (!row.usage) return <Missing label="No Tokscale usage in this period" />;
  const change = describeUsersChange(row.usage.activeUsers, row.usage.previousActiveUsers, row.usage.isNew, period);
  return (
    <Stack $align={align}>
      <span>{formatCount(row.usage.activeUsers)}</span>
      <ChangeText change={change} />
    </Stack>
  );
}

export function TokensCell({ row }: { row: DashboardRow }) {
  if (!row.usage) return <Missing label="No Tokscale usage in this period" />;
  return <span>{formatTokens(row.usage.tokens)}</span>;
}

const AgentName = styled.span`
  overflow: hidden;
  color: var(--service-text);
  text-overflow: ellipsis;
  white-space: nowrap;
`;

export function TopAgentCell({ row }: { row: DashboardRow }) {
  const agent = row.usage?.topAgent;
  if (!agent) return <Missing label="No agent data in this period" />;
  return (
    <InlineTip content={`${agent.name} carries ${formatShare(agent.share)} of this model's tokens`}>
      <AgentMark agent={agent} size={16} />
      <AgentName>{agent.name}</AgentName>
      <Sub>{formatShare(agent.share)}</Sub>
    </InlineTip>
  );
}

export function ListPriceCell({ price, align = "end" }: { price: PriceCell | null; align?: "start" | "end" }) {
  if (!price) return <Missing label="No list price on models.dev" />;
  return (
    <Stack $align={align}>
      <span>
        {formatPricePerMillion(price.input)} / {formatPricePerMillion(price.output)}
      </span>
      <Sub>{price.providerName}</Sub>
    </Stack>
  );
}

export function CacheReadCell({ price }: { price: PriceCell | null }) {
  if (!price) return <Missing label="No list price on models.dev" />;
  if (price.cacheRead === null) return <Missing label="No cache-read price listed" />;
  return <span>{formatPricePerMillion(price.cacheRead)}</span>;
}

const MIX_ORDER: Array<keyof MixCounts> = ["cacheRead", "input", "cacheWrite", "output", "reasoning"];

function mixLine(mix: MixCounts): string {
  const total = MIX_ORDER.reduce((sum, k) => sum + Math.max(0, mix[k]), 0);
  if (total <= 0) return "No tokens";
  return MIX_ORDER.filter((k) => mix[k] > 0)
    .map((k) => `${TOKEN_MIX_LABELS[k]} ${formatShare(mix[k] / total)}`)
    .join(", ");
}

export function EstimateCell({
  row,
  which,
  communityMix,
  ownMix,
  align = "end",
}: {
  row: DashboardRow;
  which: "community" | "own";
  communityMix: MixCounts | null;
  ownMix: MixCounts | null;
  align?: "start" | "end";
}) {
  const price = row.price;
  if (!price) return <Missing label="No list price on models.dev" />;
  const estimate: EffectivePrice | null = which === "community" ? price.community : price.own;
  const mix = which === "community" ? communityMix : ownMix;
  if (!estimate || !mix) {
    const label =
      which === "own" && !row.usage
        ? "No Tokscale usage in this period, so there is no own token mix"
        : communityMix === null
          ? "Usage data is not available"
          : "A price needed for this mix is not listed";
    return <Missing label={label} />;
  }
  const mixName = which === "community" ? "the community token mix" : `${row.name}'s own token mix`;
  return (
    <Inline style={{ justifyContent: align === "end" ? "flex-end" : "flex-start" }}>
      <span>{formatPricePerMillion(estimate.perMillion)}</span>
      <InfoPopover label={`How the estimate for ${row.name} at ${mixName} is calculated`}>
        <PopoverTitle>Estimate at list prices</PopoverTitle>
        <PopoverFacts>
          <dt>Estimate</dt>
          <dd>{formatPricePerMillion(estimate.perMillion)} per 1M tokens</dd>
          <dt>Mix</dt>
          <dd>
            {mixName}: {mixLine(mix)}
          </dd>
          <dt>List price</dt>
          <dd>
            {price.providerName}: input {formatPricePerMillion(price.input)}, output {formatPricePerMillion(price.output)}, cache read{" "}
            {formatPricePerMillion(price.cacheRead)}, cache write {formatPricePerMillion(price.cacheWrite)} per 1M
          </dd>
        </PopoverFacts>
        <PopoverNote>
          The list price of each token type, weighted by its share of the mix. Reasoning tokens are priced as output. Discounts,
          batch prices and long-context tiers are not applied. Not what anyone paid.
        </PopoverNote>
        {estimate.assumptions.map((text) => (
          <PopoverNote key={text}>{text}</PopoverNote>
        ))}
      </InfoPopover>
    </Inline>
  );
}

export function ContextCell({ value }: { value: number | null }) {
  if (value === null) return <Missing label="Not listed on models.dev" />;
  return <span>{formatContext(value)}</span>;
}

export function DateCell({ value }: { value: string | null }) {
  if (!value) return <Missing label="Not listed on models.dev" />;
  return <span>{formatIsoDay(value)}</span>;
}

export function OpenWeightsCell({ value }: { value: boolean | null }) {
  if (value === null) return <Missing label="Not listed on models.dev" />;
  return <span>{value ? "Yes" : "No"}</span>;
}
