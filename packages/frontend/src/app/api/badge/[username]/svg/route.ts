import { NextRequest, NextResponse } from "next/server";
import { getUserEmbedStats, type EmbedSortBy } from "@/lib/embed/getUserEmbedStats";
import {
  renderProfileBadgeSvg,
  renderBadgeErrorSvg,
  type BadgeMetric,
  type BadgeStyle,
} from "@/lib/embed/renderProfileBadgeSvg";
import { isValidGitHubUsername } from "@/lib/validation/username";

export const revalidate = 60;

const VALID_METRICS: BadgeMetric[] = ["tokens", "cost", "rank"];

function parseMetric(searchParams: URLSearchParams): BadgeMetric {
  const value = searchParams.get("metric");
  return VALID_METRICS.includes(value as BadgeMetric) ? (value as BadgeMetric) : "tokens";
}

function parseStyle(searchParams: URLSearchParams): BadgeStyle {
  return searchParams.get("style") === "flat-square" ? "flat-square" : "flat";
}

function parseSort(searchParams: URLSearchParams): EmbedSortBy {
  return searchParams.get("sort") === "cost" ? "cost" : "tokens";
}

function parseCompact(searchParams: URLSearchParams): boolean {
  const value = searchParams.get("compact");
  return value === "1" || value === "true";
}

function createSvgResponse(svg: string, init?: { status?: number; cacheControl?: string }) {
  return new NextResponse(svg, {
    status: init?.status ?? 200,
    headers: {
      "Content-Type": "image/svg+xml; charset=utf-8",
      "Cache-Control": init?.cacheControl ?? "public, max-age=0, s-maxage=60, stale-while-revalidate=300",
      "X-Content-Type-Options": "nosniff",
      "Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline';",
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

  const metric = parseMetric(searchParams);
  const style = parseStyle(searchParams);
  const sortBy = parseSort(searchParams);
  const compact = parseCompact(searchParams);
  const label = searchParams.get("label") ?? undefined;
  const color = searchParams.get("color") ?? undefined;

  if (!isValidGitHubUsername(username)) {
    const svg = renderBadgeErrorSvg("invalid username", { style, label });
    return createSvgResponse(svg, { status: 400, cacheControl: "no-store" });
  }

  try {
    const data = await getUserEmbedStats(username, sortBy);

    if (!data) {
      const svg = renderBadgeErrorSvg("not found", { style, label });
      return createSvgResponse(svg, { status: 200 });
    }

    const svg = renderProfileBadgeSvg(data, { metric, style, label, color, sort: sortBy, compact });

    console.info("[badge-svg] success", {
      username,
      durationMs: Date.now() - startedAt,
      metric,
      style,
      sortBy,
    });

    return createSvgResponse(svg);
  } catch (error) {
    console.error("[badge-svg] failed", {
      username,
      durationMs: Date.now() - startedAt,
      error: error instanceof Error ? error.message : "unknown_error",
    });

    const svg = renderBadgeErrorSvg("error", { style, label });
    return createSvgResponse(svg, { status: 500, cacheControl: "no-store" });
  }
}
