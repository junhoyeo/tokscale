"use client";

import { useRef } from "react";
import type { KeyboardEvent } from "react";
import styled from "styled-components";

export type ProfileTab = "activity" | "models";

export interface ProfileTabBarProps {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  className?: string;
}

const tabs: ReadonlyArray<{ id: ProfileTab; label: string }> = [
  { id: "activity", label: "Usage" },
  { id: "models", label: "Models" },
];

const TabList = styled.div`
  display: grid;
  width: 100%;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.25rem;
  border: 1px solid var(--service-border);
  border-radius: 8px;
  padding: 0.25rem;
  background: var(--service-surface);
`;

const TabButton = styled.button<{ $active: boolean }>`
  display: inline-flex;
  min-width: 0;
  min-height: 32px;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  border: 1px solid
    ${(props) =>
      props.$active ? "var(--service-border-strong)" : "transparent"};
  border-radius: 8px;
  padding: 0.35rem 0.5rem;
  background: ${(props) =>
    props.$active ? "var(--service-accent-soft)" : "transparent"};
  color: ${(props) =>
    props.$active ? "var(--service-text)" : "var(--service-text-muted)"};
  font: inherit;
  font-size: 0.8125rem;
  font-weight: ${(props) => (props.$active ? 600 : 500)};
  line-height: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
  transition:
    border-color 140ms ease,
    background-color 140ms ease,
    color 140ms ease;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }

  @media (hover: hover) {
    &:hover {
      background: var(--service-surface-muted);
      color: var(--service-text);
    }
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

export function ProfileTabBar({
  activeTab,
  onTabChange,
  className,
}: ProfileTabBarProps) {
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const selectAndFocus = (index: number) => {
    onTabChange(tabs[index].id);
    tabRefs.current[index]?.focus();
  };

  const handleKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    currentIndex: number,
  ) => {
    let nextIndex: number | null = null;

    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
        nextIndex = (currentIndex + 1) % tabs.length;
        break;
      case "ArrowLeft":
      case "ArrowUp":
        nextIndex = (currentIndex - 1 + tabs.length) % tabs.length;
        break;
      case "Home":
        nextIndex = 0;
        break;
      case "End":
        nextIndex = tabs.length - 1;
        break;
      default:
        return;
    }

    event.preventDefault();
    selectAndFocus(nextIndex);
  };

  return (
    <TabList
      className={className}
      role="tablist"
      aria-label="Profile sections"
      aria-orientation="horizontal"
    >
      {tabs.map((tab, index) => {
        const isActive = tab.id === activeTab;

        return (
          <TabButton
            key={tab.id}
            ref={(node) => {
              tabRefs.current[index] = node;
            }}
            id={`tab-${tab.id}`}
            type="button"
            role="tab"
            aria-selected={isActive}
            aria-controls={isActive ? `tabpanel-${tab.id}` : undefined}
            tabIndex={isActive ? 0 : -1}
            $active={isActive}
            onClick={() => onTabChange(tab.id)}
            onKeyDown={(event) => handleKeyDown(event, index)}
          >
            {tab.label}
          </TabButton>
        );
      })}
    </TabList>
  );
}
