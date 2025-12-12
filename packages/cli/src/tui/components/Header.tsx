import { Show } from "solid-js";
import type { TabType } from "../App.js";

interface HeaderProps {
  activeTab: TabType;
}

export function Header(props: HeaderProps) {
  return (
    <box flexDirection="row" paddingX={1} paddingY={0} justifyContent="space-between">
      <box flexDirection="row" gap={2}>
        <Tab name="Overview" active={props.activeTab === "overview"} />
        <Tab name="Models" active={props.activeTab === "model"} />
        <Tab name="Daily" active={props.activeTab === "daily"} />
        <Tab name="Stats" active={props.activeTab === "stats"} />
      </box>
      <text fg="cyan" bold>Token Usage Tracker</text>
    </box>
  );
}

function Tab(props: { name: string; active: boolean }) {
  return (
    <Show when={props.active} fallback={<text dim>{props.name}</text>}>
      <box>
        <text backgroundColor="cyan" fg="black" bold>{` ${props.name} `}</text>
      </box>
    </Show>
  );
}
