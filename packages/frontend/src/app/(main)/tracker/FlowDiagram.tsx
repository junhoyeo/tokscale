"use client";

import { useId, useState } from "react";
import styled from "styled-components";
import Link from "next/link";
import type { SupportedAgent } from "@/components/home/agents";
import { focusRing } from "@/components/home/primitives";

const ICON_COUNT = 4;

interface Stage {
  name: string;
  clause: string;
  note: string;
}

/**
 * How Tokscale works, as four stages you drag through: run it, it reads your
 * logs, you read the numbers, you submit counts. One range input drives the
 * clause and the fragment beside it; the arrow keys work too.
 */
const STAGES: Stage[] = [
  { name: "Install", clause: "First, run one command", note: "One command from your terminal. Nothing is installed globally." },
  { name: "Read", clause: "Then, read your logs", note: "The session logs your coding agents already keep on your disk." },
  { name: "Review", clause: "Next, read the numbers", note: "Tokens, models and estimated cost across agents, days and sessions." },
  { name: "Submit", clause: "Finally, submit counts", note: "Sign in with GitHub once, then send counts, never content." },
];

export function FlowDiagram({ agents }: { agents: SupportedAgent[] }) {
  const [stage, setStage] = useState(0);
  const [changing, setChanging] = useState(false);
  const id = useId();
  const shown = agents.slice(0, ICON_COUNT);
  const named = agents.slice(0, 3).map((agent) => agent.name);
  const more = Math.max(0, agents.length - named.length);
  const current = STAGES[stage];

  const go = (next: number) => {
    if (next === stage) return;
    setChanging(true);
    setStage(next);
  };

  return (
    <Split>
      <ClauseField>
        <Clause>
          <ClauseText id={`${id}-clause`}>{current.clause}</ClauseText>
          <ClauseNote>{current.note}</ClauseNote>
        </Clause>
      </ClauseField>

      <FragmentField>
        <FieldLabel aria-hidden="true">{current.name}</FieldLabel>
        <Fragment $visible={stage === 0} data-ui aria-hidden={stage !== 0}>
          <Terminal>
            <TerminalBar>
              <span>tokscale</span>
            </TerminalBar>
            <TerminalBody>
              <Command>
                <Prompt aria-hidden="true">$</Prompt> bunx tokscale@latest
              </Command>
              <Line>Reads your local logs. No account needed.</Line>
            </TerminalBody>
          </Terminal>
        </Fragment>

        <Fragment $visible={stage === 1} data-ui aria-hidden={stage !== 1}>
          <Machine>
            <MachineLabel>Your Machine</MachineLabel>
            <Icons aria-hidden="true">
              {shown.map((agent) => (
                // eslint-disable-next-line @next/next/no-img-element -- small static icons
                <img key={agent.key} src={agent.logoUrl} alt="" width={28} height={28} loading="lazy" decoding="async" />
              ))}
              {agents.length > shown.length ? <MoreIcons>+{agents.length - shown.length}</MoreIcons> : null}
            </Icons>
            <MachineText>
              The session logs your coding agents already keep on your disk: {named.join(", ")}
              {more > 0 ? ` and ${more} more` : ""}.
            </MachineText>
          </Machine>
        </Fragment>

        <Fragment $visible={stage === 2} data-ui aria-hidden={stage !== 2}>
          <Shot>
            {/* eslint-disable-next-line @next/next/no-img-element -- static screenshot with its own chrome */}
            <img
              src="/assets/v2/tui/tui-overview.webp"
              alt=""
              width={1800}
              height={1270}
              loading="lazy"
              decoding="async"
            />
            <ShotNote>The terminal UI, Overview view, on your machine.</ShotNote>
          </Shot>
        </Fragment>

        <Fragment $visible={stage === 3} data-ui aria-hidden={stage !== 3}>
          <SubmitBox>
            <SubmitLabel>
              <code>tokscale submit</code>
              <span>sends counts, never content</span>
            </SubmitLabel>
            <Commands>
              <code>tokscale login</code>
              <code>tokscale submit</code>
              <code>tokscale autosubmit enable</code>
            </Commands>
            <MachineText>
              Add <code>--dry-run</code> to preview the payload. Your profile lives at tokscale.ai/u/your-username.
            </MachineText>
            <Rows href="/leaderboard">See the leaderboards →</Rows>
          </SubmitBox>
        </Fragment>
      </FragmentField>

      <SliderRow>
        <Track>
          <TrackRule aria-hidden="true" />
          <TrackFill aria-hidden="true" $progress={stage / (STAGES.length - 1)} />
          <Range
            type="range"
            min={0}
            max={STAGES.length - 1}
            step={1}
            value={stage}
            aria-label="How it works, stage"
            aria-valuetext={current.name}
            aria-describedby={`${id}-clause`}
            onChange={(event) => go(Number(event.target.value))}
            data-changing={changing ? "true" : undefined}
          />
        </Track>
        <Stops role="group" aria-label="Stages">
          {STAGES.map((item, index) => (
            <StopButton
              key={item.name}
              type="button"
              $at={index / (STAGES.length - 1)}
              $active={index === stage}
              aria-current={index === stage ? "step" : undefined}
              onClick={() => go(index)}
            >
              {item.name}
            </StopButton>
          ))}
        </Stops>
      </SliderRow>
    </Split>
  );
}

