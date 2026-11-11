"use client";

import styled, { css } from "styled-components";
import type { SupportedAgent } from "@/components/home/agents";
import { Arrow, TextLink } from "@/components/home/primitives";

const ICON_COUNT = 4;
/* Below this width the flow stacks and the arrows point down. */
const STACK = "980px";

/**
 * How Tokscale works, drawn as a flow: your agents' logs and the tokscale CLI sit inside a
 * dotted "Your machine" boundary; only `tokscale submit` crosses it, with counts.
 */
export function FlowDiagram({ agents }: { agents: SupportedAgent[] }) {
  const shown = agents.slice(0, ICON_COUNT);
  const named = agents.slice(0, 3).map((agent) => agent.name);
  const more = Math.max(0, agents.length - named.length);

  return (
    <Flow>
      <Machine>
        <BoundaryLabel>Your Machine</BoundaryLabel>
        <Step>
          <StepNumber aria-hidden="true">1</StepNumber>
          <StepTitle>Your agents&apos; logs</StepTitle>
          <Icons aria-hidden="true">
            {shown.map((agent) => (
              // eslint-disable-next-line @next/next/no-img-element -- small static icons
              <img key={agent.key} src={agent.logoUrl} alt="" width={28} height={28} loading="lazy" decoding="async" />
            ))}
            {agents.length > shown.length ? <MoreIcons>+{agents.length - shown.length}</MoreIcons> : null}
          </Icons>
          <StepBody>
            The session logs your coding agents already keep on your disk: {named.join(", ")}
            {more > 0 ? ` and ${more} more` : ""}.
          </StepBody>
        </Step>

        <Connector aria-hidden="true">
          <ConnectorLabel>reads</ConnectorLabel>
        </Connector>

        <Step>
          <StepNumber aria-hidden="true">2</StepNumber>
          <StepTitle>Tokscale, locally</StepTitle>
          <StepBody>
            Opens a terminal dashboard of your usage across agents, models, days, and sessions. Your usage data stays
            on your machine.
          </StepBody>
          <Commands>
            <code>bunx tokscale@latest</code>
          </Commands>
        </Step>
      </Machine>

      <Crossing>
        <Connector aria-hidden="true" $strong />
        <CrossingLabel>
          <code>tokscale submit</code>
          <span>sends counts, never content</span>
        </CrossingLabel>
      </Crossing>

      <Public>
        <BoundaryLabel $public>tokscale.ai · Public</BoundaryLabel>
        <Step>
          <StepNumber aria-hidden="true">3</StepNumber>
          <StepTitle>Your public profile</StepTitle>
          <StepBody>
            Sign in with GitHub once, then submit, or let your scheduler submit every 24 hours. Add{" "}
            <InlineCode>--dry-run</InlineCode> to preview the payload first. Your profile lives at{" "}
            <Nowrap>tokscale.ai/u/your-username</Nowrap>.
          </StepBody>
          <Commands>
            <code>tokscale login</code>
            <code>tokscale submit</code>
            <code>tokscale autosubmit enable</code>
          </Commands>
          <StepLinks>
            <TextLink href="/leaderboard">
              Leaderboards <Arrow />
            </TextLink>
            <TextLink href="/leaderboard?view=groups">
              Groups <Arrow />
            </TextLink>
          </StepLinks>
        </Step>
      </Public>
    </Flow>
  );
}

const Flow = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 2.1fr) 132px minmax(0, 1fr);
  align-items: stretch;

  @media (max-width: ${STACK}) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

/* Dotted, like the dot-matrix art: everything inside stays on the machine. */
const Machine = styled.div`
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 72px minmax(0, 1fr);
  padding: 28px 20px 20px;
  border: 2px dotted rgba(79, 138, 230, 0.55);
  border-radius: 18px;

  @media (max-width: ${STACK}) {
    grid-template-columns: minmax(0, 1fr);
    padding: 28px 14px 14px;
  }
`;

const Public = styled.div`
  position: relative;
  display: flex;
  padding: 28px 0 0;

  > article {
    flex: 1;
    border-color: var(--v2-rule-strong);
  }
`;

const BoundaryLabel = styled.span<{ $public?: boolean }>`
  position: absolute;
  top: -10px;
  left: 20px;
  padding: 0 8px;
  background: var(--v2-canvas);
  color: #4f8ae6;
  font-size: 13px;
  font-weight: 600;
  line-height: 20px;

  ${({ $public }) =>
    $public &&
    css`
      top: 0;
      left: 0;
      padding: 0;
      color: var(--v2-text-subtle);
    `}
`;

