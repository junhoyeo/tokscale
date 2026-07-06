/**
 * gallery-app.ts — the browser side of the embed template gallery.
 *
 * Bundled by build-gallery.ts into a standalone gallery.html. It calls the
 * real embed renderers live in the browser, so the Template / Color / Theme
 * controls re-render the preview instantly.
 */
import { renderProfileEmbedSvg } from "../../src/lib/embed/renderProfileEmbedSvg";
import { renderMinimalEmbedSvg } from "../../src/lib/embed/renderMinimalEmbedSvg";
import { renderTerminalEmbedSvg } from "../../src/lib/embed/renderTerminalEmbedSvg";
import { renderGraphEmbedSvg } from "../../src/lib/embed/renderGraphEmbedSvg";
import { renderOrbitEmbedSvg } from "../../src/lib/embed/renderOrbitEmbedSvg";
import { renderVitalsEmbedSvg } from "../../src/lib/embed/renderVitalsEmbedSvg";
import { renderBlueprintEmbedSvg } from "../../src/lib/embed/renderBlueprintEmbedSvg";
import { renderReceiptEmbedSvg } from "../../src/lib/embed/renderReceiptEmbedSvg";
import { EMBED_TEMPLATES } from "../../src/lib/embed/embedShared";
import { getPaletteNames } from "../../src/lib/themes";
import type { UserEmbedStats } from "../../src/lib/embed/getUserEmbedStats";

type Renderer = (data: UserEmbedStats, options: Record<string, unknown>) => string;

const RENDERERS: Record<string, Renderer> = {
  classic: renderProfileEmbedSvg,
  minimal: renderMinimalEmbedSvg,
  terminal: renderTerminalEmbedSvg,
  graph: renderGraphEmbedSvg,
  orbit: renderOrbitEmbedSvg,
  vitals: renderVitalsEmbedSvg,
  blueprint: renderBlueprintEmbedSvg,
  receipt: renderReceiptEmbedSvg,
};

interface GalleryData {
  username: string;
  data: UserEmbedStats;
  contributions: { date: string; totalTokens: number; totalCost: number; intensity: number }[];
}

const G = (window as unknown as { __GALLERY__: GalleryData }).__GALLERY__;
const COLORS = ["default", ...getPaletteNames()];
const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
const $ = (id: string) => document.getElementById(id) as HTMLElement;

const state = { template: "classic", color: "default", theme: "dark", tokens: "compact", cost: "compact", rank: "plain", sort: "tokens", graph: "off" };
const GRAPH_CAPABLE = new Set(["classic", "minimal", "terminal", "orbit", "blueprint", "receipt"]);

function fillSelect(el: HTMLSelectElement, values: string[]) {
  el.innerHTML = values.map((v) => `<option value="${v}">${cap(v)}</option>`).join("");
}

function render() {
  const renderer = RENDERERS[state.template] ?? renderProfileEmbedSvg;
  const isClassic = state.template === "classic";
  // Only some templates support the graph; classic toggles it via the
  // presence of contributions, the rest via an explicit `graph` boolean.
  const graphCapable = GRAPH_CAPABLE.has(state.template);
  const graphOn = graphCapable && state.graph === "on";
  const svg = renderer(G.data, {
    theme: state.theme,
    color: state.color === "default" ? null : state.color,
    contributions: isClassic && !graphOn ? null : G.contributions,
    tokensFormat: state.tokens,
    costFormat: state.cost,
    rankFormat: state.rank,
    sortBy: state.sort,
    graph: graphOn,
  }).replace(/^<\?xml[^>]*\?>\s*/, "");
  $("card").innerHTML = svg;
  (gph.closest(".field") as HTMLElement).style.display = graphCapable ? "" : "none";

  const params = [`template=${state.template}`];
  if (state.color !== "default") params.push(`color=${state.color}`);
  if (state.theme !== "dark") params.push(`theme=${state.theme}`);
  if (graphOn) params.push("graph=1");
  if (state.tokens !== "compact") params.push(`tokens=${state.tokens}`);
  if (state.cost !== "compact") params.push(`cost=${state.cost}`);
  if (state.rank !== "plain") params.push(`rank=${state.rank}`);
  if (state.sort !== "tokens") params.push(`sort=${state.sort}`);
  $("url").textContent = `https://tokscale.ai/api/embed/${G.username}/svg?${params.join("&")}`;
}

const tpl = $("tpl") as HTMLSelectElement;
const col = $("col") as HTMLSelectElement;
const thm = $("thm") as HTMLSelectElement;
const tok = $("tok") as HTMLSelectElement;
const cst = $("cst") as HTMLSelectElement;
const rnk = $("rnk") as HTMLSelectElement;
const srt = $("srt") as HTMLSelectElement;
const gph = $("gph") as HTMLSelectElement;
fillSelect(tpl, [...EMBED_TEMPLATES]);
fillSelect(col, COLORS);
fillSelect(thm, ["dark", "light"]);
fillSelect(tok, ["compact", "full"]);
fillSelect(cst, ["compact", "full"]);
fillSelect(rnk, ["plain", "percent", "total"]);
fillSelect(srt, ["tokens", "cost"]);
fillSelect(gph, ["off", "on"]);

tpl.addEventListener("change", () => {
  state.template = tpl.value;
  render();
});
col.addEventListener("change", () => {
  state.color = col.value;
  render();
});
thm.addEventListener("change", () => {
  state.theme = thm.value;
  render();
});
tok.addEventListener("change", () => {
  state.tokens = tok.value;
  render();
});
cst.addEventListener("change", () => {
  state.cost = cst.value;
  render();
});
rnk.addEventListener("change", () => {
  state.rank = rnk.value;
  render();
});
srt.addEventListener("change", () => {
  state.sort = srt.value;
  render();
});
gph.addEventListener("change", () => {
  state.graph = gph.value;
  render();
});
$("copy").addEventListener("click", () => {
  navigator.clipboard?.writeText($("url").textContent ?? "").catch(() => {});
});

render();
