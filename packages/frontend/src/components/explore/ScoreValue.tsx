"use client";

/**
 * One benchmark score: the value, a small condition tag (effort, agent or
 * harness) and a source popover with the run date, interval and a link to
 * the source. A missing score reads "Not evaluated", never zero.
 *
 * The value is ink in tabular figures; the condition tag stays grey at 12px
 * and never grows into a second line.
 */
import styled from "styled-components";
import type { BenchmarkInfo } from "@/lib/benchmarks/attribution";
import type { ScoreCell } from "@/lib/benchmarks/dashboard";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { InfoPopover, PopoverFacts, PopoverLink, PopoverNote, PopoverTitle } from "./InfoPopover";
import { VisuallyHidden } from "./ExploreUI";

const Wrap = styled.span<{ $align: "start" | "end" }>`
  display: inline-flex;
  min-width: 0;
  align-items: center;
  justify-content: ${({ $align }) => ($align === "end" ? "flex-end" : "flex-start")};
  gap: 4px;
`;

const Stack = styled.span<{ $align: "start" | "end" }>`
  display: grid;
  justify-items: ${({ $align }) => ($align === "end" ? "end" : "start")};
  min-width: 0;
`;

const Value = styled.span`
  color: var(--service-text);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const Tag = styled(InstantTooltip)`
  max-width: 14em;
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const Missing = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  white-space: nowrap;
`;

const DATE_LABELS = { run: "Run date", published: "Published", release: "Model released" } as const;

export function ScoreDetails({ cell, info, modelName }: { cell: ScoreCell; info: BenchmarkInfo; modelName: string }) {
  return (
    <>
      <PopoverTitle>
        {info.name}: {cell.text}
      </PopoverTitle>
      <PopoverFacts>
        <dt>Model</dt>
        <dd>{modelName}</dd>
        <dt>Listed as</dt>
        <dd>{cell.sourceName}</dd>
        {cell.tag ? (
          <>
            <dt>Conditions</dt>
            <dd>{cell.tag}</dd>
          </>
        ) : (
          <>
            <dt>Conditions</dt>
            <dd>Not reported</dd>
          </>
        )}
        {cell.interval ? (
          <>
            <dt>Interval</dt>
            <dd>{cell.interval}</dd>
          </>
        ) : null}
        <dt>Unit</dt>
        <dd>{info.unitLabel}</dd>
        {cell.date && cell.dateKind ? (
          <>
            <dt>{DATE_LABELS[cell.dateKind]}</dt>
            <dd>{formatIsoDay(cell.date)}</dd>
          </>
        ) : null}
        {info.retrievedAt ? (
          <>
            <dt>Retrieved</dt>
            <dd>{formatIsoDay(info.retrievedAt.slice(0, 10))}</dd>
          </>
        ) : null}
      </PopoverFacts>
      {cell.resultCount > 1 ? <PopoverNote>Best of {cell.resultCount} reported results for this model.</PopoverNote> : null}
      <PopoverNote>
        <PopoverLink href={cell.sourceUrl} target="_blank" rel="noopener noreferrer">
          Source
        </PopoverLink>{" "}
        · {info.attribution}
      </PopoverNote>
    </>
  );
}

export function ScoreValue({
  cell,
  info,
  modelName,
  align = "end",
  showTag = true,
}: {
  cell: ScoreCell | null;
  info: BenchmarkInfo;
  modelName: string;
  align?: "start" | "end";
  showTag?: boolean;
}) {
  if (!cell) {
    return (
      <Missing>
        Not evaluated
        <VisuallyHidden> on {info.name}</VisuallyHidden>
      </Missing>
    );
  }
  return (
    <Wrap $align={align}>
      <Stack $align={align}>
        <Value>{cell.text}</Value>
        {showTag && cell.tag ? (
          <Tag content={cell.tag} describe={false}>
            {cell.tag}
          </Tag>
        ) : null}
      </Stack>
      <InfoPopover label={`Source for ${modelName} on ${info.name}`}>
        <ScoreDetails cell={cell} info={info} modelName={modelName} />
      </InfoPopover>
    </Wrap>
  );
}
