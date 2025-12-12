import { render } from "@opentui/solid";
import { App } from "./App.js";

export async function launchTUI() {
  await render(() => <App />, {
    exitOnCtrlC: false,
    useAlternateScreen: true,
    useMouse: false,
    targetFps: 60,
    useKittyKeyboard: {},
  } as any);
}
