/**
 * Font loading for the Open Graph cards.
 *
 * Three constraints drive the shape of this:
 *
 * 1. Satori reads ttf, otf and woff only — NOT woff2. The Manrope files already
 *    in the repo family are woff2, so they cannot be reused here.
 * 2. Manrope is on Google Fonts, which serves a `format('truetype')` subset
 *    when the request carries a `text=` parameter. A realistic card's glyph set
 *    comes back at roughly 14 KB, versus a few hundred for the whole face.
 * 3. Pretendard is not on Google Fonts and has no subsetting endpoint, so it
 *    arrives whole at ~1.6 MB per weight. It is therefore fetched only when the
 *    card actually contains CJK text, which for usernames is the exception.
 *
 * Nothing here throws. Every failure path returns fewer fonts, and an empty
 * array simply means Satori falls back to its built-in face — a plainer card
 * rather than no card, since a 500 leaves the page with no preview at all.
 */

export interface OgFont {
  name: string;
  data: ArrayBuffer;
  weight: 400 | 700;
  style: "normal";
}

/** Font stack for the card root. Satori resolves per glyph, in this order. */
export const OG_FONT_FAMILY = "Manrope, Pretendard";

/**
 * Hangul, kana, CJK ideographs, and CJK punctuation.
 *
 * Deliberately broad: a false positive costs one extra font fetch on a cached
 * image, while a false negative renders the name as tofu.
 */
const CJK_PATTERN =
  /[ᄀ-ᇿ　-〿぀-ヿ㄰-㆏㐀-䶿一-鿿ꥠ-꥿가-힯豈-﫿＀-￯]/;

export function hasCjk(text: string): boolean {
  return CJK_PATTERN.test(text);
}

/**
 * Always requested alongside the card's own text so that a card whose dynamic
 * content is empty or unusual still has digits and the symbols the stat tiles
 * are built from.
 */
const BASELINE_GLYPHS = "0123456789.,$#@%+-/ ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

// Pinned rather than floating: an unpinned CDN specifier would let a remote
// publish change what gets rendered into every social preview.
const PRETENDARD_VERSION = "1.3.9";

const PRETENDARD_FILES: Record<400 | 700, string> = {
  400: "Pretendard-Regular.otf",
  700: "Pretendard-Bold.otf",
};

// Font binaries are immutable, so they are safe to cache indefinitely.
const FETCH_OPTIONS: RequestInit = { cache: "force-cache" };

async function fetchArrayBuffer(url: string): Promise<ArrayBuffer | null> {
  try {
    const response = await fetch(url, FETCH_OPTIONS);
    if (!response.ok) return null;
    return await response.arrayBuffer();
  } catch {
    return null;
  }
}

async function loadManrope(
  text: string,
  weight: 400 | 700
): Promise<OgFont | null> {
  try {
    const query = new URLSearchParams({
      family: `Manrope:wght@${weight}`,
      text: `${BASELINE_GLYPHS}${text}`,
    });

    const cssResponse = await fetch(
      `https://fonts.googleapis.com/css2?${query}`,
      FETCH_OPTIONS
    );
    if (!cssResponse.ok) return null;

    // Google only returns a truetype src because of the `text` parameter; a
    // plain request would yield woff2, which Satori cannot parse.
    const css = await cssResponse.text();
    const url = css.match(/src:\s*url\((https:\/\/[^)]+)\)\s*format\('truetype'\)/)?.[1];
    if (!url) return null;

    const data = await fetchArrayBuffer(url);
    return data ? { name: "Manrope", data, weight, style: "normal" } : null;
  } catch {
    return null;
  }
}

async function loadPretendard(weight: 400 | 700): Promise<OgFont | null> {
  const data = await fetchArrayBuffer(
    `https://cdn.jsdelivr.net/npm/pretendard@${PRETENDARD_VERSION}/dist/public/static/${PRETENDARD_FILES[weight]}`
  );
  return data ? { name: "Pretendard", data, weight, style: "normal" } : null;
}

/**
 * `text` should be every string that will appear on the card: it decides both
 * the Manrope subset requested and whether Pretendard is needed at all.
 */
export async function loadOgFonts(text: string): Promise<OgFont[]> {
  const requests: Promise<OgFont | null>[] = [
    loadManrope(text, 400),
    loadManrope(text, 700),
  ];

  if (hasCjk(text)) {
    requests.push(loadPretendard(400), loadPretendard(700));
  }

  const fonts = await Promise.all(requests);
  return fonts.filter((font): font is OgFont => font !== null);
}
