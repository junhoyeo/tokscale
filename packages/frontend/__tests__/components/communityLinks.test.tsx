import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { FooterSection } from "../../src/components/landing/sections/FooterSection";
import { Footer } from "../../src/components/layout/Footer";
import { ServiceFooter } from "../../src/components/layout/ServiceFooter";
import {
  TOKSCALE_DISCORD_URL,
  TOKSCALE_X_HANDLE,
  TOKSCALE_X_URL,
} from "../../src/lib/community";

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function anchorFor(markup: string, href: string): string | undefined {
  return markup.match(new RegExp(`<a[^>]*href="${escapeRegExp(href)}"[^>]*>`))?.[0];
}

describe("community links", () => {
  it("point at the Tokscale Discord server and X account", () => {
    expect(TOKSCALE_DISCORD_URL).toBe("https://discord.gg/h6DUGWdBbm");
    expect(TOKSCALE_X_URL).toBe("https://x.com/Tokscale");
    expect(TOKSCALE_X_HANDLE).toBe("@Tokscale");
  });

  it.each<[string, () => ReactElement]>([
    ["landing footer", () => <FooterSection />],
    ["service footer", () => <ServiceFooter />],
    ["site footer", () => <Footer />],
  ])("are linked from the %s in a new tab", (_name, render) => {
    const markup = renderToStaticMarkup(render());

    for (const href of [TOKSCALE_DISCORD_URL, TOKSCALE_X_URL]) {
      const anchor = anchorFor(markup, href);
      expect(anchor, href).toBeDefined();
      expect(anchor).toContain('target="_blank"');
      expect(anchor).toContain('rel="noopener noreferrer"');
    }
    expect(anchorFor(markup, TOKSCALE_X_URL)).toContain('aria-label="Tokscale on X"');
  });
});
