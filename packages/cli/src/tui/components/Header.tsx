import { Show } from "solid-js";
import type { TabType } from "../types/index.js";
import { isNarrow, isVeryNarrow } from "../utils/responsive.js";

interface HeaderProps {
  activeTab: TabType;
  onTabClick?: (tab: TabType) => void;
  width?: number;
}

export function Header(props: HeaderProps) {
  const isNarrowTerminal = () => isNarrow(props.width);
  const isVeryNarrowTerminal = () => isVeryNarrow(props.width);

  const getTabName = (fullName: string, shortName: string) => 
    isVeryNarrowTerminal() ? shortName : fullName;

  return (
    <box flexDirection="row" paddingX={1} paddingY={0} justifyContent="space-between">
      <box flexDirection="row" gap={isVeryNarrowTerminal() ? 1 : 2}>
        <Tab name={getTabName("Overview", "Ovw")} tabId="overview" active={props.activeTab === "overview"} onClick={props.onTabClick} />
        <Tab name={getTabName("Models", "Mod")} tabId="model" active={props.activeTab === "model"} onClick={props.onTabClick} />
        <Tab name={getTabName("Daily", "Day")} tabId="daily" active={props.activeTab === "daily"} onClick={props.onTabClick} />
        <Tab name={getTabName("Stats", "Sta")} tabId="stats" active={props.activeTab === "stats"} onClick={props.onTabClick} />
      </box>
      <Show when={!isNarrowTerminal()}>
        <text fg="cyan" bold>Token Usage Tracker</text>
      </Show>
    </box>
  );
}

interface TabProps {
  name: string;
  tabId: TabType;
  active: boolean;
  onClick?: (tab: TabType) => void;
}

function Tab(props: TabProps) {
  const handleClick = () => props.onClick?.(props.tabId);

  return (
    <Show
      when={props.active}
      fallback={
        <box onMouseDown={handleClick}>
          <text dim>{props.name}</text>
        </box>
      }
    >
      <box onMouseDown={handleClick}>
        <text bg="cyan" fg="black" bold>{` ${props.name} `}</text>
      </box>
    </Show>
  );
}
