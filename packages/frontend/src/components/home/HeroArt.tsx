"use client";

import styled from "styled-components";
import { DitherImage } from "dither-kit/DitherImage";

/**
 * The hero's right half: the Pillars of Creation, JWST's NIRCam photograph,
 * duotoned to the site's blue ramp and rendered through the kit's ordered
 * dither — a star-forming field where density is the picture, like every
 * chart on the site.
 *
 * Source: NASA, ESA, CSA, STScI (public domain), tinted and dithered here.
 */
export function HeroArt() {
  return (
    <Art>
      <DitherImage src="/assets/v2/hero/pillars-blue.webp" cell={5} focusY={0.42} aria-label="The Pillars of Creation, dithered in blue." />
    </Art>
  );
}

/* The art panel: the hero's right half. DitherImage's wrapper is the sizing
   box, stretched before the canvas paints its cells; under 980 it keeps a
   4/3 box above the card. */
const Art = styled.div`
  position: relative;
  display: block;
  overflow: hidden;
  min-height: 420px;
  border-radius: var(--radius, 2px);
  background: var(--v2-canvas);

  > div {
    position: absolute;
    inset: 0;
  }

  canvas {
    width: 100%;
    height: 100%;
  }

  @media (max-width: 980px) {
    order: 1;
    min-height: 0;
    aspect-ratio: 4 / 3;
  }
`;
