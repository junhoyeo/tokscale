import { render } from "@opentui/solid";
import { App } from "./App.js";
import type { TUIOptions } from "./types/index.js";
import { restoreTerminalState } from "./utils/cleanup.js";

export type { TUIOptions };

export async function launchTUI(options?: TUIOptions) {
  
  const cleanup = () => {
    restoreTerminalState();
  };

  process.on('uncaughtException', (error) => {
    cleanup();
    console.error('Uncaught exception:', error);
    process.exit(1);
  });
  
  process.on('unhandledRejection', (reason) => {
    cleanup();
    console.error('Unhandled rejection:', reason);
    process.exit(1);
  });

  process.on('SIGINT', () => {
    cleanup();
    process.exit(0);
  });

  process.on('SIGTERM', () => {
    cleanup();
    process.exit(0);
  });

  process.on('beforeExit', cleanup);

  await render(() => <App {...(options ?? {})} />, {
    exitOnCtrlC: false,
    targetFps: 60,
    useKittyKeyboard: {},
  } as any);
}
