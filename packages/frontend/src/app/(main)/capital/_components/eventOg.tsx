import { ImageResponse } from "next/og";
import { OG_BORDER, OG_SIZE, OG_TEXT, OG_TEXT_MUTED, OgCardShell } from "@/components/og/OgCard";
import { MEASURE_LABELS, QUANTITY_UNITS, formatPartialDate, sortForDisplay, type EventView } from "@/lib/capital/ledger";
import { loadOgFonts } from "@/lib/og/fonts";
import { titleCase } from "@/lib/text/titleCase";

/** The values a card shows: up to two amounts, then one quantity, at most three, each with its source. */
function cardStats(e: EventView) {
  const amounts = sortForDisplay(
    e.amounts.filter((a) => a.skipped === null && a.amount.status !== "reported" && a.amount.measure !== "price_per_share"),
    (a) => a.amount.measure
  );
  const quantity = e.quantities.find((q) => q.quantity.measure !== "shares" && q.quantity.measure !== "investors") ?? e.quantities[0];
  return [
    ...amounts.slice(0, 2).map((a) => ({ label: a.amount.label ?? MEASURE_LABELS[a.amount.measure], value: a.short, source: a.source })),
    ...(quantity
      ? [{ label: quantity.quantity.label ?? QUANTITY_UNITS[quantity.quantity.measure].label, value: quantity.short, source: quantity.source }]
      : []),
  ].slice(0, 3);
}

/** The card's source line names the publishers of the values it shows; a card without values names the event's first source. */
export function eventOgSourceLabel(e: EventView): string {
  const stats = cardStats(e);
  const sources = stats.length > 0 ? stats.map((s) => s.source) : e.sources.slice(0, 1);
  const publishers = [...new Set(sources.map((s) => s.publisher.toUpperCase()))];
  if (publishers.length === 0) return "";
  return `${publishers.length > 1 ? "SOURCES" : "SOURCE"}: ${publishers.join(", ")}`;
}

/**
 * 1200x630 Open Graph card for a round or deal page, served at <event page>/og by a route
 * handler, so the URL is stable. Numbers come from the event's sourced amounts; nothing is
 * computed here.
 */
export async function eventOgImage(e: EventView): Promise<Response> {
  const eyebrow = titleCase(`${e.tag} · ${e.statusLabel} · ${formatPartialDate(e.date)}`);
  const title = e.title.replace(/\u2019/g, "'");
  const stats = cardStats(e);
  const source = eventOgSourceLabel(e);
  // The card font has no arrow glyph; the renderer would swap in an emoji.
  const parties = e.partiesText.replace(/ ↔ /g, " and ");
  const fonts = await loadOgFonts(`${eyebrow}${title}${parties}${stats.map((s) => s.label + s.value).join("")}${source}≈≤≥>`);

  return new ImageResponse(
    (
      <OgCardShell>
        <div style={{ display: "flex", flexDirection: "column", marginTop: 28 }}>
          <div style={{ display: "flex", fontSize: 20, color: OG_TEXT_MUTED }}>{eyebrow}</div>
          <div style={{ display: "flex", marginTop: 14, fontSize: 52, fontWeight: 700, color: OG_TEXT, lineHeight: 1.1 }}>{title}</div>
          <div style={{ display: "flex", marginTop: 12, fontSize: 24, color: OG_TEXT_MUTED }}>{parties}</div>
        </div>
        <div style={{ display: "flex", flex: 1 }} />
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <div style={{ display: "flex", height: 1, background: OG_BORDER }} />
          <div style={{ display: "flex", gap: 40 }}>
            {stats.map((s) => (
              <div key={s.label} style={{ display: "flex", flexDirection: "column", flexShrink: 0 }}>
                <div style={{ display: "flex", flexShrink: 0, fontSize: s.value.length > 10 ? 38 : 44, fontWeight: 600, color: OG_TEXT, whiteSpace: "nowrap" }}>
                  {s.value}
                </div>
                <div style={{ display: "flex", maxWidth: 330, marginTop: 4, fontSize: 17, color: OG_TEXT_MUTED, lineHeight: 1.35 }}>
                  {s.label}
                </div>
              </div>
            ))}
          </div>
          <div style={{ display: "flex", fontSize: 16, color: OG_TEXT_MUTED }}>{source}</div>
        </div>
      </OgCardShell>
    ),
    { ...OG_SIZE, fonts }
  );
}
