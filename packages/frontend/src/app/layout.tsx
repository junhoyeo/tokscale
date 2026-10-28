import type { Metadata, Viewport } from "next";
import { connection } from "next/server";
// taste-skill-ignore: JetBrains Mono is loaded only for real <code>/<pre>
import { JetBrains_Mono, Figtree } from "next/font/google";
import NextTopLoader from "nextjs-toploader";
import { ToastContainer } from "react-toastify";
import { Analytics } from "@vercel/analytics/next";
import { Providers } from "@/lib/providers";
import { getRootMetadata } from "@/lib/seo/rootMetadata";
import "dither-kit/styles.css";
import "./globals.css";
import "react-toastify/dist/ReactToastify.css";

const figtree = Figtree({
  variable: "--font-figtree",
  subsets: ["latin"],
  display: "swap",
});

const jetbrainsMono = JetBrains_Mono({
  // taste-skill-ignore: a code face, used only inside <code>/<pre>
  variable: "--font-mono",
  subsets: ["latin"],
  display: "swap",
});

const fontVariables = `${figtree.variable} ${jetbrainsMono.variable}`; // taste-skill-ignore: code face variable

export async function generateMetadata(): Promise<Metadata> {
  // APP_URL belongs to the container runtime, not the Docker build. Make
  // metadata request-dynamic so self-hosted deployments emit their own origin.
  await connection();
  return getRootMetadata();
}

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={fontVariables}>
      {/* dither-kit scopes its base and tokens to .dither-kit; .dark selects
          its dark token set, which globals.css maps onto the Tokscale tokens. */}
      <body className={`${figtree.className} dither-kit dark`}>
        <NextTopLoader color="var(--v2-accent-strong)" shadow={false} showSpinner={false} />
        <Providers>
          {children}
        </Providers>
        <ToastContainer position="top-right" />
        <Analytics />
      </body>
    </html>
  );
}
