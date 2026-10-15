import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { FooterSection } from "../../src/components/landing/sections/FooterSection";

describe("landing footer", () => {
  it("links the privacy policy, terms and contact pages from the site root", () => {
    const markup = renderToStaticMarkup(<FooterSection />);
    const legalNav = markup.match(/<nav[^>]*aria-label="Legal"[^>]*>[\s\S]*?<\/nav>/)?.[0];

    expect(legalNav).toBeDefined();
    for (const href of ["/privacy", "/terms", "/contact"]) {
      expect(legalNav).toContain(`href="${href}"`);
    }
  });
});
