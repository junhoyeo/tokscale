import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PublicFooter, GITHUB_URL, SPONSOR_URL } from "../../src/components/layout/PublicFooter";
import { ServiceFooter } from "../../src/components/layout/ServiceFooter";
import { TOKSCALE_DISCORD_URL, TOKSCALE_INSTAGRAM_URL, TOKSCALE_X_URL } from "../../src/lib/community";

function anchorFor(markup: string, href: string): string | undefined {
  return markup.match(new RegExp(`<a[^>]*href="${href.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"[^>]*>`))?.[0];
}

describe("site footer", () => {
  const markup = renderToStaticMarkup(<PublicFooter />);

  it("closes the page with the community and source links in a new tab", () => {
    for (const href of [TOKSCALE_DISCORD_URL, TOKSCALE_X_URL, TOKSCALE_INSTAGRAM_URL, GITHUB_URL, SPONSOR_URL]) {
      const anchor = anchorFor(markup, href);
      expect(anchor, href).toBeDefined();
      expect(anchor).toContain('target="_blank"');
      expect(anchor).toContain('rel="noopener noreferrer"');
    }
    expect(anchorFor(markup, TOKSCALE_X_URL)).toContain('aria-label="Tokscale on X"');
    expect(markup).toContain("@tokscaleai");
  });

  it("keeps the legal links and the footer navigation landmark", () => {
    expect(markup).toContain('aria-label="Footer"');
    for (const path of ["/privacy", "/terms", "/contact"]) {
      expect(markup).toContain(`href="${path}"`);
    }
  });

  it("names its link columns", () => {
    for (const heading of ["Product", "Community", "Open source", "Legal"]) {
      expect(markup).toContain(`>${heading}</h2>`);
    }
  });
});

describe("service footer", () => {
  const markup = renderToStaticMarkup(<ServiceFooter />);

  it("keeps the compact links app screens close with", () => {
    expect(markup).toContain('aria-label="Footer"');
    expect(anchorFor(markup, TOKSCALE_DISCORD_URL)).toContain('rel="noopener noreferrer"');
    expect(anchorFor(markup, TOKSCALE_X_URL)).toContain('aria-label="Tokscale on X"');
    for (const path of ["/leaderboard", "/privacy", "/terms", "/contact"]) {
      expect(markup).toContain(`href="${path}"`);
    }
  });
});