const Split = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr) auto;
  grid-template-areas:
    "clause fragment"
    "slider slider";
  gap: 0;
  min-height: 0;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
  overflow: hidden;

  @media (max-width: 860px) {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto auto auto;
    grid-template-areas:
      "fragment"
      "clause"
      "slider";
    min-height: 0;
  }
`;

/* The clause and its note sit as one block, centred in the field, so the air
   around them is symmetrical rather than a hole at the bottom. */
const ClauseField = styled.div`
  grid-area: clause;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  justify-content: center;
  gap: 24px;
  padding: 12px 32px;

  @media (max-width: 860px) {
    padding: 24px 20px 32px;
  }
`;

const Clause = styled.div`
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 100%;
`;

const ClauseText = styled.h3`
  max-width: 18ch;
  margin: 0;
  color: var(--v2-text);
  font-size: clamp(30px, 4.2vw, 60px);
  font-weight: 500;
  line-height: 1.04;
  letter-spacing: -0.025em;
  text-wrap: balance;

  @media (max-width: 860px) {
    font-size: clamp(30px, 8.4vw, 40px);
  }
`;

const ClauseNote = styled.p`
  max-width: 32ch;
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 16px;
  line-height: 1.55;
`;

const FragmentField = styled.div`
  grid-area: fragment;
  position: relative;
  display: grid;
  place-items: center;
  min-height: 300px;
  padding: 52px 32px 32px;
  background: var(--v2-canvas);

  @media (max-width: 860px) {
    min-height: 0;
    height: auto;
    padding: 24px 16px 28px;
  }
`;

/* The stage's name sits in the field's top-left corner, like a plate label. */
const FieldLabel = styled.span`
  position: absolute;
  top: 20px;
  left: 32px;
  color: var(--v2-text-muted);
  font-size: 15px;
  font-weight: 500;

  @media (max-width: 860px) {
    top: 16px;
    left: 16px;
  }
`;

const Fragment = styled.div<{ $visible: boolean }>`
  grid-area: 1 / 1;
  opacity: ${({ $visible }) => ($visible ? 1 : 0)};
  visibility: ${({ $visible }) => ($visible ? "visible" : "hidden")};
  transition: opacity 200ms ease;
  width: min(100%, 520px);

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const Terminal = styled.div`
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
`;

const TerminalBar = styled.div`
  padding: 8px 14px;
  color: var(--v2-text-muted);
  font-size: 13px;
`;

const TerminalBody = styled.div`
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px 14px 20px;
`;

const Line = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 15px;
  line-height: 1.5;
`;

/* The literal command, the one place monospace is correct. */
const Command = styled.code`
  display: block;
  color: var(--v2-text);
  font-size: 15px;
  line-height: 1.5;
`;

const Prompt = styled.span`
  color: var(--v2-accent);
  user-select: none;
`;

const Machine = styled.div`
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
`;

const MachineLabel = styled.span`
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 600;
`;

const Icons = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;

  img {
    display: block;
    width: 28px;
    height: 28px;
    border-radius: var(--radius, 2px);
  }
`;

const MoreIcons = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 28px;
  height: 28px;
  padding: 0 6px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface-raised);
  color: var(--v2-text-muted);
  font-size: 13px;
  font-weight: 600;
`;

const MachineText = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 15px;
  line-height: 1.55;

  code {
    color: var(--v2-text);
    font-size: 13px;
  }
`;

const Shot = styled.figure`
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 0;

  img {
    display: block;
    width: auto;
    max-width: 100%;
    max-height: 250px;
    margin: 0 auto;
    border-radius: var(--radius, 2px);
  }
`;

const ShotNote = styled.figcaption`
  color: var(--v2-text-muted);
  font-size: 14px;
`;

const SubmitBox = styled.div`
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
`;

const SubmitLabel = styled.div`
  display: flex;
  flex-direction: column;
  gap: 4px;

  code {
    color: var(--v2-text);
    font-size: 15px;
    font-weight: 600;
  }

  span {
    color: var(--v2-text-muted);
    font-size: 14px;
  }
`;

