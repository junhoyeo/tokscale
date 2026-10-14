"use client";

import Link from "next/link";
import styled, { css } from "styled-components";
import { TOKSCALE_DISCORD_URL, TOKSCALE_X_URL } from "@/lib/community";

export function FooterSection() {
  return (
    <LandingFooter>
      <FooterInner>
        <FooterNav aria-label="Footer links">
          <FooterLink
            href="https://github.com/junhoyeo/tokscale"
            target="_blank"
            rel="noopener noreferrer"
          >
            GitHub
          </FooterLink>
          <FooterLink
            href={TOKSCALE_DISCORD_URL}
            target="_blank"
            rel="noopener noreferrer"
          >
            Discord
          </FooterLink>
          <FooterLink
            href={TOKSCALE_X_URL}
            target="_blank"
            rel="noopener noreferrer"
            aria-label="Tokscale on X"
          >
            X
          </FooterLink>
          <FooterLink
            href="https://github.com/sponsors/junhoyeo"
            target="_blank"
            rel="noopener noreferrer"
          >
            Sponsor Tokscale
          </FooterLink>
        </FooterNav>
        {/* Ad networks check that the site root links its privacy policy. */}
        <FooterLegalNav aria-label="Legal">
          <FooterPageLink href="/privacy">Privacy</FooterPageLink>
          <FooterPageLink href="/terms">Terms</FooterPageLink>
          <FooterPageLink href="/contact">Contact</FooterPageLink>
        </FooterLegalNav>
        <FooterCopyright>© 2026 STROKE</FooterCopyright>
      </FooterInner>
    </LandingFooter>
  );
}

/* ── Footer Styled Components ── */
const LandingFooter = styled.footer`
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 48px;
  padding: 0 0 100px;

  @media (max-width: 768px) {
    padding: 0 0 60px;
  }
`;

const FooterInner = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
  padding-top: 16px;
  border-top: 1px solid rgba(255, 255, 255, 0.1);
`;

const FooterNav = styled.nav`
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 20px;

  @media (max-width: 480px) {
    flex-direction: column;
    gap: 4px;
  }
`;

const footerLinkStyles = css`
  display: inline-flex;
  align-items: center;
  min-height: 44px;
  font-family: "Wanted Sans", system-ui, -apple-system, sans-serif;
  font-weight: 400;
  font-size: 16px;
  line-height: 1.5em;
  color: #99a1af;
  text-decoration: none;

  &:hover {
    color: #ffffff;
  }

  &:focus-visible {
    outline: 2px solid #75b6ff;
    outline-offset: 3px;
  }

  @media (max-width: 480px) {
    min-height: 48px;
  }
`;

const FooterLink = styled.a`
  ${footerLinkStyles}
`;

const FooterPageLink = styled(Link)`
  ${footerLinkStyles}
`;

const FooterLegalNav = styled(FooterNav)`
  @media (max-width: 480px) {
    flex-direction: row;
    gap: 16px;
  }
`;

const FooterCopyright = styled.p`
  font-family: "Wanted Sans", system-ui, -apple-system, sans-serif;
  font-weight: 600;
  font-size: 16px;
  line-height: 1.5em;
  letter-spacing: -0.0195em;
  text-transform: uppercase;
  color: #99a1af;
`;
