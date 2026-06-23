"use client";

import styled from "styled-components";

export function FooterSection() {
  return (
    <LandingFooter>
      <FooterInner>
        <FooterCopyright>© 2026 STROKE</FooterCopyright>
      </FooterInner>
    </LandingFooter>
  );
}

/* ── Footer Styled Components ── */
const LandingFooter = styled.div`
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

const FooterCopyright = styled.p`
  font-family: "Wanted Sans", system-ui, -apple-system, sans-serif;
  font-weight: 600;
  font-size: 16px;
  line-height: 1.5em;
  letter-spacing: -0.0195em;
  text-transform: uppercase;
  color: #99a1af;
`;