const Commands = styled.div`
  display: flex;
  flex-direction: column;
  gap: 6px;

  code {
    color: var(--v2-text);
    font-size: 14px;
  }
`;

const Rows = styled(Link)`
  align-self: flex-start;
  border-radius: var(--radius, 2px);
  color: var(--v2-text);
  font-size: 14px;
  font-weight: 500;
  text-decoration: none;
  ${focusRing}

  &:hover {
    text-decoration: underline;
    text-underline-offset: 0.18em;
  }
`;

const SliderRow = styled.div`
  grid-area: slider;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 0 32px 28px;
  background: var(--v2-surface);

  @media (max-width: 860px) {
    padding: 20px 16px 24px;
  }
`;

/* A 2px rule with the passed part in full ink and a square accent thumb; the
   native input rides on top of it so arrows, Home and End keep working. */
const THUMB = 32;

const Track = styled.div`
  position: relative;
  height: ${THUMB}px;

  @media (max-width: 860px) {
    height: 40px;
  }
`;

const TrackRule = styled.div`
  position: absolute;
  top: ${THUMB / 2 - 1}px;
  right: ${THUMB / 2}px;
  left: ${THUMB / 2}px;
  height: 2px;
  background: rgba(255, 255, 255, 0.24);

  @media (max-width: 860px) {
    top: 19px;
    right: 20px;
    left: 20px;
  }
`;

const TrackFill = styled.div<{ $progress: number }>`
  position: absolute;
  top: ${THUMB / 2 - 1}px;
  left: ${THUMB / 2}px;
  width: calc((100% - ${THUMB}px) * ${({ $progress }) => $progress});
  height: 2px;
  background: var(--v2-text);
  transition: width 200ms var(--ease-out, ease-out);

  @media (max-width: 860px) {
    top: 19px;
    left: 20px;
    width: calc((100% - 40px) * ${({ $progress }) => $progress});
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const Range = styled.input`
  position: absolute;
  inset: 0;
  width: 100%;
  height: ${THUMB}px;
  margin: 0;
  appearance: none;
  background: transparent;
  cursor: pointer;

  &::-webkit-slider-runnable-track {
    height: 2px;
    background: transparent;
  }

  &::-webkit-slider-thumb {
    width: ${THUMB}px;
    height: ${THUMB}px;
    margin-top: ${-(THUMB / 2) + 1}px;
    appearance: none;
    border: 0;
    border-radius: var(--radius, 2px);
    background: var(--v2-accent);
  }

  @media (max-width: 860px) {
    height: 40px;

    &::-webkit-slider-thumb {
      width: 40px;
      height: 40px;
      margin-top: -19px;
    }

    &::-moz-range-thumb {
      width: 40px;
      height: 40px;
    }
  }

  &::-moz-range-track {
    height: 2px;
    background: transparent;
  }

  &::-moz-range-thumb {
    width: ${THUMB}px;
    height: ${THUMB}px;
    border: 0;
    border-radius: var(--radius, 2px);
    background: var(--v2-accent);
  }

  &:focus-visible {
    outline: none;
  }

  &:focus-visible::-webkit-slider-thumb {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }

  &:focus-visible::-moz-range-thumb {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }
`;

/* Each name is centred on its stop; the two end names are held inside the
   row's own measure so nothing leaves the margin. */
const Stops = styled.div`
  position: relative;
  height: 24px;
  margin-top: 2px;

  @media (max-width: 860px) {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 2px 12px;
    height: auto;
    margin-top: 6px;
  }
`;

/* Tappable stops: each name is a button centred on its stop, so the labels
   can also be used without the slider (and they never read as a kicker). */
const StopButton = styled.button<{ $at: number; $active: boolean }>`
  position: absolute;
  top: 0;
  left: calc(${THUMB / 2}px + (100% - ${THUMB}px) * ${({ $at }) => $at});
  min-height: 24px;
  padding: 0 4px;
  border: 0;
  border-radius: var(--radius, 2px);
  background: none;
  color: ${({ $active }) => ($active ? "var(--v2-text)" : "var(--v2-text-muted)")};
  font-size: 15px;
  font-weight: ${({ $active }) => ($active ? 600 : 400)};
  line-height: 24px;
  white-space: nowrap;
  transform: translateX(-50%);
  cursor: pointer;
  ${focusRing}

  &:hover {
    color: var(--v2-text);
  }

  &:first-child {
    transform: none;
  }

  &:last-child {
    transform: translateX(-100%);
  }

  @media (max-width: 860px) {
    position: static;
    display: flex;
    align-items: center;
    min-height: 36px;
    transform: none;
    line-height: 1.4;

    &:first-child,
    &:last-child {
      transform: none;
    }
  }
`;
