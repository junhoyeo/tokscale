#!/usr/bin/env bun
/**
 * Build a standalone gallery.html showcasing every embed card template.
 *
 * Bundles the live renderers for the browser and embeds sample data, so the
 * result is a single self-contained file with Template / Color / Theme
 * controls. Run with: `bun run gallery` (from packages/frontend).
 */
import { writeFileSync } from "node:fs";
import { join } from "node:path";

function sampleContributions() {
  const out: { date: string; totalTokens: number; totalCost: number; intensity: number }[] = [];
  const today = new Date();
  for (let i = 364; i >= 0; i--) {
    const d = new Date(today);
    d.setUTCDate(d.getUTCDate() - i);
    const seed = (i * 2654435761) >>> 0;
    const intensity = i > 250 ? 0 : seed % 5;
    out.push({
      date: d.toISOString().slice(0, 10),
      totalTokens: intensity * 32_000_000,
      totalCost: intensity * 42,
      intensity,
    });
  }
  return out;
}

const galleryData = {
  username: "your-name",
  data: {
    user: { id: "sample", username: "your-name", displayName: "Your Name", avatarUrl: null },
    stats: {
      totalTokens: 20_941_000_000,
      totalCost: 14_512,
      submissionCount: 120,
      rank: 134,
      rankTotal: 1174,
      updatedAt: new Date().toISOString(),
    },
  },
  contributions: sampleContributions(),
};

const built = await Bun.build({
  entrypoints: [join(import.meta.dir, "gallery-app.ts")],
  target: "browser",
  minify: true,
});
if (!built.success) {
  console.error(built.logs);
  process.exit(1);
}
const appJs = await built.outputs[0].text();

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Tokscale Embed Templates — Gallery</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body {
    margin: 0; min-height: 100vh;
    background:
      radial-gradient(900px 480px at 82% -8%, rgba(56,139,253,0.16), transparent 70%),
      radial-gradient(720px 420px at 0% 100%, rgba(163,113,247,0.12), transparent 70%),
      #0d1117;
    color: #e6edf3;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  }
  .wrap { max-width: 1000px; margin: 0 auto; padding: 52px 24px 72px; }
  header { text-align: center; margin-bottom: 30px; }
  h1 { margin: 0 0 8px; font-size: 30px; font-weight: 800; letter-spacing: -0.025em; }
  .lead { margin: 0; color: #8b949e; font-size: 15px; }
  .controls {
    display: flex; flex-wrap: wrap; justify-content: center; gap: 14px;
    padding: 18px; margin: 0 auto; max-width: 720px;
    border: 1px solid #21262d; border-radius: 18px; background: rgba(22,27,34,0.6);
  }
  .field { display: flex; flex-direction: column; gap: 7px; min-width: 170px; flex: 1; }
  .field label {
    font-size: 11px; font-weight: 700; letter-spacing: 0.08em;
    text-transform: uppercase; color: #8b949e; padding-left: 2px;
  }
  select {
    appearance: none; -webkit-appearance: none;
    padding: 11px 38px 11px 14px;
    border: 1px solid #30363d; border-radius: 11px; background: #161b22;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 24 24' fill='none' stroke='%238b949e' stroke-width='2.5' stroke-linecap='round'><path d='M6 9l6 6 6-6'/></svg>");
    background-repeat: no-repeat; background-position: right 12px center;
    color: #e6edf3; font-size: 14px; font-weight: 600; cursor: pointer;
  }
  select:hover { border-color: #58a6ff; }
  .stage {
    display: flex; align-items: center; justify-content: center;
    min-height: 360px; padding: 40px 26px; margin-top: 16px;
    border: 1px solid #21262d; border-radius: 22px;
    background: linear-gradient(180deg, rgba(22,27,34,0.55), rgba(13,17,23,0.55));
  }
  .stage svg { max-width: 100%; height: auto; border-radius: 14px; filter: drop-shadow(0 22px 50px rgba(0,0,0,0.5)); }
  .urlbar { display: flex; align-items: center; gap: 10px; margin: 22px auto 0; max-width: 720px; }
  .url {
    flex: 1; overflow-x: auto; white-space: nowrap;
    padding: 12px 14px; border: 1px solid #30363d; border-radius: 11px;
    background: #161b22; color: #a5d6ff; font-size: 13px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }
  .copy {
    padding: 12px 16px; border: 1px solid #30363d; border-radius: 11px;
    background: #21262d; color: #e6edf3; font-size: 13px; font-weight: 700; cursor: pointer;
  }
  .copy:hover { background: #2d333b; border-color: #58a6ff; }
</style>
</head>
<body>
<div class="wrap">
  <header>
    <h1>Tokscale Embed Templates</h1>
    <p class="lead">Every card design at a glance. Pick a template, accent color, and theme.</p>
  </header>
  <div class="controls">
    <div class="field"><label for="tpl">Template</label><select id="tpl"></select></div>
    <div class="field"><label for="col">Accent color</label><select id="col"></select></div>
    <div class="field"><label for="thm">Theme</label><select id="thm"></select></div>
    <div class="field"><label for="tok">Token format</label><select id="tok"></select></div>
    <div class="field"><label for="cst">Cost format</label><select id="cst"></select></div>
    <div class="field"><label for="rnk">Rank format</label><select id="rnk"></select></div>
    <div class="field"><label for="srt">Ranking</label><select id="srt"></select></div>
    <div class="field"><label for="gph">Graph</label><select id="gph"></select></div>
  </div>
  <div class="stage"><div id="card"></div></div>
  <div class="urlbar">
    <div class="url" id="url"></div>
    <button class="copy" id="copy" type="button">Copy URL</button>
  </div>
</div>
<script>window.__GALLERY__ = ${JSON.stringify(galleryData)};</script>
<script type="module">${appJs}</script>
</body>
</html>
`;

const outPath = join(import.meta.dir, "gallery.html");
writeFileSync(outPath, html);
console.log(`wrote ${outPath} (${(html.length / 1024).toFixed(0)} KB)`);
