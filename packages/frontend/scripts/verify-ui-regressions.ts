/**
 * Real Chromium regressions for the local graph, sticky tooltips, tour, and Agent board.
 * Requires an existing agent-browser installation/browser and a running local Next server.
 * Does not install browsers, start servers, or write to a database. Models/Agents need local rows.
 *
 * Start Next with an explicit LOCAL DATABASE_URL, DATABASE_SSL=false, NEXT_TELEMETRY_DISABLED=1.
 * Run: bun --no-env-file scripts/verify-ui-regressions.ts http://localhost:3304 /tmp/tokscale-fix-ui
 * Results and screenshots stay outside the checkout. The isolated browser always closes.
 */
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { clientLine, exportDay, localExport } from "../__tests__/support/localExport";

const base = new URL(process.argv[2] ?? "http://localhost:3304");
assert(["localhost", "127.0.0.1", "[::1]"].includes(base.hostname), "Only a local app may be tested");
const out = resolve(process.argv[3] ?? "/tmp/tokscale-fix-ui");
const session = `ui-regression-${process.pid}`;
const results: Record<string, unknown> = {};
await mkdir(out, { recursive: true });

async function browser(...args: string[]) {
  const child = Bun.spawn(["agent-browser", "--session", session, "--json", ...args], { stdout: "pipe", stderr: "pipe" });
  const deadline = setTimeout(() => child.kill(), 60_000);
  try {
    const [stdout, stderr, code] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
    assert.equal(code, 0, `agent-browser ${args.join(" ")} failed: ${stdout} ${stderr}`);
    const reply = JSON.parse(stdout);
    assert(reply.success, JSON.stringify(reply));
    return reply.data;
  } finally {
    clearTimeout(deadline);
  }
}
async function evaluate<T>(js: string): Promise<T> {
  return (await browser("eval", "-b", Buffer.from(js).toString("base64"))).result as T;
}
async function wait(js: string) { await browser("wait", "--fn", js); }
async function shot(name: string) { await browser("screenshot", join(out, `${name}.png`)); }
async function move(point: { x: number; y: number }) { await browser("mouse", "move", String(Math.round(point.x)), String(Math.round(point.y))); }
const tips = `[id$="tooltip"]`;
const graph = `canvas[role="application"]`;
const change = `[aria-label="Models table"] tbody tr:first-child td[data-group-start] > span > span:last-child`;
const toggle = `button:has-text("tour")`;

function fixture(dates: string[], empty = false) {
  const data = localExport();
  const orderedDates = [...dates].sort();
  const year = orderedDates.at(-1)!.slice(0, 4);
  data.contributions = dates.map(date => exportDay(date, empty ? [] : [clientLine("claude", "claude-sonnet-4-5", 12, { input: 1500, output: 3000 }, 40)]));
  data.summary = { ...data.summary, totalTokens: empty ? 0 : dates.length * 4500, totalCost: empty ? 0 : dates.length * 12,
    activeDays: empty ? 0 : dates.length, totalDays: dates.length, averagePerDay: empty ? 0 : 12, maxCostInSingleDay: empty ? 0 : 12,
    clients: ["claude"], models: ["claude-sonnet-4-5"] };
  data.years = [{ year, totalTokens: data.summary.totalTokens, totalCost: data.summary.totalCost, range: { start: `${year}-01-01`, end: orderedDates.at(-1)! } }];
  data.meta = { ...data.meta, dateRange: { start: orderedDates[0], end: orderedDates.at(-1)! } };
  return data;
}
async function load(name: string, dates: string[], empty = false) {
  const file = join(out, `${name}.json`);
  await writeFile(file, JSON.stringify(fixture(dates, empty)));
  await browser("open", new URL("/local", base).href);
  await browser("upload", 'input[type="file"]', file);
  await wait(`document.querySelector('${graph}')?.width > 300`);
  await evaluate(`document.querySelector('${graph}').parentElement.scrollIntoView({block:'center'})`);
}
async function calendar(date: string) {
  return evaluate<{ visible: boolean; scrollLeft: number; viewport: number; x: number; right: number }>(`(() => {
    const c=document.querySelector('${graph}'), s=c.parentElement, r=c.getBoundingClientRect(), v=s.getBoundingClientRect();
    const date=new Date('${date}T00:00:00Z'), first=new Date(Date.UTC(date.getUTCFullYear(),0,1));
    first.setUTCDate(first.getUTCDate()-first.getUTCDay());
    const week=Math.floor((date-first)/604800000), pitch=(r.width-56)/53, x=r.left+32+week*pitch;
    const right=x+pitch-(pitch<16?2:3);
    return {visible:x>=v.left && right<=v.right,scrollLeft:s.scrollLeft,viewport:s.clientWidth,x,right};
  })()`);
}