const Step = styled.article`
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
  padding: 24px;
  border: 1px solid var(--v2-rule);
  border-radius: 12px;
  background: var(--v2-surface);

  @media (max-width: 520px) {
    padding: 20px 16px;
  }
`;

const StepNumber = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: 1px solid var(--v2-rule-strong);
  border-radius: 999px;
  color: var(--v2-text-muted);
  font-size: 13px;
  font-weight: 600;
`;

const StepTitle = styled.h3`
  margin: 0;
  color: var(--v2-text);
  font-family: var(--v2-font-display);
  font-size: 28px;
  font-weight: 400;
  line-height: 1.15;
`;

const StepBody = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 15px;
  line-height: 1.6;
`;

const Icons = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;

  img {
    display: block;
    width: 28px;
    height: 28px;
    border-radius: 7px;
  }
`;

const MoreIcons = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 28px;
  height: 28px;
  padding: 0 6px;
  border: 1px solid var(--v2-rule-strong);
  border-radius: 7px;
  color: var(--v2-text-muted);
  font-size: 12px;
  font-weight: 600;
`;

/* A dotted arrow: horizontal between steps, pointing down when the flow stacks. */
const Connector = styled.div<{ $strong?: boolean }>`
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 48px;

  &::before {
    position: absolute;
    top: 50%;
    right: 10px;
    left: 6px;
    height: 4px;
    margin-top: -2px;
    background: radial-gradient(circle, ${({ $strong }) => ($strong ? "var(--v2-accent)" : "#4f8ae6")} 1.6px, transparent 2px) 0 50% / 8px 4px repeat-x;
    content: "";
  }

  &::after {
    position: absolute;
    top: 50%;
    right: 4px;
    width: 8px;
    height: 8px;
    margin-top: -4px;
    border-top: 2px solid ${({ $strong }) => ($strong ? "var(--v2-accent)" : "#4f8ae6")};
    border-right: 2px solid ${({ $strong }) => ($strong ? "var(--v2-accent)" : "#4f8ae6")};
    transform: rotate(45deg);
    content: "";
  }

  @media (max-width: ${STACK}) {
    min-height: 56px;

    &::before {
      top: 6px;
      right: auto;
      bottom: 10px;
      left: 50%;
      width: 4px;
      height: auto;
      margin: 0 0 0 -2px;
      background-position: 50% 0;
      background-size: 4px 8px;
      background-repeat: repeat-y;
    }

    &::after {
      top: auto;
      right: auto;
      bottom: 6px;
      left: 50%;
      margin: 0 0 0 -4px;
      transform: rotate(135deg);
    }
  }
`;

const ConnectorLabel = styled.span`
  position: relative;
  z-index: 1;
  padding: 0 6px;
  background: var(--v2-canvas);
  color: var(--v2-text-subtle);
  font-size: 12px;

  @media (max-width: ${STACK}) {
    margin-left: 72px;
    background: transparent;
  }
`;

const Crossing = styled.div`
  display: flex;
  flex-direction: column;
  align-items: stretch;
  justify-content: center;
  gap: 8px;
  padding: 28px 8px 0;

  @media (max-width: ${STACK}) {
    flex-direction: row;
    align-items: center;
    gap: 0;
    padding: 0;

    > div {
      flex: 0 0 48px;
    }
  }
`;

const CrossingLabel = styled.p`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  margin: 0;
  text-align: center;

  code {
    color: var(--v2-text);
    font-size: 13px;
    white-space: nowrap;
  }

  span {
    color: var(--v2-text-subtle);
    font-size: 12px;
    line-height: 1.4;
  }

  @media (max-width: ${STACK}) {
    flex-direction: row;
    align-items: baseline;
    gap: 8px;
    text-align: left;
  }
`;

const Commands = styled.div`
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: auto;
  padding-top: 8px;

  code {
    display: block;
    overflow-x: auto;
    padding: 8px 12px;
    border: 1px solid var(--v2-rule);
    border-radius: 6px;
    background: #000;
    color: var(--v2-text);
    font-size: 13px;
    white-space: nowrap;
  }
`;

const InlineCode = styled.code`
  padding: 1px 6px;
  white-space: nowrap;
  border: 1px solid var(--v2-rule);
  border-radius: 4px;
  color: var(--v2-text);
  font-size: 13px;
`;

const Nowrap = styled.span`
  color: var(--v2-text);
  white-space: nowrap;
`;

const StepLinks = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  padding-top: 4px;
`;
