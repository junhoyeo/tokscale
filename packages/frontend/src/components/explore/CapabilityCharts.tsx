"use client";

/**
 * Two scatter charts under the /models table:
 * - Usage vs capability: active users (log scale) against the Epoch
 *   Capabilities Index. Models with an ECI but no usage in the period are
 *   listed under the chart, because zero cannot sit on a log scale.
 * - Price vs capability: the estimated $/1M at the community token mix (log
 *   scale) against ECI, with the price frontier.
 *
 * The plots are pictures (role="img" with a text summary); every value is
 * also in a data table under "Show data".
 */
import styled from "styled-components";
import { linearDomain, linearPosition, linearTicks, logDomain, logPosition, logTicks, paretoFrontier } from "@/lib/benchmarks/charts";
import { formatPricePerMillion } from "@/lib/benchmarks/format";
import { modelPath } from "@/lib/seo/explore";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { InlineLink, ModelMark, Section, SectionNote, SectionTitle, VisuallyHidden } from "./ExploreUI";
import { formatCount, formatShare } from "./format";

export interface ChartModel {
  key: string;
  name: string;
  logoUrl: string | null;
  eci: number;
  activeUsers: number | null;
  /** Estimated $/1M at the community token mix, or null. */
  price: number | null;
}

/* Top-aligned, so opening one chart's data table does not stretch the chart beside it. */
const Grid = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  align-items: start;
  gap: 32px;

  @media (min-width: 1100px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 32px 48px;
  }
`;

const Figure = styled.figure`
  display: grid;
  align-content: start;
  gap: 8px;
  min-width: 0;
  margin: 0;
`;

const Plot = styled.div`
  position: relative;
  height: 320px;
  margin: 8px 8px 28px 48px;
  border-bottom: 1px solid var(--service-border-strong);
  border-left: 1px solid var(--service-border-strong);

  @media (max-width: 640px) {
    height: 260px;
    margin-left: 40px;
  }
`;

const Svg = styled.svg`
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  overflow: visible;
`;

const Mark = styled(InstantTooltip)`
  position: absolute;
  display: inline-flex;
  border-radius: 7px;
  transform: translate(-50%, -50%);

  &[data-frontier="true"] {
    outline: 1px solid var(--service-accent-hover);
    outline-offset: 1px;
  }
`;

/* --x is the point's left offset; the label never runs past the plot's edge on narrow screens. */
const PointLabel = styled.span`
  position: absolute;
  max-width: min(150px, calc(100% - var(--x, 0%) - 16px));
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.6875rem;
  text-overflow: ellipsis;
  white-space: nowrap;
  transform: translate(14px, -50%);
  pointer-events: none;

  &[data-flip="true"] {
    max-width: min(150px, calc(var(--x, 100%) - 16px));
    transform: translate(calc(-100% - 14px), -50%);
  }

  /* Phones keep only the first few labels; the data table has every value. */
  @media (max-width: 640px) {
    &[data-minor="true"] {
      display: none;
    }
  }
`;

const Tick = styled.span`
  position: absolute;
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const AxisTitle = styled.span`
  position: absolute;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

const Caption = styled.figcaption`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

const Strip = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.6;

  a {
    font-size: inherit;
  }
`;

const DataDetails = styled.details`
  > summary {
    display: inline-flex;
    min-height: 32px;
    align-items: center;
    color: var(--service-accent-hover);
    font-size: 0.8125rem;
    cursor: pointer;

    @media (max-width: 640px), (pointer: coarse) {
      min-height: 44px;
      font-size: 0.875rem;
    }
  }

  > summary:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const DataWrap = styled.div`
  max-height: 320px;
  overflow: auto;
  border-top: 1px solid var(--service-border);
`;

const DataTable = styled.table`
  width: 100%;
  border-collapse: collapse;
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;

  th,
  td {
    padding: 6px 8px 6px 0;
    border-bottom: 1px solid var(--service-border);
    text-align: right;
  }

  th:first-child,
  td:first-child {
    text-align: left;
  }

  th {
    color: var(--service-text-muted);
    font-weight: 500;
  }
`;

interface ScatterPoint {
  key: string;
  name: string;
  logoUrl: string | null;
  x: number;
  y: number;
}

