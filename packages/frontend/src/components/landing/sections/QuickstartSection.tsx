"use client";

import Image from "next/image";
import styled, { css, keyframes } from "styled-components";
import { useCopy, useSquircleClip } from "../hooks";
import { SquircleBorder } from "../components";

export function QuickstartSection() {
  const tui = useCopy("bunx tokscale@latest");
  const submit = useCopy("bunx tokscale@latest submit");
  const {
    setElementRef: setCardsRowRef,
    clipPath: cardsRowClipPath,
    svgDef: cardsRowSvgDef,
    borderDef: cardsRowBorderDef,
  } = useSquircleClip<HTMLDivElement>(32, 0.6, true, 1);

  return (
    <>
      {/* SVG clip-path def for cards */}
      {cardsRowSvgDef && (
        <svg
          width="0"
          height="0"
          style={{ position: "absolute", overflow: "hidden" }}
          aria-hidden="true"
          role="presentation"
        >
          <defs>
            <clipPath id={cardsRowSvgDef.id}>
              <path
                d={cardsRowSvgDef.path}
                transform={`translate(0, -${cardsRowSvgDef.cornerRadius})`}
              />
            </clipPath>
          </defs>
        </svg>
      )}

      {/* Separator Bar */}
      <SeparatorBar />

      {/* Quickstart Label */}
      <QuickstartLabel>
        <QuickstartText>Quickstart</QuickstartText>
      </QuickstartLabel>

      {/* Quickstart Cards */}
      <QuickstartCardsWrapper>
        <QuickstartCardsRow
          ref={setCardsRowRef}
          style={{
            clipPath: cardsRowClipPath || undefined,
          }}
        >
          <SquircleBorder def={cardsRowBorderDef} />
          {/* Left Card */}
          <QuickstartCard $position="left">
            <CardPatternOverlay $position="left" />
            <CardScreenshot>
              <Image
                src="/assets/landing/screenshot-tui-4d3240.png"
                alt="TUI Screenshot"
                width={171}
                height={168}
                style={{ width: 171.25, height: 168, objectFit: "cover", borderRadius: 8 }}
              />
            </CardScreenshot>
            <CardContent>
              <CardTitle>
                View your
                <br />
                Usage Stats
              </CardTitle>
              <CommandBox>
                <CommandInputArea>
                  <CommandText>bunx tokscale@latest</CommandText>
                  <GradientAccent />
                </CommandInputArea>
                <CopyBtn onClick={tui.copy}>
                  <CopyBtnText>{tui.copied ? "Copied!" : "Copy"}</CopyBtnText>
                </CopyBtn>
              </CommandBox>
            </CardContent>
          </QuickstartCard>

          {/* Right Card */}
          <QuickstartCard $position="right">
            <CardPatternOverlay $position="right" />
            <CardScreenshot>
              <Image
                src="/assets/landing/screenshot-leaderboard-75a76a.png"
                alt="Leaderboard Screenshot"
                width={152}
                height={180}
                style={{ width: 152.02, height: 180, objectFit: "cover", borderRadius: 8 }}
              />
            </CardScreenshot>
            <CardContent>
              <CardTitle>
                Submit DATA
                <br />
                to the Global Leaderboard
              </CardTitle>
              <CommandBox>
                <CommandInputArea>
                  <CommandText>bunx tokscale@latest submit</CommandText>
                  <GradientAccent $delay />
                </CommandInputArea>
                <CopyBtn onClick={submit.copy}>
                  <CopyBtnText>{submit.copied ? "Copied!" : "Copy"}</CopyBtnText>
                </CopyBtn>
              </CommandBox>
            </CardContent>
          </QuickstartCard>
        </QuickstartCardsRow>
      </QuickstartCardsWrapper>
    </>
  );
}

/* ── Separator Bar ── */
const SeparatorBar = styled.div`
  width: 100%;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  background-image: url("/assets/landing/separator-pattern-slash@gray.svg");
  background-size: 24px 24px;
  background-repeat: repeat;
  border-left: 1px solid #10233E;
  border-right: 1px solid #10233E;
  border-bottom: 1px solid #10233E;
`;

/* ── Quickstart Label ── */
const QuickstartLabel = styled.div`
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px 32px;
  background: #0073ff;
  border-left: 1px solid #10233e;
  border-right: 1px solid #10233e;
`;

