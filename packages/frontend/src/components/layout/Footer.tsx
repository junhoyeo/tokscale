"use client";

import Image from 'next/image';
import styled from 'styled-components';

const Container = styled.div`
  width: 100%;
  max-width: 1280px;
  margin-left: auto;
  margin-right: auto;
  padding-left: 24px;
  padding-right: 24px;

  @media (max-width: 560px) {
    padding-left: 12px;
    padding-right: 12px;
  }

  @media (max-width: 400px) {
    padding-left: 8px;
    padding-right: 8px;
  }
`;

const FooterElement = styled.footer`
  position: relative;
  width: 100%;
  height: 436px;
  border-radius: 20px;
  overflow: hidden;
  background: linear-gradient(to bottom, black, #10121C);

  @media (max-width: 560px) {
    height: 350px;
  }

  @media (max-width: 400px) {
    height: 320px;
  }
`;

const ContentContainer = styled.div`
  position: absolute;
  top: 52px;
  left: 60px;
  display: flex;
  flex-direction: column;
  gap: 21px;
  z-index: 10;

  @media (max-width: 560px) {
    top: 32px;
    left: 24px;
    gap: 16px;
  }

  @media (max-width: 400px) {
    top: 24px;
    left: 16px;
    gap: 12px;
  }
`;

const LogoContainer = styled.div`
  position: relative;
  width: 107.29px;
  height: 100px;

  @media (max-width: 560px) {
    width: 80px;
    height: 74.58px;
  }

  @media (max-width: 400px) {
    width: 64px;
    height: 59.66px;
  }
`;

const LogoImage = styled(Image)`
  object-fit: contain;
`;

const LogoLink = styled.a`
  display: block;
  opacity: 1;
  transition: opacity 0.2s ease-in-out;

  &:hover {
    opacity: 0.8;
  }
`;

const LogoSvg = styled(Image)`
  width: 184px;
  height: 21px;

  @media (max-width: 560px) {
    width: 138px;
    height: 15.75px;
  }

  @media (max-width: 400px) {
    width: 110px;
    height: 12.55px;
  }
`;

const Divider = styled.div`
  width: 74px;
  height: 2px;
  background: #0073FF;

  @media (max-width: 560px) {
    width: 56px;
  }

  @media (max-width: 400px) {
    width: 44px;
  }
`;

const TextContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: 4px;
`;

const CopyrightText = styled.p`
  color: #0073FF;
  font-size: 18px;
  font-weight: 500;
  line-height: 1.25;
  font-family: sans-serif;

  @media (max-width: 560px) {
    font-size: 14px;
  }
`;

const GitHubLink = styled.a`
  color: #3D526C;
  font-size: 18px;
  font-weight: 500;
  line-height: 1.25;
  font-family: sans-serif;
  transition: color 0.2s ease-in-out;

  &:hover {
    color: #0073FF;
  }

  @media (max-width: 560px) {
    font-size: 14px;
  }
`;

const GlobeContainer = styled.div`
  position: absolute;
  right: 0;
  top: -135px;
  pointer-events: none;
  user-select: none;

  @media (max-width: 560px) {
    top: -70px;
    right: -20px;
  }

  @media (max-width: 400px) {
    top: -50px;
    right: -30px;
  }
`;

const SpinningGlobe = styled(Image)`
  width: 435px;
  height: auto;
  animation: spin 60s linear infinite;

  @keyframes spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  @media (max-width: 560px) {
    width: 280px;
  }

  @media (max-width: 400px) {
    width: 220px;
  }
`;

export function Footer() {
  return (
    <Container>
      <FooterElement>
        <ContentContainer>
          <LogoContainer>
            <LogoImage
              src="/assets/footer-logo-icon.png"
              alt="Tokscale Icon"
              fill
            />
          </LogoContainer>

          <LogoLink 
            href="https://tokscale.ai" 
            target="_blank" 
            rel="noopener noreferrer"
          >
            <LogoSvg
              src="/assets/footer-logo.svg"
              alt="Tokscale"
              width={184}
              height={21}
            />
          </LogoLink>

          <Divider />

          <TextContainer>
            <CopyrightText>
              © 2025 Tokscale. All rights reserved.
            </CopyrightText>
            <GitHubLink 
              href="https://github.com/junhoyeo/tokscale" 
              target="_blank" 
              rel="noopener noreferrer"
            >
              github.com/junhoyeo/tokscale
            </GitHubLink>
          </TextContainer>
        </ContentContainer>

        <GlobeContainer>
          <SpinningGlobe
            src="/assets/footer-globe.svg"
            alt=""
            width={435}
            height={435}
            priority
          />
        </GlobeContainer>
      </FooterElement>
    </Container>
  );
}
