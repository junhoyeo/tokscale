import { renderToStaticMarkup } from "react-dom/server";
import { existsSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { INSTAGRAM_POSTS, InstagramSection } from "@/components/home/InstagramSection";
import { TOKSCALE_INSTAGRAM_URL } from "@/lib/community";

describe("home Instagram section", () => {
  it("links each post and the account, and every card image is vendored", () => {
    const markup = renderToStaticMarkup(<InstagramSection />);
    expect(markup).toContain(`href="${TOKSCALE_INSTAGRAM_URL}"`);
    expect(markup).toContain("Follow @tokscaleai");
    expect(INSTAGRAM_POSTS.length).toBeGreaterThan(0);
    for (const post of INSTAGRAM_POSTS) {
      expect(post.href).toMatch(/^https:\/\/www\.instagram\.com\/(p|reel)\/[\w-]+\/$/);
      expect(markup).toContain(`href="${post.href}"`);
      expect(existsSync(path.join(process.cwd(), "public", post.image)), post.image).toBe(true);
    }
  });
});