interface ScatterProps {
  points: ScatterPoint[];
  xLabel: string;
  yLabel: string;
  formatX: (value: number) => string;
  frontierKeys?: Set<string>;
  /** Points to label by name, most important first. */
  labelKeys: string[];
  summary: string;
}

function Scatter({ points, xLabel, yLabel, formatX, frontierKeys, labelKeys, summary }: ScatterProps) {
  const xDomain = logDomain(points.map((p) => p.x));
  const yDomain = linearDomain(points.map((p) => p.y), 5);
  if (!xDomain || !yDomain) return null;
  const xTicks = logTicks(xDomain[0], xDomain[1]);
  const yTicks = linearTicks(yDomain[0], yDomain[1], 5);
  const px = (x: number) => logPosition(x, xDomain[0], xDomain[1]) * 100;
  const py = (y: number) => 100 - linearPosition(y, yDomain[0], yDomain[1]) * 100;
  const frontier = frontierKeys ? points.filter((p) => frontierKeys.has(p.key)).sort((a, b) => a.x - b.x) : [];
  const labels = placeLabels(points, labelKeys, px, py);
  // Step line: horizontal to the next cheaper-and-better point, then up.
  const path = frontier
    .map((p, i) => {
      const x = px(p.x) * 10;
      const y = py(p.y) * 10;
      if (i === 0) return `M${x} ${y}`;
      return `L${x} ${py(frontier[i - 1].y) * 10} L${x} ${y}`;
    })
    .join(" ");

  return (
    <Plot role="img" aria-label={summary}>
      <Svg viewBox="0 0 1000 1000" preserveAspectRatio="none" aria-hidden="true" focusable="false">
        {yTicks.map((t) => (
          <line key={`y${t}`} x1={0} x2={1000} y1={py(t) * 10} y2={py(t) * 10} stroke="rgba(255,255,255,0.07)" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        ))}
        {xTicks.map((t) => (
          <line key={`x${t}`} x1={px(t) * 10} x2={px(t) * 10} y1={0} y2={1000} stroke="rgba(255,255,255,0.05)" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        ))}
        {path ? <path d={path} fill="none" stroke="var(--service-accent-hover)" strokeWidth={1.5} strokeDasharray="4 3" vectorEffect="non-scaling-stroke" /> : null}
      </Svg>
      {yTicks.map((t) => (
        <Tick key={`yl${t}`} aria-hidden="true" style={{ top: `${py(t)}%`, right: "calc(100% + 8px)", transform: "translateY(-50%)" }}>
          {t}
        </Tick>
      ))}
      {xTicks.map((t) => (
        <Tick key={`xl${t}`} aria-hidden="true" style={{ left: `${px(t)}%`, top: "calc(100% + 6px)", transform: "translateX(-50%)" }}>
          {formatX(t)}
        </Tick>
      ))}
      <AxisTitle aria-hidden="true" style={{ left: 0, top: "-22px" }}>
        {yLabel}
      </AxisTitle>
      <AxisTitle aria-hidden="true" style={{ right: 0, top: "calc(100% + 6px)", transform: "translateY(100%)" }}>
        {xLabel}
      </AxisTitle>
      {points.map((p) => {
        const labelRank = [...labels.keys()].indexOf(p.key);
        const left = px(p.x);
        const top = py(p.y);
        return (
          <span key={p.key} aria-hidden="true">
            <Mark
              style={{ left: `${left}%`, top: `${top}%` }}
              content={`${p.name}: ${yLabel} ${p.y.toFixed(1)}, ${xLabel} ${formatX(p.x)}`}
              describe={false}
              data-frontier={frontierKeys?.has(p.key) ? "true" : undefined}
            >
              <ModelMark model={p} size={20} />
            </Mark>
            {labels.has(p.key) ? (
              <PointLabel
                style={{ left: `${left}%`, top: `${top}%`, "--x": `${left}%` } as React.CSSProperties}
                data-flip={labels.get(p.key) === "left" ? "true" : undefined}
                data-minor={labelRank >= 3 ? "true" : undefined}
              >
                {p.name}
              </PointLabel>
            ) : null}
          </span>
        );
      })}
    </Plot>
  );
}

