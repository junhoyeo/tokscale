"use client";

import Link from "next/link";
import styled from "styled-components";

export function ServiceFooter() {
  return (
    <Footer>
      <Inner>
        <Product>Tokscale</Product>
        <Links aria-label="Footer links">
          <Link href="/leaderboard">Leaderboard</Link>
          <Link href="/leaderboard?view=groups">Groups</Link>
          <a
            href="https://github.com/junhoyeo/tokscale"
            target="_blank"
            rel="noopener noreferrer"
          >
            Source
          </a>
        </Links>
      </Inner>
    </Footer>
  );
}

const Footer = styled.footer`
  width: 100%;
  border-top: 1px solid var(--service-border);
`;

const Inner = styled.div`
  width: 100%;
  max-width: 1500px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin: 0 auto;
  padding: 20px 32px;

  @media (max-width: 520px) {
    padding-right: 16px;
    padding-left: 16px;
  }
`;

const Product = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
`;

const Links = styled.nav`
  display: flex;
  align-items: center;
  gap: 16px;

  a {
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    text-decoration: none;
  }

  a:hover {
    color: var(--service-text);
  }

  a:focus-visible {
    border-radius: 4px;
    outline: 2px solid var(--service-focus);
    outline-offset: 3px;
  }
`;
