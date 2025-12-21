import { Show } from "solid-js";
import { exec } from "node:child_process";
import type { TabType } from "../types/index.js";
import { isNarrow, isVeryNarrow } from "../utils/responsive.js";

const REPO_URL = "https://github.com/junhoyeo/tokscale";

function openUrl(url: string) {
  const cmd = process.platform === "darwin" ? "open" : process.platform === "win32" ? "start" : "xdg-open";
  exec(`${cmd} ${url}`);
}

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
        <Tab name={getTabName("Agent", "Agt")} tabId="agent" active={props.activeTab === "agent"} onClick={props.onTabClick} />
      </box>
      <Show when={!isNarrowTerminal()}>
        <box flexDirection="row" onMouseDown={() => openUrl(REPO_URL)}>
          <text fg="cyan" bold>tokscale</text>
          <text fg="#666666">{" | GitHub"}</text>
        </box>
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
