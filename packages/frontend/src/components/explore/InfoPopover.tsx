"use client";

/**
 * Small non-modal popover for source details. Built on the native Popover
 * API: the panel renders in the top layer (never clipped by a scrolling
 * table), closes on Escape and outside click, and follows its trigger in the
 * DOM so Tab moves from the button into the panel's links.
 */
import { useCallback, useEffect, useId, useRef, type ReactNode } from "react";
import styled from "styled-components";
import { InstantTooltip } from "@/components/ui/InstantTooltip";

/* popover and popoverTarget are valid DOM props in React 19; forward them without styled-components' dev warning. */
const forwardDomProps = { shouldForwardProp: (prop: string) => !prop.startsWith("$") };

const Trigger = styled.button.withConfig(forwardDomProps)`
  display: inline-grid;
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  place-items: center;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-weight: 600;
  line-height: 1;

  &:hover {
    border-color: var(--service-border-strong);
    color: var(--service-text);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }

  @media (pointer: coarse) {
    width: 32px;
    height: 32px;
  }
`;

const Panel = styled.div.withConfig(forwardDomProps)`
  position: fixed;
  inset: auto;
  width: min(320px, calc(100vw - 24px));
  max-height: min(420px, calc(100vh - 24px));
  margin: 0;
  overflow: auto;
  padding: 12px 14px;
  border: 1px solid var(--service-border-strong);
  border-radius: 10px;
  background: var(--service-surface);
  color: var(--service-text);
  font-size: 0.8125rem;
  line-height: 1.5;
  text-align: left;
  white-space: normal;

  &:not(:popover-open) {
    display: none;
  }

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

function InfoGlyph() {
  return (
    <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <path d="M8 7.2v4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="8" cy="4.9" r="0.9" fill="currentColor" />
    </svg>
  );
}

/** Place the panel under (or above) the trigger, clamped to the viewport. */
function place(trigger: HTMLElement, panel: HTMLElement) {
  const rect = trigger.getBoundingClientRect();
  const width = panel.offsetWidth;
  const height = panel.offsetHeight;
  const gap = 6;
  const left = Math.min(Math.max(12, rect.left + rect.width / 2 - width / 2), window.innerWidth - width - 12);
  const below = rect.bottom + gap;
  const top = below + height <= window.innerHeight - 12 ? below : Math.max(12, rect.top - gap - height);
  panel.style.left = `${Math.round(left)}px`;
  panel.style.top = `${Math.round(top)}px`;
}

export function InfoPopover({ label, children }: { label: string; children: ReactNode }) {
  const id = useId().replace(/:/g, "");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const reposition = useCallback(() => {
    const trigger = triggerRef.current;
    const panel = panelRef.current;
    if (trigger && panel && panel.matches(":popover-open")) place(trigger, panel);
  }, []);

  useEffect(() => {
    const panel = panelRef.current;
    if (!panel) return;
    const onToggle = (event: Event) => {
      const open = (event as ToggleEvent).newState === "open";
      if (open) {
        reposition();
        window.addEventListener("scroll", reposition, true);
        window.addEventListener("resize", reposition);
      } else {
        window.removeEventListener("scroll", reposition, true);
        window.removeEventListener("resize", reposition);
      }
    };
    panel.addEventListener("toggle", onToggle);
    return () => {
      panel.removeEventListener("toggle", onToggle);
      window.removeEventListener("scroll", reposition, true);
      window.removeEventListener("resize", reposition);
    };
  }, [reposition]);

  return (
    <>
      <InstantTooltip as={Trigger} ref={triggerRef} type="button" popoverTarget={id} aria-label={label} content={label} describe={false}>
        <InfoGlyph />
      </InstantTooltip>
      <Panel ref={panelRef} id={id} popover="auto" role="dialog" aria-label={label}>
        {children}
      </Panel>
    </>
  );
}

export const PopoverTitle = styled.p`
  margin: 0 0 6px;
  color: var(--service-text);
  font-weight: 600;
`;

export const PopoverFacts = styled.dl`
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 2px 12px;
  margin: 0 0 8px;
  font-variant-numeric: tabular-nums;

  dt {
    color: var(--service-text-muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
`;

export const PopoverNote = styled.p`
  margin: 6px 0 0;
  color: var(--service-text-muted);
`;

export const PopoverLink = styled.a`
  color: var(--service-accent-hover);
  text-decoration: none;

  &:hover {
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;
