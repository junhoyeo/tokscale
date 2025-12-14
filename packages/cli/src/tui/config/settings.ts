import { homedir } from "os";
import { join } from "path";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "fs";

const CONFIG_DIR = join(homedir(), ".config", "tokscale");
const CONFIG_FILE = join(CONFIG_DIR, "tui-settings.json");

interface TUISettings {
  colorPalette: string;
}

export function loadSettings(): TUISettings {
  try {
    if (existsSync(CONFIG_FILE)) {
      return JSON.parse(readFileSync(CONFIG_FILE, "utf-8"));
    }
  } catch {
  }
  return { colorPalette: "green" };
}

export function saveSettings(settings: TUISettings): void {
  if (!existsSync(CONFIG_DIR)) {
    mkdirSync(CONFIG_DIR, { recursive: true });
  }
  writeFileSync(CONFIG_FILE, JSON.stringify(settings, null, 2));
}