/**
 * Decide which point labels to draw and on which side, greedily in priority
 * order, skipping a label that would overlap one already placed. Sizes are
 * estimates in percent of a ~560 x 320 px plot.
 */
function placeLabels(points: ScatterPoint[], priority: string[], px: (x: number) => number, py: (y: number) => number): Map<string, "right" | "left"> {
  const charWidth = 1.15;
  const height = 4.2;
  const markHalf = 2.2;
  const placed: Array<{ x0: number; x1: number; y0: number; y1: number }> = points.map((p) => ({
    x0: px(p.x) - markHalf,
    x1: px(p.x) + markHalf,
    y0: py(p.y) - height / 2,
    y1: py(p.y) + height / 2,
  }));
  const out = new Map<string, "right" | "left">();
  for (const key of priority) {
    const p = points.find((q) => q.key === key);
    if (!p) continue;
    const x = px(p.x);
    const y = py(p.y);
    const width = Math.min(26, p.name.length * charWidth);
    const options: Array<["right" | "left", number, number]> = [
      ["right", x + markHalf + 0.6, x + markHalf + 0.6 + width],
      ["left", x - markHalf - 0.6 - width, x - markHalf - 0.6],
    ];
    for (const [side, x0, x1] of options) {
      if (x0 < 0 || x1 > 100) continue;
      const box = { x0, x1, y0: y - height / 2, y1: y + height / 2 };
      const own = (b: (typeof placed)[number]) => Math.abs((b.x0 + b.x1) / 2 - x) < 0.01 && Math.abs((b.y0 + b.y1) / 2 - y) < 0.01;
      const clash = placed.some((b) => !own(b) && b.x0 < box.x1 && box.x0 < b.x1 && b.y0 < box.y1 && box.y0 < b.y1);
      if (!clash) {
        out.set(key, side);
        placed.push(box);
        break;
      }
    }
  }
  return out;
}

function topKeys<T extends { key: string }>(items: readonly T[], score: (item: T) => number, n: number): string[] {
  return [...items].sort((a, b) => score(b) - score(a)).slice(0, n).map((i) => i.key);
}

