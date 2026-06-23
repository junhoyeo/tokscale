"use client";

import Image from "next/image";
import styled from "styled-components";

export function DescriptionSection() {
  return (
    <DescriptionSectionWrapper>
      <DescriptionText>
        A high-performance CLI tool
        <br />
        and visualization dashboard
        <br />
        for tracking token usage and costs
        <br />
        across multiple AI coding agents.
      </DescriptionText>

      <ClientLogosContainer>
        <ClientLogosFadeLeft />
        <Image
          src="/assets/landing/client-logos-grid.svg"
          alt="Supported AI coding clients"
          width={965}
          height={100}
          style={{ width: "100%", maxWidth: 965, height: "auto" }}
        />
        <ClientLogosFadeRight />
      </ClientLogosContainer>

      <GitHubBtn
        href="https://github.com/junhoyeo/tokscale"
        target="_blank"
        rel="noopener noreferrer"
      >
        <GitHubBtnText>GitHub</GitHubBtnText>
      </GitHubBtn>
    </DescriptionSectionWrapper>
  );
}

/* ── Description Section Styled Components ── */
const DescriptionSectionWrapper = styled.div`
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 48px;
  padding: 100px 32px 120px;
  background: linear-gradient(180deg, rgba(0, 0, 0, 1) 0%, rgba(1, 10, 21, 1) 50%);

  @media (max-width: 768px) {
    padding: 40px 20px 60px;
    gap: 32px;
  }
`;

const DescriptionText = styled.p`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 40px;
  line-height: 1.2em;
  letter-spacing: -0.03em;
  text-align: center;
  color: #b6c0d4;

  @media (max-width: 768px) {
    font-size: 28px;
  }

  @media (max-width: 480px) {
    font-size: 22px;
  }
`;

const ClientLogosContainer = styled.div`
  position: relative;
  width: 100%;
  max-width: 965px;
  display: flex;
  justify-content: center;
  overflow: hidden;
`;

const ClientLogosFadeLeft = styled.div`
  position: absolute;
  left: 0;
  top: 0;
  width: 324px;
  height: 100%;
  background: linear-gradient(90deg, rgba(1, 10, 21, 1) 0%, rgba(1, 10, 21, 0) 100%);
  z-index: 1;
  pointer-events: none;

  @media (max-width: 768px) {
    width: 120px;
  }
`;

const ClientLogosFadeRight = styled.div`
  position: absolute;
  right: 0;
  top: 0;
  width: 325px;
  height: 100%;
  background: linear-gradient(270deg, rgba(1, 10, 21, 1) 0%, rgba(1, 10, 21, 0) 100%);
  z-index: 1;
  pointer-events: none;

  @media (max-width: 768px) {
    width: 120px;
  }
`;

const GitHubBtn = styled.a`
  display: inline-flex;
  justify-content: center;
  align-items: center;
  gap: 4px;
  padding: 9px 28px;
  background: #ffffff;
  border-radius: 32px;
  text-decoration: none;
  transition: opacity 0.15s;

  &:hover {
    opacity: 0.9;
  }
`;

const GitHubBtnText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 23px;
  line-height: 1.2em;
  color: #000000;
`;
