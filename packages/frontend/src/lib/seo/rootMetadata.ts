import type { Metadata } from "next";
import { getPublicOrigin } from "./urls";

export function getRootMetadata(origin = getPublicOrigin()): Metadata {
  const ogImage = `${origin}/og-image.png`;

  return {
    metadataBase: new URL(origin),
    title: "Tokscale - AI Token Usage Tracker & Leaderboard",
    description: "Track, visualize, and compete on AI coding assistant token usage across Claude Code, Cursor, OpenCode, Codex, Gemini, Kimi, and Qwen. The Kardashev Scale for AI Devs.",
    icons: {
      icon: [
        { url: "/favicon-16x16.png", sizes: "16x16", type: "image/png" },
        { url: "/favicon-32x32.png", sizes: "32x32", type: "image/png" },
      ],
      apple: "/apple-icon.png",
    },
    manifest: "/site.webmanifest",
    openGraph: {
      title: "Tokscale - AI Token Usage Tracker & Leaderboard",
      description: "Track, visualize, and compete on AI coding assistant token usage across Claude Code, Cursor, OpenCode, Codex, Gemini, Kimi, and Qwen. The Kardashev Scale for AI Devs.",
      type: "website",
      url: origin,
      siteName: "Tokscale",
      images: [{ url: ogImage, width: 1200, height: 630, alt: "Tokscale - AI Token Usage Tracker" }],
    },
    twitter: {
      card: "summary_large_image",
      title: "Tokscale - AI Token Usage Tracker & Leaderboard",
      description: "Track, visualize, and compete on AI coding assistant token usage across Claude Code, Cursor, OpenCode, Codex, Gemini, Kimi, and Qwen.",
      images: [ogImage],
    },
  };
}
