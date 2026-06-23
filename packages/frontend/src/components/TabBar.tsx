"use client";

import styled, { css } from "styled-components";

export interface TabItem<T extends string> {
  id: T;
  label: string;
}

export interface TabBarProps<T extends string> {
  tabs: TabItem<T>[];
  activeTab: T;
  onTabChange: (tab: T) => void;
}

const TabBarContainer = styled.div`
  display: inline-flex;
  flex-direction: row;
  align-items: center;
  border-radius: 25px;
  border: 1px solid;
  padding: 6px;
`;

const TabButton = styled.button<{ $active?: boolean }>`
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 25px;
  padding-left: 1.25rem;
  padding-right: 1.25rem;
  padding-top: 10px;
  padding-bottom: 10px;
  transition: color 150ms, background-color 150ms;

  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--background, #10121C), 0 0 0 4px #3b82f6;
  }

  ${(props) =>
    props.$active
      ? css`
          background-color: var(--color-bg-active);
        `
      : css`
          background-color: transparent;
        `}
`;

const TabLabel = styled.span<{ $active?: boolean }>`
  font-size: 1.125rem;
  font-weight: 600;
  line-height: 1;
  white-space: nowrap;

  ${(props) =>
    props.$active
      ? css`
          color: var(--color-fg-default);
        `
      : css`
          color: var(--color-fg-muted);
        `}
`;

export function TabBar<T extends string>({
  tabs,
  activeTab,
  onTabChange,
}: TabBarProps<T>) {
  const handleKeyDown = (e: React.KeyboardEvent, currentIndex: number) => {
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      const nextIndex = (currentIndex + 1) % tabs.length;
      onTabChange(tabs[nextIndex].id);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      const prevIndex = (currentIndex - 1 + tabs.length) % tabs.length;
      onTabChange(tabs[prevIndex].id);
    } else if (e.key === "Home") {
      e.preventDefault();
      onTabChange(tabs[0].id);
    } else if (e.key === "End") {
      e.preventDefault();
      onTabChange(tabs[tabs.length - 1].id);
    }
  };

  return (
    <TabBarContainer
      role="tablist"
      aria-label="Content tabs"
      style={{
        width: "fit-content",
        backgroundColor: "var(--color-bg-elevated)",
        borderColor: "var(--color-border-default)",
      }}
    >
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
            $active={isActive}
          >
            <TabLabel $active={isActive}>
              {tab.label}
            </TabLabel>
          </TabButton>
        );
      })}
    </TabBarContainer>
  );
}
