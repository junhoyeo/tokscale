"use client";

import { useState } from "react";
import styled from "styled-components";
import type { SupportedAgent } from "@/components/home/agents";

/** Phones show this many agents until the list is expanded; wider screens show all. */
const MOBILE_COUNT = 12;
const MOBILE_MAX = "680px";

export function AgentsGrid({ agents }: { agents: SupportedAgent[] }) {
  const [expanded, setExpanded] = useState(false);
  const collapsible = agents.length > MOBILE_COUNT;

  return (
    <div>
      <Grid id="supported-agents-list" data-expanded={expanded ? "true" : "false"}>
        {agents.map((agent) => (
          <li key={agent.key}>
            {/* eslint-disable-next-line @next/next/no-img-element -- small static icons */}
            <img src={agent.logoUrl} alt="" width={20} height={20} loading="lazy" decoding="async" />
            <span title={agent.name}>{agent.name}</span>
          </li>
        ))}
      </Grid>
      {collapsible ? (
        <Toggle
          type="button"
          aria-expanded={expanded}
          aria-controls="supported-agents-list"
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? "Show fewer" : `Show all ${agents.length}`}
        </Toggle>
      ) : null}
    </div>
  );
}

const Grid = styled.ul`
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(156px, 1fr));
  gap: 0 20px;
  margin: 0;
  padding: 0;
  list-style: none;

  li {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    min-height: 40px;
    padding: 0 10px;
    border-radius: var(--radius, 2px);
  }

  img {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    border-radius: var(--radius, 2px);
  }

  span {
    overflow: hidden;
    color: var(--v2-text);
    font-size: 14px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: ${MOBILE_MAX}) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0 16px;

    &[data-expanded="false"] li:nth-child(n + ${MOBILE_COUNT + 1}) {
      display: none;
    }
  }
`;

const Toggle = styled.button`
  display: none;
  min-height: 44px;
  margin-top: 12px;
  padding: 0 16px;
  border: 0;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 500;
  cursor: pointer;

  &:hover {
    background: var(--v2-surface-raised);
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 3px;
  }

  @media (max-width: ${MOBILE_MAX}) {
    display: inline-flex;
    align-items: center;
  }
`;
