"use client";

/**
 * Specs section of a model page, from models.dev. A value models.dev does not
 * list reads "—" with a "not listed" label, never zero.
 */
import styled from "styled-components";
import type { ModelSpecsView } from "@/lib/benchmarks/modelPage";
import { formatContext, formatIsoDay, formatModalities } from "@/lib/benchmarks/format";
import { Section, SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";

const List = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0 16px;
  margin: 0;

  @media (min-width: 720px) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0 32px;
  }

  @media (min-width: 1200px) {
    grid-template-columns: repeat(5, minmax(0, 1fr));
  }
`;

const Item = styled.div`
  display: grid;
  align-content: start;
  gap: 2px;
  min-width: 0;
  padding: 10px 0;
  border-bottom: 1px solid var(--service-border);

  dt {
    color: var(--service-text-muted);
    font-size: 0.8125rem;
  }

  dd {
    margin: 0;
    color: var(--service-text);
    font-size: 0.9375rem;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  @media (max-width: 640px) {
    dt {
      font-size: 0.875rem;
    }

    dd {
      font-size: 1rem;
    }
  }
`;

const Missing = styled.span`
  color: var(--service-text-muted);
`;

const Empty = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

function NotListed() {
  return (
    <Missing>
      —<VisuallyHidden> not listed</VisuallyHidden>
    </Missing>
  );
}

function yesNo(value: boolean | null) {
  return value === null ? <NotListed /> : value ? "Yes" : "No";
}

function tokens(value: number | null) {
  return value === null ? <NotListed /> : `${formatContext(value)} tokens`;
}

function day(value: string | null) {
  return value === null ? <NotListed /> : formatIsoDay(value);
}

function modalities(values: readonly string[]) {
  return values.length === 0 ? <NotListed /> : formatModalities(values);
}

export function ModelSpecs({ specs, modelName }: { specs: ModelSpecsView | null; modelName: string }) {
  if (!specs) {
    return (
      <Section aria-labelledby="specs-title">
        <SectionTitle id="specs-title">Specs</SectionTitle>
        <Empty>models.dev does not list {modelName}, so Tokscale has no specs for it.</Empty>
      </Section>
    );
  }
  const items = [
    { label: "Context window", value: tokens(specs.context) },
    { label: "Max output", value: tokens(specs.maxOutput) },
    { label: "Input", value: modalities(specs.inputModalities) },
    { label: "Output", value: modalities(specs.outputModalities) },
    { label: "Knowledge cutoff", value: day(specs.knowledgeCutoff) },
    { label: "Release date", value: day(specs.releaseDate) },
    { label: "Open weights", value: yesNo(specs.openWeights) },
    { label: "Reasoning", value: yesNo(specs.reasoning) },
    { label: "Tool calling", value: yesNo(specs.toolCall) },
  ];
  return (
    <Section aria-labelledby="specs-title">
      <div>
        <SectionTitle id="specs-title">Specs</SectionTitle>
        <SectionNote>
          From models.dev (MIT), retrieved {formatIsoDay(specs.retrievedAt.slice(0, 10))}, as the lab or the first listed
          provider reports them. A dash means models.dev does not list the value.
        </SectionNote>
      </div>
      <List>
        {items.map((item) => (
          <Item key={item.label}>
            <dt>{item.label}</dt>
            <dd>{item.value}</dd>
          </Item>
        ))}
      </List>
    </Section>
  );
}
