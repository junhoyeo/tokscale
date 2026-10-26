"use client";

import Image from "next/image";
import Link from "next/link";
import styled from "styled-components";
import { TOKSCALE_DISCORD_URL, TOKSCALE_INSTAGRAM_URL, TOKSCALE_X_URL } from "@/lib/community";

export const GITHUB_URL = "https://github.com/junhoyeo/tokscale";
export const SPONSOR_URL = "https://github.com/sponsors/junhoyeo";

/** Compact v2 footer for public pages. */
export function PublicFooter() {
  return (
    <Footer>
      <Inner>
        <Brand>
          <Link href="/" aria-label="Tokscale home">
            <Image src="/assets/hero-logo.svg" alt="" width={84} height={16} />
          </Link>
        </Brand>
        <Links aria-label="Footer">
          <a href={GITHUB_URL} target="_blank" rel="noopener noreferrer">
            GitHub
          </a>
          <a href={TOKSCALE_DISCORD_URL} target="_blank" rel="noopener noreferrer">
            Discord
          </a>
          <a href={TOKSCALE_X_URL} target="_blank" rel="noopener noreferrer" aria-label="Tokscale on X">
            X
          </a>
          <a href={TOKSCALE_INSTAGRAM_URL} target="_blank" rel="noopener noreferrer" aria-label="Tokscale on Instagram">
            Instagram
          </a>
          <a href={SPONSOR_URL} target="_blank" rel="noopener noreferrer">
            Sponsor
          </a>
          <Link href="/privacy">Privacy</Link>
          <Link href="/terms">Terms</Link>
          <Link href="/contact">Contact</Link>
        </Links>
      </Inner>
    </Footer>
  );
}

const Footer = styled.footer`
  width: 100%;
  border-top: 1px solid var(--v2-rule);
  background: var(--v2-canvas);
`;

const Inner = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 16px 32px;
  width: 100%;
  max-width: 1200px;
  margin: 0 auto;
  padding: 24px 32px 32px;

  @media (max-width: 520px) {
    flex-direction: column;
    align-items: flex-start;
    padding: 24px 16px 32px;
  }
`;

const Brand = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 16px;
  color: var(--v2-text-subtle);
  font-size: 13px;

  a {
    display: flex;
    border-radius: 4px;
  }

  a:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 3px;
  }
`;

const Links = styled.nav`
  display: flex;
  flex-wrap: wrap;
  gap: 4px 20px;

  a {
    display: inline-flex;
    align-items: center;
    min-height: 32px;
    border-radius: 4px;
    color: var(--v2-text-muted);
    font-size: 14px;
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
    a {
      min-height: 44px;
    }
  }
`;
