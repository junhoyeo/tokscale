"use client";

import Link from "next/link";
import styled from "styled-components";

// Top-of-page segmented control that swaps between the global user leaderboard
// and the group browser. Pure-link nav (no client state), so SSR + back/forward
// behave naturally and the URL is shareable.

export type LeaderboardView = "users" | "groups";

const Bar = styled.nav`
  margin: 24px 0 0;
  display: flex;
  align-items: center;
  gap: 16px;
`;

const Group = styled.div`
  display: inline-flex;
  padding: 4px;
  border: 1px solid var(--color-border-default);
  border-radius: 10px;
  background: var(--color-bg-subtle);
`;

const Item = styled(Link)<{ $active: boolean }>`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  padding: 0 14px;
  border-radius: 8px;
  font-size: 13px;
  font-weight: 600;
  text-decoration: none;
  color: ${({ $active }) => ($active ? "var(--color-fg-default)" : "var(--color-fg-muted)")};
  background: ${({ $active }) => ($active ? "var(--color-bg-default)" : "transparent")};
  transition: background 0.12s, color 0.12s;

  &:hover {
    color: var(--color-fg-default);
  }
`;

const Title = styled.h1`
  margin: 0;
  font-size: 30px;
  font-weight: 700;
  color: var(--color-fg-default);
`;

interface ViewSelectorProps {
  current: LeaderboardView;
}

export default function ViewSelector({ current }: ViewSelectorProps) {
  return (
    <Bar aria-label="Leaderboard view">
      <Title>{current === "groups" ? "Groups" : "Leaderboard"}</Title>
      <Group role="tablist">
        <Item href="/leaderboard?view=users" $active={current === "users"} role="tab" aria-selected={current === "users"}>
          Users
        </Item>
        <Item href="/leaderboard?view=groups" $active={current === "groups"} role="tab" aria-selected={current === "groups"}>
          Groups
        </Item>
      </Group>
    </Bar>
  );
}
