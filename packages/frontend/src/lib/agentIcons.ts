import { ownValue } from "@/lib/safeRecord";

/**
 * Agent (client) icons shown in the UI, vendored from the logo column of the
 * README's Overview table into public/assets/v2/agents/ (provenance in
 * SOURCES.json there). Each is a 64x64 tile: transparent icons sit on white, as
 * on GitHub, and wide ones are fitted into the square. Clients that share a
 * README image share one file. 9Router has no README row; its icon is the
 * favicon of https://9router.com.
 *
 * SOURCE_LOGOS in lib/constants stays the absolute-URL registry that
 * server-rendered embeds use.
 */
export const AGENT_ICONS: Readonly<Record<string, string>> = {
  aside: "/assets/v2/agents/client-aside.webp",
  opencode: "/assets/v2/agents/client-opencode.webp",
  claude: "/assets/v2/agents/client-claude.webp",
  codex: "/assets/v2/agents/client-openai.webp",
  mcode: "/assets/v2/agents/gh-minimax-ai.webp",
  copilot: "/assets/v2/agents/client-copilot.webp",
  gemini: "/assets/v2/agents/client-gemini.webp",
  cursor: "/assets/v2/agents/client-cursor.webp",
  amp: "/assets/v2/agents/client-amp.webp",
  codebuff: "/assets/v2/agents/client-codebuff.webp",
  freebuff: "/assets/v2/agents/client-freebuff.webp",
  droid: "/assets/v2/agents/client-droid.webp",
  openclaw: "/assets/v2/agents/client-openclaw.webp",
  hermes: "/assets/v2/agents/client-hermes.webp",
  pi: "/assets/v2/agents/client-pi.webp",
  kimi: "/assets/v2/agents/client-kimi.webp",
  qwen: "/assets/v2/agents/client-qwen.webp",
  roocode: "/assets/v2/agents/client-roocode.webp",
  kilocode: "/assets/v2/agents/client-kilocode.webp",
  kilo: "/assets/v2/agents/client-kilocode.webp",
  mux: "/assets/v2/agents/client-mux.webp",
  kiro: "/assets/v2/agents/gh-kirodotdev.webp",
  crush: "/assets/v2/agents/client-crush.webp",
  goose: "/assets/v2/agents/client-goose.webp",
  antigravity: "/assets/v2/agents/client-antigravity.webp",
  "antigravity-cli": "/assets/v2/agents/client-antigravity.webp",
  "antigravity-extension": "/assets/v2/agents/client-antigravity.webp",
  zed: "/assets/v2/agents/client-zed.webp",
  trae: "/assets/v2/agents/client-trae.webp",
  warp: "/assets/v2/agents/gh-warpdotdev.webp",
  cline: "/assets/v2/agents/gh-cline.webp",
  synthetic: "/assets/v2/agents/client-synthetic.webp",
  gjc: "/assets/v2/agents/github-com-7246e920-f3f8-4b6e-847e-030ae04e86c2.webp",
  "9router": "/assets/v2/agents/9router.svg",
  grok: "/assets/v2/agents/gh-xai-org.webp",
  jcode: "/assets/v2/agents/client-jcode.webp",
  commandcode: "/assets/v2/agents/raw-githubusercontent-com-command-code-logo-black-bg.webp",
  micode: "/assets/v2/agents/gh-xiaomimimo.webp",
  junie: "/assets/v2/agents/gh-jetbrains.webp",
  zcode: "/assets/v2/agents/gh-zai-org.webp",
  opencodereview: "/assets/v2/agents/gh-alibaba.webp",
  codebuddy: "/assets/v2/agents/pc3-gtimg-com-43068-48-1764842447.webp",
  workbuddy: "/assets/v2/agents/static-workbuddy-cn-share-logo.webp",
  "devin-cli": "/assets/v2/agents/client-devin.webp",
  "devin-desktop": "/assets/v2/agents/client-devin.webp",
  senpi: "/assets/v2/agents/client-senpi.webp",
  augment: "/assets/v2/agents/gh-augmentcode.webp",
  kimchi: "/assets/v2/agents/gh-getkimchi.webp",
  reasonix: "/assets/v2/agents/client-synthetic.webp",
  "prime-agent": "/assets/v2/agents/gh-primeintellect-ai.webp",
  cherrystudio: "/assets/v2/agents/client-cherrystudio.webp",
  dsh: "/assets/v2/agents/gh-deepseek-ai.webp",
  fx: "/assets/v2/agents/client-fx.webp",
  omp: "/assets/v2/agents/omp-sh-favicon-180x180.webp",
  lmstudio: "/assets/v2/agents/gh-lmstudio-ai.webp",
  unsloth: "/assets/v2/agents/gh-unslothai.webp",
  hindsight: "/assets/v2/agents/client-hindsight.webp",
  "micode-desktop": "/assets/v2/agents/client-micode-desktop.webp",
  muse: "/assets/v2/agents/gh-meta-models.webp",
  vibe: "/assets/v2/agents/gh-mistralai.webp",
};

/** Vendored icon for a client id, or null when there is none. */
export function agentIconUrl(key: string): string | null {
  return ownValue(AGENT_ICONS, key) ?? null;
}