export function CapabilityCharts({
  models,
  periodLabel,
  cacheReadShare,
}: {
  models: ChartModel[];
  periodLabel: string;
  /** Cache reads' share of the community's tokens in the period, or null without usage data. */
  cacheReadShare: number | null;
}) {
  const used = models.filter((m) => m.activeUsers !== null && m.activeUsers > 0);
  const unused = models.filter((m) => m.activeUsers === null || m.activeUsers === 0).sort((a, b) => b.eci - a.eci);
  const usagePoints = used.map((m) => ({ key: m.key, name: m.name, logoUrl: m.logoUrl, x: m.activeUsers ?? 0, y: m.eci }));
  // Most used, the highest ECI, and the highest ECI among rarely used models (the gaps).
  const usageLabels = [
    ...topKeys(used, (m) => m.activeUsers ?? 0, 6),
    ...topKeys(used, (m) => m.eci, 3),
    ...topKeys(used.filter((m) => (m.activeUsers ?? 0) < 10), (m) => m.eci, 2),
  ];

  const priced = models.filter((m) => m.price !== null && m.price > 0);
  const pricePoints = priced.map((m) => ({ key: m.key, name: m.name, logoUrl: m.logoUrl, x: m.price ?? 0, y: m.eci }));
  const frontier = paretoFrontier(pricePoints.map((p) => ({ key: p.key, x: p.x, y: p.y })));
  const frontierKeys = new Set(frontier.map((p) => p.key));

  const topUsed = [...used].sort((a, b) => (b.activeUsers ?? 0) - (a.activeUsers ?? 0))[0];
  const topEci = [...models].sort((a, b) => b.eci - a.eci)[0];

  return (
    <Section aria-labelledby="charts-title">
      <div>
        <SectionTitle id="charts-title">Usage, capability and price</SectionTitle>
        <SectionNote>
          The models in the table that have an Epoch Capabilities Index. These charts put usage and scores side by side; they
          do not show that one causes the other.
        </SectionNote>
      </div>
      <Grid>
        <Figure>
          <Scatter
            points={usagePoints}
            xLabel="Active users (log scale)"
            yLabel="ECI"
            formatX={(v) => formatCount(v)}
            labelKeys={usageLabels}
            summary={`Scatter chart of ${usagePoints.length} models: active users in the ${periodLabel.toLowerCase()} against the Epoch Capabilities Index.${topUsed ? ` Most used: ${topUsed.name} with ${formatCount(topUsed.activeUsers ?? 0)} active users and ECI ${topUsed.eci.toFixed(1)}.` : ""}${topEci ? ` Highest ECI: ${topEci.name} at ${topEci.eci.toFixed(1)}.` : ""} Full values are in the table below the chart.`}
          />
          <Caption>
            <strong>Usage vs capability.</strong> Active users in the {periodLabel.toLowerCase()} (log scale) against the Epoch
            Capabilities Index.
          </Caption>
          {unused.length > 0 ? (
            <Strip>
              ECI but no Tokscale usage in this period:{" "}
              {unused.map((m, i) => (
                <span key={m.key}>
                  {i > 0 ? ", " : null}
                  <InlineLink href={modelPath(m.key)}>{m.name}</InlineLink> ({m.eci.toFixed(1)})
                </span>
              ))}
              .
            </Strip>
          ) : null}
          <DataDetails>
            <summary>Show data</summary>
            <DataWrap>
              <DataTable>
                <caption>
                  <VisuallyHidden>Active users and Epoch Capabilities Index by model</VisuallyHidden>
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Model</th>
                    <th scope="col">Active users</th>
                    <th scope="col">ECI</th>
                  </tr>
                </thead>
                <tbody>
                  {[...models].sort((a, b) => b.eci - a.eci).map((m) => (
                    <tr key={m.key}>
                      <td>{m.name}</td>
                      <td>{m.activeUsers ? formatCount(m.activeUsers) : "No usage"}</td>
                      <td>{m.eci.toFixed(1)}</td>
                    </tr>
                  ))}
                </tbody>
              </DataTable>
            </DataWrap>
          </DataDetails>
        </Figure>

        <Figure>
          {pricePoints.length > 0 ? (
            <Scatter
              points={pricePoints}
              xLabel="Est. $/1M at agent mix (log scale)"
              yLabel="ECI"
              formatX={(v) => formatPricePerMillion(v)}
              frontierKeys={frontierKeys}
              labelKeys={[...topKeys(priced, (m) => m.eci, 3), ...frontier.map((p) => p.key).reverse()]}
              summary={`Scatter chart of ${pricePoints.length} models: estimated price per million tokens at the community token mix against the Epoch Capabilities Index. Models on the price frontier: ${frontier.map((p) => pricePoints.find((q) => q.key === p.key)?.name).join(", ")}. Full values are in the table below the chart.`}
            />
          ) : (
            <SectionNote>Price estimates need usage data, which is not available here.</SectionNote>
          )}
          <Caption>
            <strong>Price vs capability.</strong> Estimated USD per 1M tokens at the community token mix in the{" "}
            {periodLabel.toLowerCase()} (log scale, list prices
            {cacheReadShare !== null ? `; cache reads are ${formatShare(cacheReadShare)} of that mix` : ""}) against ECI. The
            dashed line is the price frontier: no model is both cheaper and higher on ECI than a model on it.
          </Caption>
          <DataDetails>
            <summary>Show data</summary>
            <DataWrap>
              <DataTable>
                <caption>
                  <VisuallyHidden>Estimated price at the community token mix and Epoch Capabilities Index by model</VisuallyHidden>
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Model</th>
                    <th scope="col">Est. $/1M</th>
                    <th scope="col">ECI</th>
                    <th scope="col">Frontier</th>
                  </tr>
                </thead>
                <tbody>
                  {[...priced].sort((a, b) => (a.price ?? 0) - (b.price ?? 0)).map((m) => (
                    <tr key={m.key}>
                      <td>{m.name}</td>
                      <td>{formatPricePerMillion(m.price)}</td>
                      <td>{m.eci.toFixed(1)}</td>
                      <td>{frontierKeys.has(m.key) ? "Yes" : ""}</td>
                    </tr>
                  ))}
                </tbody>
              </DataTable>
            </DataWrap>
          </DataDetails>
        </Figure>
      </Grid>
    </Section>
  );
}
