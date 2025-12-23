"use client";

import { useState } from "react";
import Image from "next/image";
import Link from "next/link";
import styled from "styled-components";
import heroBg from "@/../public/assets/hero-bg.png";

export function BlackholeHero() {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText("bunx tokscale");
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <HeroContainer>
      <BackgroundWrapper>
        <HeroBgImage
          src={heroBg}
          alt=""
          width={3000}
          height={940}
          priority
          quality={100}
          unoptimized
          placeholder="blur"
          style={{ width: "100%", height: "100%", backgroundColor: "black" }}
        />
      </BackgroundWrapper>

      <ContentWrapper>
        <LogoWrapper>
          <LogoImage
            src="/assets/hero-logo.svg"
            alt="Tokscale Logo"
            fill
          />
        </LogoWrapper>

        <HeroTitle>
          The Kardashev Scale
          <br />
          for AI Devs
        </HeroTitle>

        <CommandCard>
          <CopyButton onClick={handleCopy}>
            <CopyButtonText>
              {copied ? "Copied" : "Copy"}
            </CopyButtonText>
          </CopyButton>
          
          <CommandDisplay>
            <CommandTextWrapper>
              <CommandPrefix>
                bunx&nbsp;
              </CommandPrefix>
              <CommandName>
                tokscale
              </CommandName>
            </CommandTextWrapper>
            <GradientSeparator />
          </CommandDisplay>
        </CommandCard>

        <FooterContainer>
          <StarContainer>
            <StyledStarIcon 
              src="/assets/github-icon.svg" 
              alt="GitHub Star" 
              width={24} 
              height={24}
            />
            <StarText>
              Star me on GitHub!
            </StarText>
          </StarContainer>
          <GitHubLink
            href="https://github.com/junhoyeo/tokscale"
            target="_blank"
            rel="noopener noreferrer"
          >
            junhoyeo/tokscale
          </GitHubLink>
        </FooterContainer>
      </ContentWrapper>
    </HeroContainer>
  );
}

const HeroContainer = styled.div`
  position: relative;
  width: 100%;
  max-width: 80rem;
  margin-left: auto;
  margin-right: auto;
  margin-bottom: 2.5rem;
  overflow: hidden;
  height: 470px;
  z-index: 0;
  border-radius: 0px 0px 20px 20px;
  border-bottom: 1px solid rgba(105, 105, 105, 0.4);
  border-left: 1px solid rgba(105, 105, 105, 0.4);
  border-right: 1px solid rgba(105, 105, 105, 0.4);
`;

const BackgroundWrapper = styled.div`
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  left: 0;
  z-index: 0;
  background-color: black;
`;

const HeroBgImage = styled(Image)`
  object-fit: cover;
`;

const ContentWrapper = styled.div`
  position: relative;
  z-index: 10;
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding-top: 90px;
  gap: 39px;
`;

const LogoWrapper = styled.div`
  position: relative;
  width: 173px;
  height: 36px;
  flex-shrink: 0;
`;

const LogoImage = styled(Image)`
  object-fit: contain;
`;

const HeroTitle = styled.h1`
  font-size: 48px;
  font-weight: 700;
  color: white;
  text-align: center;
  font-family: var(--font-figtree), "Figtree", sans-serif;
  line-height: 0.94em;
  letter-spacing: -0.05em;
  text-shadow: 0px 6px 12px 0px rgba(0, 30, 66, 0.6);
`;

const CommandCard = styled.div`
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px;
  border-radius: 0.75rem;
  border-width: 1px;
  border-style: solid;
  backdrop-filter: blur(4px);
  background-color: #10121C;
  border-color: rgba(49, 56, 65, 0.4);
`;

const CopyButton = styled.button`
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 0.5rem;
  transition: all 150ms;
  flex-shrink: 0;
  background-color: #0073FF;
  height: 36px;
  width: 86px;
  cursor: pointer;
  border: none;
  
  &:hover {
    opacity: 0.9;
  }
  
  &:active {
    transform: scale(0.95);
  }
`;

const CopyButtonText = styled.span`
  font-size: 15px;
  font-weight: 700;
  color: white;
  line-height: 1;
  letter-spacing: -0.025em;
`;

const CommandDisplay = styled.div`
  display: flex;
  align-items: center;
  position: relative;
  overflow: hidden;
  height: 36px;
  background-color: #1A1B1C;
  border-radius: 0.5rem;
  flex-shrink: 0;
  padding-left: 0.75rem;
  padding-right: 0.75rem;
  width: 190px;
`;

const CommandTextWrapper = styled.div`
  z-index: 10;
  display: flex;
  align-items: center;
`;

const CommandPrefix = styled.span`
  color: #FFF;
  font-family: "Inconsolata", monospace;
  font-size: 16px;
  font-weight: 500;
  line-height: 94%;
  letter-spacing: -0.8px;
`;

const CommandName = styled.span`
  background: linear-gradient(90deg, #0CF 0%, #0073FF 100%);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  font-family: "Inconsolata", monospace;
  font-size: 16px;
  font-weight: 500;
  line-height: 94%;
  letter-spacing: -0.8px;
`;

const GradientSeparator = styled.div`
  margin-left: 10px;
  flex-shrink: 0;
  width: 25px;
  height: 36px;
  background: linear-gradient(270deg, rgba(26, 27, 28, 0) 0%, rgba(1, 127, 255, 0.14) 50%, rgba(26, 27, 28, 0) 100%);
`;

const FooterContainer = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
`;

const StarContainer = styled.div`
  display: flex;
  align-items: center;
  gap: 6px;
`;

const StyledStarIcon = styled(Image)`
  display: block;
`;

const StarText = styled.span`
  font-size: 18px;
  font-weight: 700;
  color: white;
  font-family: var(--font-figtree), "Figtree", sans-serif;
`;

const GitHubLink = styled(Link)`
  font-size: 16px;
  font-weight: 600;
  transition: color 150ms;
  color: #4B6486;
  font-family: var(--font-figtree), "Figtree", sans-serif;
  text-decoration: none;

  &:hover {
    color: white;
  }
`;
