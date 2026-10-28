"use client";

import Link from "next/link";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";
import { DitherGradient } from "dither-kit/DitherGradient";
import {
  TOKSCALE_DISCORD_URL,
  TOKSCALE_INSTAGRAM_URL,
  TOKSCALE_X_URL,
} from "@/lib/community";
import { TokscaleMark, TokscaleWordmark } from "@/components/layout/TokscaleMark";

export const GITHUB_URL = "https://github.com/junhoyeo/tokscale";
export const SPONSOR_URL = "https://github.com/sponsors/junhoyeo";

interface FooterLink {
  label: string;
  href: string;
  external?: boolean;
  ariaLabel?: string;
}

const COLUMNS: { heading: string; links: FooterLink[] }[] = [
  {
    heading: "Product",
    links: [
      { label: "Leaderboards", href: "/leaderboard" },
      { label: "Models", href: "/models" },
      { label: "Capital", href: "/capital" },
      { label: "Tracker", href: "/tracker" },
    ],
  },
  {
    heading: "Community",
    links: [
      { label: "Discord", href: TOKSCALE_DISCORD_URL, external: true },
      { label: "X", href: TOKSCALE_X_URL, external: true, ariaLabel: "Tokscale on X" },
      { label: "Instagram", href: TOKSCALE_INSTAGRAM_URL, external: true, ariaLabel: "Tokscale on Instagram, @tokscaleai" },
    ],
  },
  {
    heading: "Open source",
    links: [
      { label: "Source", href: GITHUB_URL, external: true },
      { label: "Sponsor", href: SPONSOR_URL, external: true },
    ],
  },
  {
    heading: "Legal",
    links: [
      { label: "Privacy", href: "/privacy" },
      { label: "Terms", href: "/terms" },
      { label: "Contact", href: "/contact" },
    ],
  },
];

/**
 * The site's close. The brand, one sentence and the one action on the left;
 * the link columns on the right; a quiet © row; then a dithered horizon
 * (dither-kit's DitherGradient) rising from the bottom edge in the accent, the
 * page's motif at its largest and calmest.
 */
export function PublicFooter() {
  const currentYear = new Date().getFullYear();

  return (
    <Footer>
      <Inner>
        <Top>
          <Brand>
            <BrandLink href="/" aria-label="Tokscale home">
              <TokscaleMark height={22} />
              <TokscaleWordmark height={17} />
            </BrandLink>
            <Statement>
              Every figure on the leaderboard is public and comes from the people on it.
            </Statement>
            <Actions>
              <StartTracking
                color="blue"
                variant="solid"
                nativeButton={false}
                role="link"
                render={<Link href="/tracker" />}
              >
                Start tracking
              </StartTracking>
              <Handle href={TOKSCALE_INSTAGRAM_URL} target="_blank" rel="noopener noreferrer">
                @tokscaleai
              </Handle>
            </Actions>
          </Brand>

          <Nav aria-label="Footer">
            {COLUMNS.map((column) => (
              <Column key={column.heading}>
                <ColumnHeading>{column.heading}</ColumnHeading>
                <ColumnList>
                  {column.links.map((link) => (
                    <li key={link.label}>
                      {link.external ? (
                        <a
                          href={link.href}
                          target="_blank"
                          rel="noopener noreferrer"
                          aria-label={link.ariaLabel}
                        >
                          {link.label}
                        </a>
                      ) : (
                        <Link href={link.href}>{link.label}</Link>
                      )}
                    </li>
                  ))}
                </ColumnList>
              </Column>
            ))}
          </Nav>
        </Top>

        <Bottom>
          <Fine>
            © <time dateTime={String(currentYear)} suppressHydrationWarning>{currentYear}</time> Tokscale
          </Fine>
          <Fine>Usage data is submitted by the people it describes.</Fine>
        </Bottom>
      </Inner>

      <Horizon aria-hidden="true">
        <DitherGradient from="blue" to="transparent" direction="up" cell={4} renderMode="static" className="horizon" />
      </Horizon>
    </Footer>
  );
}

const Footer = styled.footer`
  width: 100%;
  margin-top: 96px;
  /* A tone step, not a rule: the close is its own surface on the canvas. */
  background: var(--v2-surface);

  @media (max-width: 720px) {
    margin-top: 64px;
  }
`;

const Inner = styled.div`
  width: 100%;
  max-width: 1500px;
  margin: 0 auto;
  padding: 56px 32px 0;

  @media (max-width: 720px) {
    padding: 40px 16px 0;
  }
`;

/* Brand block in the first five of twelve columns, links from the seventh:
   both edges of the content width are closed above 900px. */
const Top = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  gap: 48px;

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 40px;
  }
`;

const Brand = styled.div`
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 20px;
`;

const BrandLink = styled(Link)`
  display: inline-flex;
  align-items: flex-end;
  gap: 9px;
  color: var(--v2-text);

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 4px;
  }
`;

const Statement = styled.p`
  max-width: 34ch;
  margin: 0;
  color: var(--v2-text);
  font-size: 20px;
  font-weight: 500;
  letter-spacing: -0.01em;
  line-height: 1.4;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px 20px;
`;

const StartTracking = styled(DitherButton)`
  min-height: 40px;
  color: #fff;
  font-family: var(--v2-font-ui);
  font-size: 14px;
  font-weight: 500;

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }
`;

const Handle = styled.a`
  color: var(--v2-text-muted);
  font-size: 14px;
  font-weight: 500;
  text-decoration: none;

  &:hover {
    color: var(--v2-text);
    text-decoration: underline;
    text-decoration-color: var(--v2-accent);
    text-underline-offset: 4px;
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 3px;
  }
`;

const Nav = styled.nav`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 32px 24px;

  @media (max-width: 560px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
`;

const Column = styled.div`
  display: flex;
  flex-direction: column;
  gap: 14px;
`;

const ColumnHeading = styled.h2`
  margin: 0;
  color: var(--v2-text);
  font-size: 14px;
  font-weight: 600;
  line-height: 1.4;
`;

const ColumnList = styled.ul`
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 0;
  padding: 0;
  list-style: none;

  a {
    color: var(--v2-text-muted);
    font-size: 14px;
    line-height: 1.4;
    text-decoration: none;
  }

  a:hover {
    color: var(--v2-text);
  }

  a:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 3px;
  }

  @media (pointer: coarse) {
    gap: 0;

    a {
      display: inline-flex;
      align-items: center;
      min-height: 44px;
    }
  }
`;

const Bottom = styled.div`
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 8px 24px;
  margin-top: 56px;
  padding-top: 20px;

  @media (max-width: 720px) {
    margin-top: 40px;
  }
`;

const Fine = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
`;

/* The dithered horizon: full bleed, accent at the bottom edge thinning to
   nothing, so the page ends in its own pixels rather than in a hard edge. */
const Horizon = styled.div`
  position: relative;
  height: 140px;
  margin-top: 32px;
  pointer-events: none;

  .horizon {
    position: absolute;
    inset: 0;
  }

  @media (max-width: 720px) {
    height: 96px;
    margin-top: 24px;
  }
`;
