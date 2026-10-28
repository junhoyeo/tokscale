"use client";

/**
 * "Sources and attribution": every external source used on the page, with
 * its license, retrieval date and credit line. A real section: 16px heading,
 * 13px notes, source and licence names as ink links that underline on hover.
 */
import styled from "styled-components";
import type { SourceNote } from "@/lib/benchmarks/attribution";
import { formatIsoDay } from "@/lib/benchmarks/format";
import { Section, SectionNote, SectionTitle } from "./ExploreUI";

const List = styled.ul`
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr));
  gap: 16px 32px;
  margin: 0;
  padding: 0;
  list-style: none;
`;

/** One sentence at a readable measure, not the full table width. */
const Note = styled(SectionNote)`
  max-width: 84ch;
`;

const Item = styled.li`
  display: grid;
  align-content: start;
  gap: 3px;
  min-width: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const nameStyles = `
  color: var(--service-text);
  font-weight: 500;
  text-decoration: none;
`;

const sourceLinkStyles = `
  &:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const Name = styled.a`
  ${nameStyles}
  ${sourceLinkStyles}
  justify-self: start;
`;

const NameText = styled.span`
  ${nameStyles}
`;

const License = styled.a`
  color: inherit;
  text-decoration: none;

  &:hover {
    color: var(--service-text);
  }

  ${sourceLinkStyles}
`;

const Methodology = styled.a`
  color: var(--service-text);
  text-decoration: none;

  &:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

export function SourcesFooter({ notes, extra }: { notes: SourceNote[]; extra?: string }) {
  return (
    <Section aria-labelledby="sources-title">
      <div>
        <SectionTitle id="sources-title">Sources and attribution</SectionTitle>
        <Note>
          Each score keeps its own source, unit and run conditions. Tokscale does not combine benchmarks into one score, and a
          missing score means the model was not evaluated, not that it scored zero.{extra ? ` ${extra}` : ""}
        </Note>
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
          <NameText>Tokscale</NameText>
          <span>Active users, tokens and token mix from public Tokscale submissions. Hidden accounts are excluded.</span>
          <span>
            <Methodology href="/methodology">How we count</Methodology>
          </span>
        </Item>
      </List>
    </Section>
  );
}
