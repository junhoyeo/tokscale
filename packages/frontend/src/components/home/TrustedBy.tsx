"use client";

import Image from "next/image";
import styled from "styled-components";
import { Container } from "./primitives";
import { Reveal } from "./Reveal";

/** Companies where Tokscale users report working (community reports, not customer relationships). */
const LOGOS = [
  { name: "Microsoft", src: "/assets/v2/trusted/microsoft.svg", width: 105, height: 23, display: 21 },
  { name: "Amazon", src: "/assets/v2/trusted/amazon.svg", width: 73, height: 24, display: 22 },
  { name: "Google", src: "/assets/v2/trusted/google.svg", width: 76, height: 26, display: 23 },
  { name: "ByteDance", src: "/assets/v2/trusted/bytedance.svg", width: 251, height: 43.3, display: 20 },
  { name: "Tencent", src: "/assets/v2/trusted/tencent.png", width: 400, height: 54, display: 18 },
  { name: "Alibaba", src: "/assets/v2/trusted/alibaba.png", width: 902, height: 116, display: 18 },
  { name: "Intel", src: "/assets/v2/trusted/intel.svg", width: 24, height: 9.31, display: 18 },
  { name: "Toss", src: "/assets/v2/trusted/toss.png", width: 173, height: 52, display: 23 },
  { name: "Hashed", src: "/assets/v2/trusted/hashed.svg", width: 108, height: 28, display: 21 },
] as const;

/**
 * One centred line naming the companies, then the marks in one row at their
 * own heights. The soft grey keeps the logos subordinate to the page's ink.
 */
export function TrustedBy() {
  return (
    <Wrap aria-labelledby="trusted-by-title">
      <Container>
        <Reveal>
          <Label id="trusted-by-title">Trusted by Professionals at</Label>
        </Reveal>
        <Logos>
          {LOGOS.map((logo) => (
            <li key={logo.name}>
              <Image
                src={logo.src}
                alt={logo.name}
                height={logo.display}
                width={Math.round((logo.width * logo.display) / logo.height)}
                style={{ ["--logo-h" as string]: `${logo.display}px` }}
                unoptimized
              />
            </li>
          ))}
        </Logos>
      </Container>
    </Wrap>
  );
}

const Wrap = styled.section`
  margin-top: 48px;

  @media (max-width: 720px) {
    margin-top: 32px;
  }
`;

/* The section heading, kept to one centred line above the marks. */
const Label = styled.h2`
  margin: 0 0 18px;
  color: var(--v2-text-muted);
  font-size: 17px;
  font-weight: 500;
  line-height: 1.3;
  text-align: center;

  @media (max-width: 720px) {
    margin-bottom: 14px;
    font-size: 16px;
  }
`;

const Logos = styled.ul`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 18px 36px;
  margin: 0;
  padding: 0;
  list-style: none;

  img {
    display: block;
    width: auto;
    height: var(--logo-h);
    opacity: 0.85;
  }

  @media (max-width: 1180px) {
    column-gap: 28px;
  }

  /* Two balanced rows (5 + 4) between phones and small laptops. */
  @media (max-width: 1120px) and (min-width: 721px) {
    max-width: 640px;
    margin: 0 auto;
  }

  /* Phones: a 3 x 3 grid; a wide logo scales down to fit its cell. */
  @media (max-width: 720px) {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 22px 20px;
    justify-items: center;
    width: 100%;

    li {
      min-width: 0;
      max-width: 100%;
    }

    img {
      max-width: 100%;
      object-fit: contain;
    }
  }

  @media (max-width: 360px) {
    column-gap: 14px;

    img {
      height: calc(var(--logo-h) * 0.86);
    }
  }
`;
