"use client";

/**
 * "Sources and attribution": every external source used on the page, with
 * its license, retrieval date and credit line.
 */
import styled from "styled-components";
import type { SourceNote } from "@/lib/benchmarks/attribution";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { InlineLink, Section, SectionNote, SectionTitle } from "./ExploreUI";

const List = styled.ul`
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr));
  gap: 16px 32px;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const Item = styled.li`
  display: grid;
  align-content: start;
  gap: 4px;
  min-width: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Name = styled.a`
  color: var(--service-text);
  font-weight: 600;
  text-decoration: none;

  &:hover {
    color: var(--service-accent-hover);
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const License = styled.a`
  color: var(--service-accent-hover);
  text-decoration: none;

  &:hover {
    text-decoration: underline !important;
  }
`;

export function SourcesFooter({ notes, extra }: { notes: SourceNote[]; extra?: string }) {
  return (
    <Section aria-labelledby="sources-title">
      <div>
        <SectionTitle id="sources-title">Sources and attribution</SectionTitle>
        <SectionNote>
          Each score keeps its own source, unit and run conditions. Tokscale does not combine benchmarks into one score, and a
          missing score means the model was not evaluated, not that it scored zero.{extra ? ` ${extra}` : ""}
        </SectionNote>
      </div>
      <List>
        {notes.map((note) => (
          <Item key={note.id}>
            <Name href={note.url} target="_blank" rel="noopener noreferrer">
              {note.name}
            </Name>
            <span>{note.provides}.</span>
            <span>
              {note.licenseUrl ? (
                <License href={note.licenseUrl} target="_blank" rel="noopener noreferrer">
                  {note.license}
                </License>
              ) : (
                note.license
              )}
              {note.retrievedAt ? ` · Retrieved ${formatIsoDay(note.retrievedAt.slice(0, 10))}` : null}
              {note.dataDate ? ` · Data through ${formatIsoDay(note.dataDate)}` : null}
            </span>
            <span>{note.credit}</span>
          </Item>
        ))}
        <Item>
          <Name as="span">Tokscale</Name>
          <span>Active users, tokens and token mix from public Tokscale submissions. Hidden accounts are excluded.</span>
          <span>
            <InlineLink href="/methodology">How we count</InlineLink>
          </span>
        </Item>
      </List>
    </Section>
  );
}
