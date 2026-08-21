import { ImageResponse } from "next/og";
import {
  OG_SIZE,
  OG_TEXT,
  OG_TEXT_MUTED,
  OgCardShell,
  OgStat,
} from "@/components/og/OgCard";
import { getGroupBySlug, getGroupMemberCount } from "@/lib/groups/queries";
import { loadOgFonts } from "@/lib/og/fonts";

export const alt = "Group token usage on Tokscale";
export const size = OG_SIZE;
export const contentType = "image/png";

/**
 * Per-group Open Graph card.
 *
 * Private groups get the generic card: the page 404s for non-members, but this
 * route still runs, and rendering the name into a shareable image would leak it
 * to anyone who guessed the slug. Same rule as generateMetadata on the page.
 */
export default async function Image({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;

  const group = await getGroupBySlug(slug).catch(() => null);
  const isPublic = group?.isPublic === true;

  const memberCount = isPublic
    ? await getGroupMemberCount(group.id).catch(() => null)
    : null;

  const description = isPublic ? group.description?.trim() : null;

  // Group names and descriptions are free text, so CJK is considerably more
  // likely here than in a GitHub username.
  const fonts = await loadOgFonts(
    `${isPublic ? group.name : ""}${description ?? ""}`
  );

  return new ImageResponse(
    (
      <OgCardShell>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            marginTop: 48,
          }}
        >
          <div style={{ fontSize: 26, color: OG_TEXT_MUTED, letterSpacing: 2 }}>
            GROUP
          </div>
          <div
            style={{
              marginTop: 12,
              fontSize: 60,
              fontWeight: 700,
              color: OG_TEXT,
              lineHeight: 1.1,
            }}
          >
            {isPublic ? group.name : "Tokscale"}
          </div>
          {description ? (
            <div style={{ marginTop: 14, fontSize: 30, color: OG_TEXT_MUTED }}>
              {description}
            </div>
          ) : null}
        </div>

        <div style={{ display: "flex", flex: 1 }} />

        {typeof memberCount === "number" ? (
          <div style={{ display: "flex", gap: 20 }}>
            <OgStat
              label={memberCount === 1 ? "MEMBER" : "MEMBERS"}
              value={String(memberCount)}
            />
          </div>
        ) : (
          <div style={{ display: "flex", fontSize: 34, color: OG_TEXT_MUTED }}>
            AI token usage tracker and leaderboard
          </div>
        )}
      </OgCardShell>
    ),
    { ...size, fonts }
  );
}
