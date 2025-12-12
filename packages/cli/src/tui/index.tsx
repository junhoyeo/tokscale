import { render } from "ink";
import { App } from "./App.js";

export async function launchTUI() {
  const { waitUntilExit } = render(<App />);
  await waitUntilExit();
}
