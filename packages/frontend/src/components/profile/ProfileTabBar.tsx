"use client";

import styled from "styled-components";
import { DitherTabs } from "dither-kit/DitherTabs";

export type ProfileTab = "activity" | "models";

export interface ProfileTabBarProps {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  /** Panels (`DitherTabPanel`) inherit the tab ids for aria linkage. */
  children?: React.ReactNode;
  className?: string;
}

const TABS = [
  { value: "activity", label: "Usage" },
  { value: "models", label: "Models" },
] as const;

/**
 * The profile's underline tabs. The kit's `DitherTabs` carries the dithered
 * accent marker; this shell puts it on the full-width rule the style asks for
 * and keeps the labels at app-screen size.
 */
const TabsShell = styled.div`
  min-width: 0;

  && [role="tablist"] {
    border-bottom: 1px solid var(--service-border);
    gap: 1.75rem;
  }

  && [role="tab"] {
    padding-top: 0.5rem;
    padding-bottom: 0.625rem;
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--service-text-muted);
  }

  && [role="tab"][aria-selected="true"] {
    color: var(--service-text);
  }

  && [role="tab"]:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

export function ProfileTabBar({
  activeTab,
  onTabChange,
  children,
  className,
}: ProfileTabBarProps) {
  return (
    <TabsShell className={className}>
      <DitherTabs
        tabs={TABS.map((tab) => ({ value: tab.value, label: tab.label }))}
        value={activeTab}
        onChange={(next) => onTabChange(next as ProfileTab)}
        variant="underline"
        color="#2f8fff"
      >
        {children}
      </DitherTabs>
    </TabsShell>
  );
}
