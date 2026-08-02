import { NextRequest, NextResponse } from "next/server";
import {
  getUserEmbedStats,
  getUserEmbedContributions,
  type EmbedSortBy,
} from "@/lib/embed/getUserEmbedStats";
import {
  renderProfileEmbedErrorSvg,
  renderProfileEmbedSvg,
} from "@/lib/embed/renderProfileEmbedSvg";
import { renderMinimalEmbedSvg } from "@/lib/embed/renderMinimalEmbedSvg";
import { renderTerminalEmbedSvg } from "@/lib/embed/renderTerminalEmbedSvg";
import { renderGraphEmbedSvg } from "@/lib/embed/renderGraphEmbedSvg";
import { renderOrbitEmbedSvg } from "@/lib/embed/renderOrbitEmbedSvg";
import { renderVitalsEmbedSvg } from "@/lib/embed/renderVitalsEmbedSvg";
import { renderBlueprintEmbedSvg } from "@/lib/embed/renderBlueprintEmbedSvg";
import { renderReceiptEmbedSvg } from "@/lib/embed/renderReceiptEmbedSvg";
import {
  type EmbedTheme,
  type EmbedTemplate,
  type EmbedColorName,
  type EmbedNumberFormat,
  parseEmbedTemplate,
  parseEmbedColor,
  parseNumberFormat,
  parseRankFormat,
} from "@/lib/embed/embedShared";
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

function createSvgResponse(
  svg: string,
  init?: { status?: number; cacheControl?: string },
) {
  return new NextResponse(svg, {
    status: init?.status ?? 200,
    headers: {
      "Content-Type": "image/svg+xml; charset=utf-8",
      "Cache-Control":
        init?.cacheControl ??
        "public, max-age=0, s-maxage=60, stale-while-revalidate=300",
      "X-Content-Type-Options": "nosniff",
      "Content-Security-Policy":
        "default-src 'none'; img-src data:; style-src 'unsafe-inline';",
    },
  });
}

interface RouteParams {
  params: Promise<{ username: string }>;
}

async function getContributionsWithEvidence(
  username: string,
  template: EmbedTemplate | "3d",
) {
  try {
    return await getUserEmbedContributions(username);
  } catch (error) {
    console.warn("[embed-svg] contribution fetch failed", {
      username,
      template,
      error: error instanceof Error ? error.message : "unknown_error",
    });
    return null;
  }
}

function requiresContributions(
  template: EmbedTemplate,
  showGraph: boolean,
  compact: boolean,
): boolean {
  if (
    template === "graph" ||
    template === "vitals" ||
    template === "blueprint"
  ) {
    return true;
  }
  return showGraph && !(template === "classic" && compact);
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
  const template: EmbedTemplate = parseEmbedTemplate(
    searchParams.get("template"),
  );
  const color: EmbedColorName | null = parseEmbedColor(
    searchParams.get("color"),
  );
  const tokensFormat: EmbedNumberFormat | undefined = parseNumberFormat(
    searchParams.get("tokens"),
  );
  const costFormat: EmbedNumberFormat | undefined = parseNumberFormat(
    searchParams.get("cost"),
  );
  const rankFormat = parseRankFormat(searchParams.get("rank"));

  if (!isValidGitHubUsername(username)) {
    const svg =
      view === "3d"
        ? renderIsometric3DErrorSvg("Invalid username format", { theme })
        : renderProfileEmbedErrorSvg("Invalid username format", {
            theme,
            color,
            compact: true,
          });
    return createSvgResponse(svg, { status: 400, cacheControl: "no-store" });
  }

  try {
    const data = await getUserEmbedStats(username, sortBy);

    if (!data) {
      const svg =
        view === "3d"
          ? renderIsometric3DErrorSvg(`User @${username} was not found`, {
              theme,
            })
          : renderProfileEmbedErrorSvg(`User @${username} was not found`, {
              theme,
              color,
              compact,
            });
      return createSvgResponse(svg, { status: 200 });
    }

    if (data.user.username !== username) {
      const redirectUrl = new URL(request.url);
      redirectUrl.pathname = `/api/embed/${data.user.username}/svg`;
      return NextResponse.redirect(redirectUrl, 308);
    }

    if (view === "3d") {
      const contributions = await getContributionsWithEvidence(username, "3d");

      if (!contributions) {
        const svg = renderIsometric3DErrorSvg(
          "Contribution history is temporarily unavailable",
          { theme },
        );
        return createSvgResponse(svg, {
          status: 503,
          cacheControl: "no-store",
        });
      }

      const svg = renderIsometric3DEmbedSvg(data, contributions, {
        theme,
        compact,
      });

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

    const contributionDataRequired = requiresContributions(
      template,
      showGraph,
      compact,
    );
    const wantsContributions =
      contributionDataRequired ||
      template === "orbit" ||
      template === "receipt";
    const contributions = wantsContributions
      ? await getContributionsWithEvidence(username, template)
      : null;

    if (contributionDataRequired && !contributions) {
      const svg = renderProfileEmbedErrorSvg(
        "Contribution history is temporarily unavailable",
        { theme, color, compact: true },
      );
      return createSvgResponse(svg, {
        status: 503,
        cacheControl: "no-store",
      });
    }

    const common = {
      theme,
      color,
      sortBy,
      tokensFormat,
      costFormat,
      rankFormat,
      contributions,
    };
    let svg: string;
    switch (template) {
      case "minimal":
        svg = renderMinimalEmbedSvg(data, { ...common, graph: showGraph });
        break;
      case "terminal":
        svg = renderTerminalEmbedSvg(data, { ...common, graph: showGraph });
        break;
      case "graph":
        svg = renderGraphEmbedSvg(data, common);
        break;
      case "orbit":
        svg = renderOrbitEmbedSvg(data, { ...common, graph: showGraph });
        break;
      case "vitals":
        svg = renderVitalsEmbedSvg(data, common);
        break;
      case "blueprint":
        svg = renderBlueprintEmbedSvg(data, { ...common, graph: showGraph });
        break;
      case "receipt":
        svg = renderReceiptEmbedSvg(data, { ...common, graph: showGraph });
        break;
      default:
        svg = renderProfileEmbedSvg(data, {
          ...common,
          compact,
          compactNumbers: compact,
        });
        break;
    }

    console.info("[embed-svg] success", {
      username,
      status: 200,
      durationMs: Date.now() - startedAt,
      template,
      color,
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

    const svg =
      view === "3d"
        ? renderIsometric3DErrorSvg(
            "Tokscale stats are temporarily unavailable",
            { theme },
          )
        : renderProfileEmbedErrorSvg(
            "Tokscale stats are temporarily unavailable",
            { theme, color, compact },
          );

    return createSvgResponse(svg, {
      status: 500,
      cacheControl: "no-store",
    });
  }
}