const QuickstartText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 20px;
  line-height: 1em;
  text-transform: uppercase;
  text-align: center;
  color: #ffffff;
`;

/* ── Quickstart Cards ── */
const QuickstartCardsWrapper = styled.div`
  width: 100%;
  padding-bottom: 64px;
`;

const QuickstartCardsRow = styled.div`
  position: relative;
  width: 100%;
  display: flex;
  flex-direction: row;
  background: #01070f;
  overflow: hidden;

  @media (max-width: 900px) {
    flex-direction: column;
  }
`;

const QuickstartCard = styled.div<{ $position: "left" | "right" }>`
  position: relative;
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  padding: ${({ $position }) =>
    $position === "left" ? "32px" : "21px 32px 32px"};
  min-height: ${({ $position }) =>
    $position === "left" ? "320px" : "320px"};
  ${({ $position }) =>
    $position === "left" && css`
      border-right: 1px solid #10233e;
    `}

  @media (max-width: 1000px) {
    padding-left: 20px;
    padding-right: 20px;
    padding-bottom: 20px;
  }

  @media (max-width: 900px) {
    ${({ $position }) =>
      $position === "left" && css`
      border-right: none;
      border-bottom: 1px solid #10233e;
    `}

    padding-left: 32px;
    padding-right: 32px;
    padding-bottom: 32px;
  }
`;

const CardPatternOverlay = styled.div<{ $position: "left" | "right" }>`
  position: absolute;
  left: 0;
  top: ${({ $position }) => ($position === "left" ? "120px" : "96px")};
  width: 100%;
  height: 24px;
  background-image: url("/assets/landing/separator-pattern-slash@gray.svg");
  background-size: 24px 24px;
  background-repeat: repeat;
  pointer-events: none;
`;

const CardScreenshot = styled.div`
  position: absolute;
  top: 32px;
  right: 32px;
`;

const CardContent = styled.div`
  display: flex;
  flex-direction: column;
  align-self: stretch;
  gap: 20px;
  margin-top: auto;
`;

const CardTitle = styled.h3`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 20px;
  line-height: 1em;
  text-transform: uppercase;
  color: #ffffff;
  z-index: 1;
`;

const CommandBox = styled.div`
  display: flex;
  flex-direction: row;
  align-items: center;
  align-self: stretch;
  gap: 6px;
  padding: 8px;
  background: #010a15;
  border: 1px solid #10233e;
  border-radius: 12px;
`;

const CommandInputArea = styled.div`
  position: relative;
  display: flex;
  flex-direction: row;
  align-items: center;
  flex: 1;
  gap: 10px;
  padding: 0 12px;
  background: #111b2c;
  border-radius: 8px;
  height: 36px;
  overflow: hidden;
`;

const CommandText = styled.span`
  font-family: "Inconsolata", monospace !important;
  font-weight: 700;
  font-size: 16px;
  line-height: 0.94em;
  letter-spacing: -0.05em;
  text-align: center;
  color: #9ad7ed;
  white-space: nowrap;

  @media (max-width: 480px) {
    font-size: 14px;
  }
`;

const cursorSweep = keyframes`
  0%, 100% {
    left: 0;
  }
  50% {
    left: calc(100% - 25px);
  }
`;
const GradientAccent = styled.div<{ $delay?: boolean }>`
  position: absolute;
  top: 0;
  left: 0;
  width: 25px;
  height: 36px;
  background: linear-gradient(
    270deg,
    rgba(26, 27, 28, 0) 0%,
    rgba(1, 127, 255, 0.14) 50%,
    rgba(26, 27, 28, 0) 100%
  );
  animation: ${cursorSweep} 6s ease-in-out infinite;
  animation-delay: ${({ $delay }) => ($delay ? '-2s' : '0s')};
  pointer-events: none;
`;

const CopyBtn = styled.button`
  display: flex;
  justify-content: center;
  align-items: center;
  width: 86px;
  height: 36px;
  background: #0073ff;
  border-radius: 8px;
  border: none;
  cursor: pointer;
  flex-shrink: 0;
  transition: opacity 0.15s;

  &:hover {
    opacity: 0.9;
  }
  &:active {
    transform: scale(0.97);
  }
`;

const CopyBtnText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 18px;
  line-height: 0.94em;
  letter-spacing: -0.05em;
  text-align: center;
  color: #ffffff;
`;
