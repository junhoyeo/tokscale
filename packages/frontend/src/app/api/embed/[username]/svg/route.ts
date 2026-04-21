import { NextRequest, NextResponse } from "next/server";
import { getUserEmbedStats, getUserEmbedContributions, type EmbedSortBy } from "@/lib/embed/getUserEmbedStats";
import {
  renderProfileEmbedErrorSvg,
  renderProfileEmbedSvg,
  type EmbedTheme,
} from "@/lib/embed/renderProfileEmbedSvg";
import {
  renderIsometric3DEmbedSvg,
  renderIsometric3DErrorSvg,
} from "@/lib/embed/renderIsometric3DSvg";
import { isValidGitHubUsername } from "@/lib/validation/username";

export const revalidate = 60;

function parseTheme(searchParams: URLSearchParams): EmbedTheme {
  return searchParams.get("theme") === "light" ? "light" : "dark";
}

function parseCompact(searchParams: URLSearchParams): boolean {
  const value = searchParams.get("compact");
  return value === "1" || value === "true";
}

function parseSort(searchParams: URLSearchParams): EmbedSortBy {
  const value = searchParams.get("sort");
  return value === "cost" ? "cost" : "tokens";
}

function parseGraph(searchParams: URLSearchParams): boolean {
  const value = searchParams.get("graph");
  return value === "1" || value === "true";
}

function parseView(searchParams: URLSearchParams): "2d" | "3d" {
  return searchParams.get("view") === "3d" ? "3d" : "2d";
}

function createSvgResponse(svg: string, init?: { status?: number; cacheControl?: string }) {
  return new NextResponse(svg, {
    status: init?.status ?? 200,
    headers: {
      "Content-Type": "image/svg+xml; charset=utf-8",
      "Cache-Control": init?.cacheControl ?? "public, max-age=0, s-maxage=60, stale-while-revalidate=300",
      "X-Content-Type-Options": "nosniff",
      "Content-Security-Policy": "default-src 'none'; img-src data:; style-src 'unsafe-inline';",
    },
  });
}

interface RouteParams {
  params: Promise<{ username: string }>;
}

export async function GET(request: NextRequest, { params }: RouteParams) {
  const startedAt = Date.now();
  const { username } = await params;
  const { searchParams } = new URL(request.url);

  const theme = parseTheme(searchParams);
  const compact = parseCompact(searchParams);
  const sortBy = parseSort(searchParams);
  const showGraph = parseGraph(searchParams);
  const view = parseView(searchParams);

  if (!isValidGitHubUsername(username)) {
    const svg = view === "3d"
      ? renderIsometric3DErrorSvg("Invalid username format", { theme })
      : renderProfileEmbedErrorSvg("Invalid username format", { theme, compact: true });
    return createSvgResponse(svg, { status: 400, cacheControl: "no-store" });
  }

  try {
    const data = await getUserEmbedStats(username, sortBy);

    if (!data) {
      const svg = view === "3d"
        ? renderIsometric3DErrorSvg(`User @${username} was not found`, { theme })
        : renderProfileEmbedErrorSvg(`User @${username} was not found`, { theme, compact });
      return createSvgResponse(svg, { status: 200 });
    }

    if (view === "3d") {
      const contributions = await getUserEmbedContributions(username).catch(() => null);

      if (!contributions) {
        const svg = renderIsometric3DErrorSvg("No contribution data available yet", { theme });
        return createSvgResponse(svg);
      }

      const svg = renderIsometric3DEmbedSvg(data, contributions, { theme, compact });

      console.info("[embed-svg-3d] success", {
        username,
        status: 200,
        durationMs: Date.now() - startedAt,
        sortBy,
        theme,
        compact,
      });

      return createSvgResponse(svg);
    }

    const contributions = showGraph && !compact
      ? await getUserEmbedContributions(username).catch(() => null)
      : null;

    const svg = renderProfileEmbedSvg(data, {
      theme,
      compact,
      compactNumbers: compact,
      sortBy,
      contributions,
    });

    console.info("[embed-svg] success", {
      username,
      status: 200,
      durationMs: Date.now() - startedAt,
      compact,
      sortBy,
      theme,
      graph: showGraph,
    });

    return createSvgResponse(svg);
  } catch (error) {
    console.error("[embed-svg] failed", {
      username,
      durationMs: Date.now() - startedAt,
      error: error instanceof Error ? error.message : "unknown_error",
    });

    const svg = view === "3d"
      ? renderIsometric3DErrorSvg("Tokscale stats are temporarily unavailable", { theme })
      : renderProfileEmbedErrorSvg("Tokscale stats are temporarily unavailable", { theme, compact });

    return createSvgResponse(svg, {
      status: 500,
      cacheControl: "no-store",
    });
  }
}
