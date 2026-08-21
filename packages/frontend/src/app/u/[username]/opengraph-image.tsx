import { ImageResponse } from "next/og";
import {
  OG_BORDER,
  OG_SIZE,
  OG_TEXT,
  OG_TEXT_MUTED,
  OgCardShell,
  OgStat,
} from "@/components/og/OgCard";
import { getUserEmbedStats } from "@/lib/embed/getUserEmbedStats";
import { loadOgFonts } from "@/lib/og/fonts";
import { formatCompact } from "@/lib/format";

export const alt = "AI token usage on Tokscale";
export const size = OG_SIZE;
export const contentType = "image/png";

/**
 * Per-profile Open Graph card, rendered from the user's real numbers.
 *
 * Never throws: a missing user or an unreachable database falls through to a
 * generic card, because a 500 here would leave the profile with no preview
 * image at all wherever it gets shared.
 */
export default async function Image({
  params,
}: {
  params: Promise<{ username: string }>;
}) {
  const { username } = await params;

  const stats = await getUserEmbedStats(username).catch(() => null);
  const user = stats?.user;
  const displayName = user?.displayName?.trim();
  const handle = user?.username ?? username;
  const rank = stats?.stats.rank;

  // Only the dynamic strings need declaring: loadOgFonts adds the digits and
  // symbols the stat tiles use. A CJK display name is what pulls in Pretendard.
  const fonts = await loadOgFonts(`${handle}${displayName ?? ""}`);

  return new ImageResponse(
    (
      <OgCardShell>
        <div style={{ display: "flex", alignItems: "center", marginTop: 48 }}>
          {user?.avatarUrl ? (
            <img
              src={user.avatarUrl}
              width={132}
              height={132}
              alt=""
              style={{ borderRadius: 66, border: `1px solid ${OG_BORDER}` }}
            />
          ) : null}

          <div
            style={{
              display: "flex",
              flexDirection: "column",
              marginLeft: user?.avatarUrl ? 32 : 0,
            }}
          >
            {/* Single interpolation: `@{handle}` would be two text children,
                which Satori rejects on a node with no explicit display. */}
            <div style={{ fontSize: 60, fontWeight: 700, color: OG_TEXT, lineHeight: 1.1 }}>
              {`@${handle}`}
            </div>
            {displayName ? (
              <div style={{ marginTop: 8, fontSize: 30, color: OG_TEXT_MUTED }}>
                {displayName}
              </div>
            ) : null}
          </div>
        </div>

        <div style={{ display: "flex", flex: 1 }} />

        {stats ? (
          <div style={{ display: "flex", gap: 20 }}>
            <OgStat
              label="TOKENS"
              value={formatCompact(stats.stats.totalTokens, "number")}
            />
            <OgStat
              label="SPENT"
              value={formatCompact(stats.stats.totalCost, "currency")}
            />
            {typeof rank === "number" ? (
              <OgStat label="RANK" value={`#${rank}`} />
            ) : null}
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
