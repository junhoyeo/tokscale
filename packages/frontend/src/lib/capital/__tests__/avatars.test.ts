import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { OrgMark } from "@/app/(main)/capital/_components/OrgMark";
import { ledger } from "@/lib/capital/ledger";

const ORGS_DIR = path.join(process.cwd(), "public", "assets", "v2", "orgs");
const sources = JSON.parse(readFileSync(path.join(ORGS_DIR, "SOURCES.json"), "utf8")) as {
  orgs: Record<string, { name: string; source: string; page: string; image: string; fetched: string }>;
};
const orgs = [...ledger.content.orgs.values()];

describe("org avatars", () => {
  it("vendors every avatar and records where it came from", () => {
    const withAvatar = orgs.filter((org) => org.avatar);
    expect(withAvatar.length).toBeGreaterThan(40);
    for (const org of withAvatar) {
      expect(org.avatar).toBe(`/assets/v2/orgs/${org.slug}.webp`);
      expect(existsSync(path.join(process.cwd(), "public", org.avatar!)), org.slug).toBe(true);
      const source = sources.orgs[org.slug];
      expect(source, org.slug).toBeDefined();
      expect(source.page).toMatch(/^https:\/\/[a-z0-9.-]+\//);
      expect(source.image).toMatch(/^https:\/\//);
    }
    // No stray files or provenance entries without an org.
    const slugs = new Set(withAvatar.map((org) => org.slug));
    expect(Object.keys(sources.orgs).filter((slug) => !slugs.has(slug))).toEqual([]);
  });

  it("keeps the vendored lab glyph where one exists", () => {
    expect(orgs.filter((org) => org.logo && org.avatar).map((org) => org.slug)).toEqual([]);
  });

  it("renders the glyph, then the avatar, then the monogram", () => {
    const imageSrc = (el: ReturnType<typeof OrgMark>) => (el.props.children as { props: { src: string } }).props.src;
    const glyph = OrgMark({ org: { logo: "/assets/v2/models/openai.svg", avatar: "/assets/v2/orgs/x.webp", monogram: "O" } });
    expect(imageSrc(glyph)).toBe("/assets/v2/models/openai.svg");
    expect(glyph.props.className).not.toContain("markAvatar");
    const avatar = OrgMark({ org: { logo: null, avatar: "/assets/v2/orgs/sequoia-capital.webp", monogram: "SC" } });
    expect(imageSrc(avatar)).toBe("/assets/v2/orgs/sequoia-capital.webp");
    expect(avatar.props.className).toContain("markAvatar");
    expect(OrgMark({ org: { logo: null, avatar: null, monogram: "TM" } }).props.children).toBe("TM");
  });
});
