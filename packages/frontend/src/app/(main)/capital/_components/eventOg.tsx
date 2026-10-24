import { ImageResponse } from "next/og";
import { OG_BORDER, OG_SIZE, OG_TEXT, OG_TEXT_MUTED, OgCardShell } from "@/components/og/OgCard";
import { MEASURE_LABELS, QUANTITY_UNITS, formatPartialDate, sortForDisplay, type EventView } from "@/lib/capital/ledger";
import { loadOgFonts } from "@/lib/og/fonts";
import { titleCase } from "@/lib/text/titleCase";

const ACCENT = "#53d1f3";

/**
 * 1200x630 Open Graph card for a round or deal page, served at <event page>/og by a route
 * handler, so the URL is stable. Numbers come from the event's sourced amounts; nothing is
 * computed here.
 */
export async function eventOgImage(e: EventView): Promise<Response> {
  const eyebrow = titleCase(`${e.tag} · ${e.statusLabel} · ${formatPartialDate(e.date)}`);
  const title = e.title.replace(/\u2019/g, "'");
  const amounts = sortForDisplay(
    e.amounts.filter((a) => a.skipped === null && a.amount.status !== "reported" && a.amount.measure !== "price_per_share"),
    (a) => a.amount.measure
  );
  const quantity = e.quantities.find((q) => q.quantity.measure !== "shares" && q.quantity.measure !== "investors") ?? e.quantities[0];
  const stats = [
    ...amounts.slice(0, 2).map((a) => ({ label: (a.amount.label ?? MEASURE_LABELS[a.amount.measure]).toUpperCase(), value: a.short })),
    ...(quantity ? [{ label: (quantity.quantity.label ?? QUANTITY_UNITS[quantity.quantity.measure].label).toUpperCase(), value: quantity.short }] : []),
  ].slice(0, 3);
  const source = e.sources[0] ? `SOURCE: ${e.sources[0].publisher.toUpperCase()}` : "";
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
                <div style={{ display: "flex", flexShrink: 0, fontSize: s.value.length > 10 ? 38 : 44, fontWeight: 700, color: ACCENT, whiteSpace: "nowrap" }}>
                  {s.value}
                </div>
                <div style={{ display: "flex", maxWidth: 330, marginTop: 4, fontSize: 16, color: OG_TEXT_MUTED, letterSpacing: 2, lineHeight: 1.35 }}>
                  {s.label}
                </div>
              </div>
            ))}
          </div>
          <div style={{ display: "flex", fontSize: 16, color: OG_TEXT_MUTED, letterSpacing: 2 }}>{source}</div>
        </div>
      </OgCardShell>
    ),
    { ...OG_SIZE, fonts }
  );
}
