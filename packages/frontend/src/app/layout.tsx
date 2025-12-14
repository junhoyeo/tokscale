import type { Metadata } from "next";
import { JetBrains_Mono, Figtree } from "next/font/google";
import { Providers } from "@/lib/providers";
import "./globals.css";

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
  title: "Token Usage Leaderboard",
  description: "The Kardashev Scale for AI Devs",
  openGraph: {
    title: "Token Usage Leaderboard",
    description: "The Kardashev Scale for AI Devs",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Token Usage Leaderboard",
    description: "The Kardashev Scale for AI Devs",
  },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${figtree.variable} ${jetbrainsMono.variable}`}>
      <body className={`${figtree.className} antialiased`}>
        <Providers>
          {children}
        </Providers>
      </body>
    </html>
  );
}
