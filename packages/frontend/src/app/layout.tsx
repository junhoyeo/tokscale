import type { Metadata } from "next";
import { JetBrains_Mono, Figtree } from "next/font/google";
import NextTopLoader from "nextjs-toploader";
import { ToastContainer } from "react-toastify";
import { Analytics } from "@vercel/analytics/next";
import { Providers } from "@/lib/providers";
import "./globals.css";
import "react-toastify/dist/ReactToastify.css";

const figtree = Figtree({
  variable: "--font-figtree",
  subsets: ["latin"],
  display: "swap",
});

const jetbrainsMono = JetBrains_Mono({
  variable: "--font-mono",
  subsets: ["latin"],
  display: "swap",
});

export const metadata: Metadata = {
  title: "Tokscale - AI Token Usage Tracker & Leaderboard",
  description: "Track, visualize, and compete on AI coding assistant token usage across Claude Code, Cursor, OpenCode, Codex, and Gemini. The Kardashev Scale for AI Devs.",
  metadataBase: new URL("https://tokscale.ai"),
  openGraph: {
    title: "Tokscale - AI Token Usage Tracker & Leaderboard",
    description: "Track, visualize, and compete on AI coding assistant token usage across Claude Code, Cursor, OpenCode, Codex, and Gemini. The Kardashev Scale for AI Devs.",
    type: "website",
    url: "https://tokscale.ai",
    siteName: "Tokscale",
    images: [
      {
        url: "https://tokscale.ai/og-image.png",
        width: 1200,
        height: 630,
        alt: "Tokscale - AI Token Usage Tracker",
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: "Tokscale - AI Token Usage Tracker & Leaderboard",
    description: "Track, visualize, and compete on AI coding assistant token usage across Claude Code, Cursor, OpenCode, Codex, and Gemini.",
    images: ["https://tokscale.ai/og-image.png"],
  },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${figtree.variable} ${jetbrainsMono.variable}`}>
      <body className={figtree.className}>
        <NextTopLoader color="#3B82F6" showSpinner={false} />
        <Providers>
          {children}
        </Providers>
        <ToastContainer position="top-right" />
        <Analytics />
      </body>
    </html>
  );
}