try {
  await browser("open", new URL("/local", base).href);
  await browser("set", "viewport", "375", "812");
  for (const [name, dates] of [
    ["local-march", ["2026-03-02", "2026-03-26"]],
    ["local-current-year", [`${new Date().getFullYear()}-09-01`, `${new Date().getFullYear()}-10-20`]],
    ["local-old-export", ["2020-03-02", "2020-03-26"]],
  ] as const) {
    await load(name, [...dates]);
    const state = await calendar(dates[1]);
    assert(state.visible, `${name}: selected/latest activity must be visible: ${JSON.stringify(state)}`);
    results[name] = state;
    await shot(`${name}-375`);
  }

  await evaluate(`document.querySelector('${graph}').parentElement.scrollLeft=240`);
  await browser("set", "viewport", "390", "812");
  await wait(`document.querySelector('${graph}').parentElement.clientWidth > 321`);
  const afterResize = await calendar("2020-03-26");
  assert.equal(afterResize.scrollLeft, 240, "Resizing must preserve deliberate horizontal scrolling");
  results["calendar-resize"] = afterResize;
  await browser("focus", '[aria-label="Color palette"]');
  await browser("press", "Enter");
  await browser("press", "ArrowDown");
  await browser("press", "Enter");
  assert(await evaluate(`!document.querySelector('[role="listbox"]') && document.activeElement.getAttribute('aria-label')==='Color palette'`));
  assert.equal((await calendar("2020-03-26")).scrollLeft, 240, "Palette selection must not reset the scroller");
  await browser("focus", graph);
  await browser("press", "ArrowDown");
  await browser("press", "Enter");
  assert((await calendar("2020-03-26")).visible, "Keyboard focus must reveal the selected week");
  results["select-and-calendar-keyboard"] = "passed";

  await load("local-unsorted", ["2026-10-20", "2026-03-02"]);
  assert((await calendar("2026-10-20")).visible, "The /local caller must select the actual latest day of an unsorted export");
  assert(await evaluate(`document.querySelector('[aria-label="Day breakdown"]')?.textContent.includes('October 20, 2026')`));
  results["local-unsorted"] = await calendar("2026-10-20");
  await shot("local-unsorted-390");
  await load("local-empty", ["2026-03-02", "2026-12-31"], true);
  assert.equal((await calendar("2026-03-02")).scrollLeft, 0, "No activity must start at the beginning, not December");
  await shot("local-empty-390");
  const empty = fixture(["2026-03-02"], true);
  empty.contributions = [];
  empty.years = [];
  empty.summary.totalDays = 0;
  await writeFile(join(out, "local-no-years.json"), JSON.stringify(empty));
  await browser("open", new URL("/local", base).href);
  await browser("upload", 'input[type="file"]', join(out, "local-no-years.json"));
  await wait(`document.querySelector('${graph}')?.getAttribute('aria-label')==='Daily token usage in ${new Date().getFullYear()}'`);
  results["local-no-years"] = "empty calendar rendered without a date error";

  await browser("set", "viewport", "1000", "800");
  await browser("open", new URL("/models", base).href);
  await wait(`document.querySelector('${change}')`);
  const occluded = await evaluate<{ x: number; y: number; hit: string }>(`(() => {
    const s=document.querySelector('[aria-label="Models table"]'); s.scrollLeft=s.scrollWidth;
    const r=document.querySelector('${change}').getBoundingClientRect(), x=(r.left+r.right)/2,y=(r.top+r.bottom)/2;
    const hit=document.elementFromPoint(x,y); if(!hit?.closest('.model')) throw Error('Fixture needs an occluded change cell');
    return {x,y,hit:hit.textContent};
  })()`);
  await move(occluded);
  assert(await evaluate(`!document.querySelector('${tips}')`), "Covered active-user value must not show its tooltip");
  results["sticky-occlusion"] = occluded;
  await shot("models-sticky-occlusion-1000");

  const visible = await evaluate<{ x: number; y: number; href: string }>(`(() => {
    document.querySelector('[aria-label="Models table"]').scrollLeft=0;
    const r=document.querySelector('${change}').getBoundingClientRect(),x=(r.left+r.right)/2,y=(r.top+r.bottom)/2;
    const hit=document.elementFromPoint(x,y);if(!hit?.matches('[data-row-cover]'))throw Error('Expected stretched cover');
    return {x,y,href:hit.getAttribute('href')};
  })()`);
  await move(visible);
  await wait(`document.querySelector('${tips}')`);
  const tipPoint = await evaluate<{ x: number; y: number }>(`(() => {const r=document.querySelector('${tips}').getBoundingClientRect();return{x:(r.left+r.right)/2,y:(r.top+r.bottom)/2}})()`);
  await move(tipPoint);
  assert(await evaluate(`!!document.querySelector('${tips}')`), "Tooltip must remain hoverable across the row cover");
  await shot("models-hoverable-tooltip-1000");
  await browser("press", "Escape");
  assert(await evaluate(`!document.querySelector('${tips}')`), "Escape must dismiss the tooltip");
  // Leave the trigger's row-tracking box before entering again after Escape.
  await move({ x: visible.x + 110, y: visible.y });
  await move(visible);
  await wait(`document.querySelector('${tips}')`);
  await evaluate(`document.querySelector('[aria-label="Models table"]').scrollLeft=154`);
  await wait(`!document.querySelector('${tips}')`);
  results["tooltip-stationary-scroll"] = "closed when the pointer no longer hits the value";
  await evaluate(`document.querySelector('[aria-label="Models table"]').scrollLeft=0`);
  await move(visible);
  await browser("mouse", "down");
  await browser("mouse", "up");
  await wait(`location.pathname===${JSON.stringify(visible.href)}`);
  results["tooltip-hover-and-row-link"] = { href: visible.href, hoverable: true, escape: true };

  await browser("set", "viewport", "1440", "1000");
  await browser("open", new URL("/tracker", base).href);
  await wait(`document.querySelector('[role="tablist"][aria-label="Terminal UI views"]')`);
  await evaluate(`document.querySelector('[role="tablist"][aria-label="Terminal UI views"]').parentElement.parentElement.scrollIntoView({block:'center'})`);
  await browser("click", toggle);
  assert(await evaluate(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Play tour'))`));
  await browser("click", toggle);
  await browser("mouse", "move", "5", "80");
  await wait(`document.querySelector('[role="tab"][aria-selected="true"]').lastElementChild.getAnimations().some(a=>a.playState==='running' && a.currentTime>100)`);
  results["tour-retained-focus"] = await evaluate(`(() => {const p=document.querySelector('[role="tab"][aria-selected="true"]').lastElementChild;return{focused:document.activeElement.textContent,playState:getComputedStyle(p).animationPlayState,currentTime:p.getAnimations()[0].currentTime}})()`);
  assert(await evaluate(`document.activeElement.textContent.includes('Pause tour') && document.activeElement.getAttribute('aria-pressed')==='false'`));
  await shot("tracker-playing-focused-1440");
  await browser("hover", '[role="tabpanel"]');
  await wait(`getComputedStyle(document.querySelector('[role="tab"][aria-selected="true"]').lastElementChild).animationPlayState==='paused'`);
  await browser("mouse", "move", "5", "80");
  await browser("focus", '[role="tab"][aria-selected="true"]');
  await wait(`getComputedStyle(document.querySelector('[role="tab"][aria-selected="true"]').lastElementChild).animationPlayState==='paused'`);
  await browser("press", "ArrowRight");
  assert(await evaluate(`document.activeElement.matches('[role="tab"][aria-selected="true"]')`));
  await browser("focus", toggle);
  await browser("press", "Enter");
  await wait(`document.querySelector('[role="tab"][aria-selected="true"]').lastElementChild.getAnimations().some(a=>a.playState==='running' && a.currentTime>100)`);
  await browser("set", "media", "dark", "reduced-motion");
  await wait(`!Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('tour'))`);
  assert(await evaluate(`!document.querySelector('[role="tab"][aria-selected="true"]').lastElementChild.getAnimations().length`));
  results["tour-content-pause-keyboard-reduced-motion"] = "passed";
  await browser("set", "media", "dark", "no-preference");

  await browser("open", new URL("/leaderboard?view=agents", base).href);
  await wait(`document.querySelector('table tbody tr')`);
  for (const width of [840, 960, 1099, 1100, 1280]) {
    await browser("set", "viewport", String(width), "1000");
    await evaluate(`document.querySelector('main').scrollIntoView({block:'start'})`);
    const state = await evaluate<{ count: number; overflow: number; pageWidth: number }>(`(() => {
      const table=document.querySelector('table'), mobile=document.querySelector('ol[aria-label^="Agents ranked"]');
      const names = table.getBoundingClientRect().width
        ? [...table.querySelectorAll('tbody tr')].map(r=>({name:r.cells[1].querySelector('span > span'),bound:r.cells[1].getBoundingClientRect().right}))
        : [...mobile.children].map(r=>({name:r.querySelector('span[class*="MobileNameText"]'),bound:r.querySelector('span[class*="MobileName"]').getBoundingClientRect().right}));
      const overflows=names.map(({name,bound})=>{if(!name)throw Error('Missing agent name'); const range=document.createRange(); range.selectNodeContents(name);return Math.max(...[...range.getClientRects()].map(r=>r.right-bound),0)});
      return {count:names.length,overflow:Math.max(...overflows),pageWidth:document.documentElement.scrollWidth};
    })()`);
    assert(state.count>0, "Agent names must be present");
    assert(state.overflow<=1, `Agent name spills at ${width}px: ${JSON.stringify(state)}`);
    assert(state.pageWidth<=width, `Page overflow at ${width}px`);
    results[`agents-${width}`] = state;
    await shot(`agents-${width}`);
    if (width === 1100) {
      await evaluate(`(() => {const rows=[...document.querySelectorAll('table tbody tr')]; rows.sort((a,b)=>b.cells[1].textContent.length-a.cells[1].textContent.length); rows[0].scrollIntoView({block:'center'})})()`);
      await shot("agents-long-name-1100");
    }
  }
  console.log(JSON.stringify(results, null, 2));
} catch (error) {
  console.error("Browser regression failed:", error);
  throw error;
} finally {
  await writeFile(join(out, "browser-results.json"), JSON.stringify(results, null, 2));
  // Do not replace an assertion/navigation failure with a secondary daemon-close error.
  await browser("close").catch(error => console.error("Browser cleanup:", error));
}
