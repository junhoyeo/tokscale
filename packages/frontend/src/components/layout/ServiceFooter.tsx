"use client";

import Link from "next/link";
import styled from "styled-components";
import { TOKSCALE_DISCORD_URL, TOKSCALE_X_URL } from "@/lib/community";

/** The compact close for app screens: one quiet row on the page ground. */
export function ServiceFooter() {
  return (
    <Footer>
      <Inner>
        <Product>Tokscale</Product>
        <Links aria-label="Footer">
          <Link href="/leaderboard">Leaderboards</Link>
          <Link href="/leaderboard?view=groups">Groups</Link>
          <Link href="/privacy">Privacy</Link>
          <Link href="/terms">Terms</Link>
          <Link href="/contact">Contact</Link>
          <a href="https://github.com/junhoyeo/tokscale" target="_blank" rel="noopener noreferrer">
            Source
          </a>
          <a href={TOKSCALE_DISCORD_URL} target="_blank" rel="noopener noreferrer">
            Discord
          </a>
          <a href={TOKSCALE_X_URL} target="_blank" rel="noopener noreferrer" aria-label="Tokscale on X">
            X
          </a>
        </Links>
      </Inner>
    </Footer>
  );
}

const Footer = styled.footer`
  width: 100%;
  background: var(--v2-canvas);
`;

const Inner = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px 32px;
  width: 100%;
  max-width: 1500px;
  margin: 0 auto;
  padding: 20px 32px 28px;

  @media (max-width: 720px) {
    flex-direction: column;
    align-items: flex-start;
    padding: 20px 16px 28px;
  }
`;

const Product = styled.span`
  color: var(--v2-text-muted);
  font-size: 13px;
  font-weight: 500;
`;

const Links = styled.nav`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: 0 18px;

  a {
    display: inline-flex;
    align-items: center;
    min-height: 32px;
    color: var(--v2-text-muted);
    font-size: 13px;
    text-decoration: none;
  }

  a:hover {
    color: var(--v2-text);
    text-decoration: underline;
    text-decoration-color: currentColor;
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
