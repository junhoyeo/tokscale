"use client";

import { useEffect, useRef, useState, type FocusEvent, type KeyboardEvent, type ReactNode } from "react";
import styled, { css, keyframes } from "styled-components";

export interface TuiView {
  id: string;
  label: string;
  caption: string;
  src: string;
}

export const TUI_VIEWS: TuiView[] = [
  {
    id: "overview",
    label: "Overview",
    caption: "Tokens per day by model, and every model ranked by estimated cost.",
    src: "/assets/v2/tui/tui-overview.webp",
  },
  {
    id: "models",
    label: "Models",
    caption: "Each model with its provider, agent, input, output, cache tokens, and estimated cost.",
    src: "/assets/v2/tui/tui-models.webp",
  },
  {
    id: "daily",
    label: "Daily",
    caption: "One row per day: messages, tokens by type, and estimated cost.",
    src: "/assets/v2/tui/tui-daily.webp",
  },
  {
    id: "hourly",
    label: "Hourly",
    caption: "Usage hour by hour, with the agent and model behind each hour.",
    src: "/assets/v2/tui/tui-hourly.webp",
  },
  {
    id: "sessions",
    label: "Sessions",
    caption: "One session in detail: agent, workspace, models, duration, tokens, and cost.",
    src: "/assets/v2/tui/tui-sessions.webp",
  },
  {
    id: "stats",
    label: "Stats",
    caption: "A year of activity as a contribution graph, with streaks and your most used model.",
    src: "/assets/v2/tui/tui-stats.webp",
  },
];

/** How long each view stays up during the tour. */
export const TUI_TOUR_MS = 6000;
/* Below this width the views list becomes a row of tabs above the screenshot. */
const SIDE_BY_SIDE = "1080px";

interface TuiShowcaseProps {
  views?: TuiView[];
  /** Section heading and lead, shown beside the screenshot on wide screens. */
  intro?: ReactNode;
  tourMs?: number;
}

/**
 * The terminal UI as a short guided tour: the views advance on their own while the section is
 * on screen, and pause on hover, on focus, with the Pause button, or once someone picks a view.
 * No autoplay under prefers-reduced-motion.
 */
export function TuiShowcase({ views = TUI_VIEWS, intro, tourMs = TUI_TOUR_MS }: TuiShowcaseProps) {
  const [active, setActive] = useState(0);
  const [playing, setPlaying] = useState(true);
  const [reduced, setReduced] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [inView, setInView] = useState(false);
  const [mounted, setMounted] = useState<ReadonlySet<number>>(() => new Set([0, 1 % views.length]));
  const root = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  const tabs = useRef<Array<HTMLButtonElement | null>>([]);
  const view = views[active];

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    const node = root.current;
    if (!node) return;
    const observer = new IntersectionObserver(([entry]) => setInView(entry.isIntersecting), { threshold: 0.35 });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const touring = playing && !reduced;
  const running = touring && inView && !hovered && !focused;

  // Show a view and load the one after it, so each change can cross-fade.
  const show = (index: number) => {
    setActive(index);
    setMounted((current) => {
      const next = (index + 1) % views.length;
      return current.has(index) && current.has(next) ? current : new Set([...current, index, next]);
    });
  };

  const pick = (index: number) => {
    show(index);
    setPlaying(false);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const last = views.length - 1;
    let next: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") next = active === last ? 0 : active + 1;
    if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = active === 0 ? last : active - 1;
    if (event.key === "Home") next = 0;
    if (event.key === "End") next = last;
    if (next === null) return;
    event.preventDefault();
    pick(next);
    tabs.current[next]?.focus();
  };

  const onBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false);
  };

  return (
    <Layout
      ref={root}
      // The rotation control must be able to start the tour while it retains focus or hover.
      // Reading/interacting with the tabs and panel still pauses automatic rotation.
      onMouseOver={(event) => setHovered(!toggle.current?.contains(event.target as Node))}
      onMouseLeave={() => setHovered(false)}
      onFocus={(event) => setFocused(!toggle.current?.contains(event.target as Node))}
      onBlur={onBlur}
    >
      <Side>
        {intro}
        <TabList role="tablist" aria-label="Terminal UI views" onKeyDown={onKeyDown}>
          {views.map((item, index) => (
            <Tab
              key={item.id}
              ref={(node) => {
                tabs.current[index] = node;
              }}
              type="button"
              role="tab"
              id={`tui-tab-${item.id}`}
              aria-label={item.label}
              aria-selected={index === active}
              aria-controls={`tui-panel-${item.id}`}
              tabIndex={index === active ? 0 : -1}
              $active={index === active}
              onClick={() => pick(index)}
            >
              <TabLabel>{item.label}</TabLabel>
              {index === active ? (
                touring ? (
                  <Progress
                    $running={running}
                    style={{ animationDuration: `${tourMs}ms` }}
                    onAnimationEnd={() => show((active + 1) % views.length)}
                  />
                ) : (
                  <Indicator />
                )
              ) : null}
            </Tab>
          ))}
        </TabList>
        {reduced ? null : (
          <TourToggle ref={toggle} type="button" aria-pressed={!playing} onClick={() => setPlaying((value) => !value)}>
            {playing ? <PauseIcon /> : <PlayIcon />}
            {playing ? "Pause tour" : "Play tour"}
          </TourToggle>
        )}
      </Side>

      <Panel role="tabpanel" id={`tui-panel-${view.id}`} aria-labelledby={`tui-tab-${view.id}`}>
        <Caption>{view.caption}</Caption>
        <Frame>
          {views.map((item, index) =>
            mounted.has(index) ? (
              // eslint-disable-next-line @next/next/no-img-element -- static screenshots, sized by CSS
              <img
                key={item.id}
                src={item.src}
                alt={index === active ? `Tokscale terminal UI, ${item.label} view. ${item.caption}` : ""}
                aria-hidden={index === active ? undefined : true}
                data-active={index === active ? "true" : undefined}
                width={1800}
                height={1270}
                loading="lazy"
                decoding="async"
              />
            ) : null,
          )}
        </Frame>
      </Panel>
    </Layout>
  );
}

function PauseIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
      <rect x="2" y="1.5" width="2.6" height="9" rx="0.6" fill="currentColor" />
      <rect x="7.4" y="1.5" width="2.6" height="9" rx="0.6" fill="currentColor" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
      <path d="M3 1.6v8.8a.5.5 0 0 0 .76.43l7-4.4a.5.5 0 0 0 0-.86l-7-4.4A.5.5 0 0 0 3 1.6Z" fill="currentColor" />
    </svg>
  );
}

const fill = keyframes`
  from {
    transform: scaleX(0);
  }
  to {
    transform: scaleX(1);
  }
`;

const Layout = styled.div`
  display: grid;
  grid-template-areas: "panel side";
  grid-template-columns: minmax(0, 7fr) minmax(0, 4fr);
  gap: 48px;
  align-items: center;

  @media (max-width: ${SIDE_BY_SIDE}) {
    grid-template-areas:
      "side"
      "panel";
    grid-template-columns: minmax(0, 1fr);
    gap: 20px;
  }
`;

const Side = styled.div`
  display: flex;
  flex-direction: column;
  gap: 28px;
  grid-area: side;
  min-width: 0;

  @media (max-width: ${SIDE_BY_SIDE}) {
    gap: 20px;
  }
`;

const TabList = styled.div`
  display: flex;
  flex-direction: column;
  gap: 2px;

  @media (max-width: ${SIDE_BY_SIDE}) {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 6px;
    max-width: 100%;
    padding-bottom: 2px;
  }
`;

const Tab = styled.button<{ $active: boolean }>`
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  width: fit-content;
  padding: 10px 12px 12px;
  border: 0;
  border-radius: var(--radius, 2px);
  background: ${({ $active }) => ($active ? "var(--v2-surface)" : "transparent")};
  color: ${({ $active }) => ($active ? "var(--v2-text)" : "var(--v2-text-muted)")};
  text-align: left;
  cursor: pointer;

  &:hover {
    color: var(--v2-text);
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }

  @media (max-width: ${SIDE_BY_SIDE}) {
    flex-shrink: 0;
    min-height: 40px;
    padding: 8px 12px 10px;
    justify-content: center;

    &:focus-visible {
      outline-offset: -2px;
    }
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }
`;

const TabLabel = styled.span`
  font-size: 15px;
  font-weight: 500;

  @media (max-width: ${SIDE_BY_SIDE}) {
    font-size: 14px;
  }
`;

const barPosition = css`
  position: absolute;
  right: 0;
  bottom: 0;
  left: 0;
  height: 2px;
  background: var(--v2-accent);

  @media (max-width: ${SIDE_BY_SIDE}) {
    right: 8px;
    left: 8px;
  }
`;

const Progress = styled.span<{ $running: boolean }>`
  ${barPosition}
  transform-origin: left center;
  animation-name: ${fill};
  animation-timing-function: linear;
  animation-fill-mode: forwards;
  animation-play-state: ${({ $running }) => ($running ? "running" : "paused")};
`;

const Indicator = styled.span`
  ${barPosition}
`;

const TourToggle = styled.button`
  display: inline-flex;
  align-items: center;
  align-self: flex-start;
  gap: 8px;
  min-height: 36px;
  padding: 0 12px;
  border: 0;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);
  color: var(--v2-text-muted);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;

  &:hover {
    color: var(--v2-text);
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }
`;

const Panel = styled.div`
  display: flex;
  flex-direction: column;
  gap: 16px;
  grid-area: panel;
  min-width: 0;
`;

/* The selected view's one-line caption, above its screenshot at every width. */
const Caption = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 16px;
  line-height: 1.5;
`;

/* The screenshots carry their own window chrome; stacked so a change can cross-fade. */
const Frame = styled.div`
  position: relative;
  aspect-ratio: 1800 / 1270;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);

  img {
    position: absolute;
    inset: 0;
    display: block;
    width: 100%;
    height: 100%;
    opacity: 0;
  }

  img[data-active] {
    opacity: 1;
  }

  @media (prefers-reduced-motion: no-preference) {
    img {
      transition: opacity 0.45s ease;
    }
  }
`;
