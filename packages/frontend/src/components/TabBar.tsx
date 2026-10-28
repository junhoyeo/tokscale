"use client";

import styled from "styled-components";
import { DitherUnderline } from "@/components/ui/DitherUnderline";

export interface TabItem<T extends string> {
  id: T;
  label: string;
}

export interface TabBarProps<T extends string> {
  tabs: TabItem<T>[];
  activeTab: T;
  onTabChange: (tab: T) => void;
}

/**
 * Plain tabs: the only mark of the selected tab is the dithered accent line
 * under its label, as wide as the label. Arrow keys, Home and End move the
 * selection, and the selected tab is the one tab stop.
 */
export function TabBar<T extends string>({ tabs, activeTab, onTabChange }: TabBarProps<T>) {
  const handleKeyDown = (e: React.KeyboardEvent, currentIndex: number) => {
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      onTabChange(tabs[(currentIndex + 1) % tabs.length].id);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      onTabChange(tabs[(currentIndex - 1 + tabs.length) % tabs.length].id);
    } else if (e.key === "Home") {
      e.preventDefault();
      onTabChange(tabs[0].id);
    } else if (e.key === "End") {
      e.preventDefault();
      onTabChange(tabs[tabs.length - 1].id);
    }
  };

  return (
    <TabList role="tablist" aria-label="Content tabs">
      {tabs.map((tab, index) => {
        const isActive = activeTab === tab.id;
        return (
          <TabButton
            key={tab.id}
            role="tab"
            aria-selected={isActive}
            tabIndex={isActive ? 0 : -1}
            onClick={() => onTabChange(tab.id)}
            onKeyDown={(e) => handleKeyDown(e, index)}
          >
            <TabLabel $active={isActive}>
              {tab.label}
              {isActive ? <DitherUnderline /> : null}
            </TabLabel>
          </TabButton>
        );
      })}
    </TabList>
  );
}

const TabList = styled.div`
  display: inline-flex;
  flex-direction: row;
  align-items: flex-end;
  gap: 24px;
`;

const TabButton = styled.button`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 8px 0 10px;
  background: transparent;

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }
`;

const TabLabel = styled.span<{ $active?: boolean }>`
  position: relative;
  display: inline-flex;
  padding-bottom: 8px;
  color: ${({ $active }) => ($active ? "var(--v2-text)" : "var(--v2-text-muted)")};
  font-size: 15px;
  font-weight: ${({ $active }) => ($active ? 600 : 400)};
  line-height: 1.2;
  white-space: nowrap;

  &:hover {
    color: var(--v2-text);
  }

  /* Absolutely placed: the canvas never feeds its own width back into the tab. */
  > span {
    position: absolute;
    bottom: 0;
    left: 0;
    display: block;
    width: 100%;
    height: 2px;
    pointer-events: none;
  }

  canvas {
    display: block;
    image-rendering: pixelated;
  }
`;
