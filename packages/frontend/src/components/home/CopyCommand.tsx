"use client";

import { useEffect, useRef, useState } from "react";
import styled, { css, keyframes } from "styled-components";
import { CheckIcon, CopyIcon } from "@/components/ui/Icons";

interface CopyCommandProps {
  command: string;
  /** Accessible name for the copy button, e.g. "Copy install command". */
  label?: string;
  prompt?: boolean;
  /** Wrap long values (README snippets) instead of scrolling them. */
  wrap?: boolean;
  /** Type the command in once on first paint (CSS only; static under reduced motion). */
  typed?: boolean;
}

export function CopyCommand({ command, label = "Copy command", prompt = true, wrap = false, typed = false }: CopyCommandProps) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  return (
    <Box $wrap={wrap}>
      <Code $wrap={wrap}>
        {prompt ? <Prompt aria-hidden="true">$</Prompt> : null}
        {typed && !wrap ? (
          <TypedCode style={{ "--chars": command.length } as React.CSSProperties}>{command}</TypedCode>
        ) : (
          <code>{command}</code>
        )}
      </Code>
      <CopyButton type="button" onClick={() => void copy()} aria-label={copied ? "Copied" : label} $copied={copied}>
        {copied ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
        <span aria-hidden="true">{copied ? "Copied" : "Copy"}</span>
      </CopyButton>
      <Status role="status" aria-live="polite">
        {copied ? "Copied to clipboard" : ""}
      </Status>
    </Box>
  );
}

const Box = styled.div<{ $wrap: boolean }>`
  position: relative;
  display: flex;
  align-items: ${({ $wrap }) => ($wrap ? "flex-start" : "center")};
  gap: 8px;
  width: 100%;
  max-width: ${({ $wrap }) => ($wrap ? "none" : "420px")};
  min-height: 52px;
  padding: 4px 4px 4px 16px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface-raised);
`;

const Code = styled.span<{ $wrap: boolean }>`
  display: flex;
  flex: 1;
  gap: 10px;
  min-width: 0;
  overflow-x: ${({ $wrap }) => ($wrap ? "visible" : "auto")};
  padding: ${({ $wrap }) => ($wrap ? "10px 0" : "0")};
  white-space: ${({ $wrap }) => ($wrap ? "normal" : "nowrap")};

  code {
    color: var(--v2-text);
    font-size: ${({ $wrap }) => ($wrap ? "13px" : "15px")};
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
`;

const typing = keyframes`
  from {
    max-width: 0;
  }
  to {
    max-width: calc(var(--chars) * 1ch);
  }
`;

const caretBlink = keyframes`
  0%,
  49% {
    border-right-color: var(--v2-accent);
  }
  50%,
  100% {
    border-right-color: transparent;
  }
`;

/* Monospace, so `ch` steps reveal one character at a time. The caret blinks a few times, then goes. */
const TypedCode = styled.code`
  display: inline-block;
  overflow: hidden;
  max-width: calc(var(--chars) * 1ch);
  border-right: 2px solid transparent;
  vertical-align: bottom;
  white-space: nowrap;

  @media (prefers-reduced-motion: no-preference) {
    ${css`
      animation:
        ${typing} 0.9s steps(var(--chars), end) 0.25s both,
        ${caretBlink} 0.7s step-end 0s 4;
    `}
  }
`;

const Prompt = styled.span`
  color: var(--v2-text-subtle);
  font-size: 15px;
  user-select: none;
`;

/* The section's one control, filled, with its verb written on it. */
const CopyButton = styled.button<{ $copied: boolean }>`
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 40px;
  padding: 0 14px;
  border: 0;
  border-radius: var(--radius, 2px);
  background: ${({ $copied }) => ($copied ? "var(--v2-surface)" : "var(--v2-accent-strong)")};
  color: ${({ $copied }) => ($copied ? "var(--v2-text)" : "#fff")};
  font-size: 15px;
  font-weight: 600;
  cursor: pointer;
  transition: background-color 160ms ease;

  &:hover {
    background: ${({ $copied }) => ($copied ? "var(--v2-surface-raised)" : "var(--v2-accent)")};
  }

  &:active {
    transform: scale(0.97);
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;

    &:active {
      transform: none;
    }
  }
`;

const Status = styled.span`
  position: absolute;
  width: 1px;
  height: 1px; /* taste-skill-ignore: screen-reader-only announcement */
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
`;
