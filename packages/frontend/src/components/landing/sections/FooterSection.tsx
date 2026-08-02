"use client";

import styled from "styled-components";

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
            href="https://github.com/sponsors/junhoyeo"
            target="_blank"
            rel="noopener noreferrer"
          >
            Sponsor Tokscale
          </FooterLink>
        </FooterNav>
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

const FooterLink = styled.a`
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

const FooterCopyright = styled.p`
  font-family: "Wanted Sans", system-ui, -apple-system, sans-serif;
  font-weight: 600;
  font-size: 16px;
  line-height: 1.5em;
  letter-spacing: -0.0195em;
  text-transform: uppercase;
  color: #99a1af;
`;
