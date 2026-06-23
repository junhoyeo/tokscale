"use client";

import { useState, useRef } from "react";
import Image from "next/image";
import styled from "styled-components";

interface HeroSectionProps {
  stargazersCount: number;
}

export function HeroSection({ stargazersCount }: HeroSectionProps) {
  const [isHovered, setIsHovered] = useState(false);
  const [mousePos, setMousePos] = useState({ x: 0, y: 0 });
  const containerRef = useRef<HTMLDivElement>(null);

  const handleMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect();
      setMousePos({
        x: e.clientX - rect.left,
        y: e.clientY - rect.top,
      });
    }
  };

  const starsText =
    stargazersCount > 0
      ? `${stargazersCount.toLocaleString()} stars`
      : "Star on GitHub";

  return (
    <>
      <HeroRow>
        <HeroLeft>
          <HeroBgStarfield
            src="/assets/landing/hero-bg-starfield.png"
            alt=""
            width={1076}
            height={536}
          />
          <HeroVideo
            src="/assets/landing/hero-video-transparent.webm"
            autoPlay
            loop
            muted
            playsInline
          />
        </HeroLeft>

        <HeroRight>
          {/* Top part with BG image */}
          <HeroTopSection>
            <HeroContent>
              <HeroTitle>
                The Kardashev
                <br />
                Scale for AI Devs
              </HeroTitle>

              <HeroButtonsRow>
                <CTAWrapper>
                  <StarButton
                    href="https://github.com/junhoyeo/tokscale"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    <StarGlow />
                    <StarButtonText>Star us on GitHub</StarButtonText>
                    <ArrowIcon />
                  </StarButton>
                  <StarBadge>
                    <Image
                      src="/assets/landing/star-link-icon.svg"
                      alt="Star"
                      width={18}
                      height={18}
                    />
                    <StarBadgeText>{starsText}</StarBadgeText>
                  </StarBadge>
                </CTAWrapper>
              </HeroButtonsRow>
            </HeroContent>
          </HeroTopSection>

          {/* Bottom part: Trusted By */}
          <TrustedBySection
            ref={containerRef}
            onMouseEnter={() => setIsHovered(true)}
            onMouseLeave={() => setIsHovered(false)}
            onMouseMove={handleMouseMove}
          >
            <TrustedByLabel><IB>Trusted by</IB> <IB>professionals at</IB></TrustedByLabel>
            <TrustedByLogos>
              <TrustedByLogo
                src="/assets/landing/trusted-by-microsoft.svg"
                alt="Microsoft"
                width={104}
                height={22}
              />
              <TrustedByLogo
                src="/assets/landing/trusted-by-amazon.svg"
                alt="Amazon"
                width={72}
                height={24}
              />
              <TrustedByLogo
                src="/assets/landing/trusted-by-meta.svg"
                alt="Meta"
                width={103}
                height={17}
              />
              <TrustedByLogo
                src="/assets/landing/trusted-by-google.svg"
                alt="Google"
                width={76}
                height={26}
              />
              <TrustedByLogo
                src="/assets/landing/trusted-by-toss.png"
                alt="Toss"
                width={86}
                height={26}
              />
              <TrustedByLogo
                src="/assets/landing/trusted-by-hashed.svg"
                alt="Hashed"
                width={108}
                height={28}
              />
            </TrustedByLogos>
            <CursorTooltip
              $visible={isHovered}
              style={{
                left: mousePos.x,
                top: mousePos.y,
              }}
            >
              Based on{" "}
              <TooltipLink
                href="https://github.com/junhoyeo/tokscale"
                target="_blank"
                rel="noopener noreferrer"
              >
                tokscale
              </TooltipLink>{" "}
              community reports
            </CursorTooltip>
          </TrustedBySection>
        </HeroRight>
      </HeroRow>
    </>
  );
}

/* ── Hero Styled Components ── */
const HeroRow = styled.div`
  width: 100%;
  display: flex;
  flex-direction: row;
  height: 536px;
  border: 1px solid #10233e;
  overflow: hidden;

  @media (max-width: 900px) {
    flex-direction: column;
    height: auto;
  }
`;

const HeroLeft = styled.div`
  position: relative;
  flex: 0 0 600px;
  display: flex;
  flex-direction: column;
  align-items: center;
  align-self: stretch;
  justify-content: center;
  background: #000000;
  border-right: 1px solid #10233e;
  overflow: hidden;
  padding-bottom: 64px;

  @media (max-width: 1060px) {
    flex: 1;
  }

  @media (max-width: 900px) {
    flex: 0 0 auto;
    width: 100%;
    height: 400px;
    border-right: none;
    /* border-bottom: 1px solid #10233e; */
    padding-bottom: 32px;
    padding-top: 60px;
    overflow: visible;
  }
`;

const HeroBgStarfield = styled(Image)`
  position: absolute;
  top: 0;
  left: 50%;
  transform: translateX(-50%);
  width: 1076px;
  height: 536px;
  object-fit: cover;
  pointer-events: none;

  @media (max-width: 900px) {
    width: 100%;
    height: 100%;
  }
`;

const HeroVideo = styled.video`
  position: relative;
  width: 552px;
  max-width: 552px;
  min-width: 552px;
  height: 552px;
  max-height: 552px;
  min-height: 552px;

  object-fit: contain;
  z-index: 1;
  margin-top: 120px;
  margin-right: -40px;

  @media (max-width: 900px) {
    width: 70%;
    max-width: none;
    min-width: none;
    max-height: none;
    min-height: none;
  }
`;

const HeroRight = styled.div`
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-self: stretch;
  overflow: hidden;
`;

const HeroTopSection = styled.div`
  position: relative;
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  gap: 17px;
  padding: 97px 40px 33px;
  background-image: url("/assets/landing/hero-trusted-bg.png");
  background-size: cover;
  background-position: center;
  border-bottom: 1px solid #10233e;
  
  @media (max-width: 900px) {
    padding: 64px 20px;
    justify-content: center;

    background-image: linear-gradient(to bottom, rgba(1, 17, 36, 0), #011124);
    z-index: 1;
  }

  @media (max-width: 480px) {
    padding: 28px 20px 48px
  }
`;

const HeroContent = styled.div`
  display: flex;
  flex-direction: column;
  gap: 20px;

  @media (max-width: 900px) {
    align-items: center;
  }
`;

const HeroTitle = styled.h1`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 48px;
  line-height: 0.94em;
  letter-spacing: -0.05em;
  color: #ffffff;

  @media (max-width: 900px) {
    text-align: center;
  }

  @media (max-width: 480px) {
    font-size: 36px;
  }
`;

const HeroButtonsRow = styled.div`
  display: flex;
  flex-direction: row;
  gap: 20px;
`;

const CTAWrapper = styled.div`
  position: relative;
  width: fit-content;
  height: 48px;

  @media (max-width: 480px) {
    width: fit-content;
    height: 44px;
  }
`;

const StarButton = styled.a`
  position: relative;
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 6px;
  width: fit-content;
  height: 48px;
  padding: 0 28px;
  background: #000000;
  border-radius: 16px;
  border: none;
  box-shadow: 0px 4px 48.3px 0px rgba(0, 115, 255, 0.14);
  text-decoration: none;
  overflow: hidden;
  transition: opacity 0.2s;
  flex-shrink: 0;

  &::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 16px;
    padding: 1px;
    background: linear-gradient(207deg, rgba(70, 107, 159, 1) 0%, rgba(0, 115, 255, 1) 100%);
    -webkit-mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
    -webkit-mask-composite: xor;
    mask-composite: exclude;
    pointer-events: none;
  }

  &:hover {
    opacity: 0.9;
  }

  @media (max-width: 480px) {
    padding: 0 20px;
  }
`;

const StarGlow = styled.div`
  position: absolute;
  left: -36px;
  top: 16px;
  width: 89px;
  height: 70px;
  border-radius: 50%;
  background: #0073ff;
  opacity: 0.54;
  filter: blur(39.2px);
  pointer-events: none;
`;

const StarButtonText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 800;
  font-size: 18px;
  line-height: 1.33em;
  letter-spacing: -0.0174em;
  text-align: center;
  color: #ffffff;
  white-space: nowrap;
  z-index: 1;

  @media (max-width: 480px) {
    font-size: 16px;
  }
`;

const StarBadge = styled.div`
  position: absolute;
  bottom: -20px;
  right: -16px;
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 3.5px;
  padding: 6px 8px;
  background: rgba(0, 115, 255, 0.08);
  border: 1px solid rgba(0, 115, 255, 0.26);
  backdrop-filter: blur(4px);
  border-radius: 12px;
`;

const StarBadgeText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 16px;
  line-height: 1em;
  letter-spacing: -0.0114em;
  text-align: center;
  color: #87f0f2;
`;

const TrustedBySection = styled.div`
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 28px;
  padding: 28px 32px 36px;
  background: #01070f;
  cursor: help;

  @media (max-width: 900px) {
    padding: 20px 20px 28px;
    align-items: center;
  }
`;

const TrustedByLabel = styled.p`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 16px;
  line-height: 1.25em;
  text-transform: uppercase;
  color: #8292b1;

  @media (max-width: 900px) {
    text-align: center;
  }
`;

const TrustedByLogos = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 38px;
  max-width: 408px;

  @media (max-width: 900px) {
    justify-content: center;
  }
`;

const TrustedByLogo = styled(Image)`
  height: auto;
  object-fit: contain;
`;

const ArrowIconSvg = styled.svg`
  margin-left: 6px;
  flex-shrink: 0;
  position: relative;
  z-index: 1;
  transition: transform 0.2s ease;

  ${StarButton}:hover & {
    transform: translateX(3px);
  }
`;

const ArrowIcon = () => (
  <ArrowIconSvg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
  >
    <path
      d="M3.33334 8H12.6667"
      stroke="#FFFFFF"
      strokeWidth="1.33301"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
    <path
      d="M8.66666 4L12.6667 8L8.66666 12"
      stroke="#FFFFFF"
      strokeWidth="1.33301"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </ArrowIconSvg>
);

const CursorTooltip = styled.div<{ $visible: boolean }>`
  position: absolute;
  pointer-events: none;
  transform: translate(8px, 16px);
  background-color: #111B2C;
  color: #e5e5e5;
  border-radius: 8px;
  padding: 10px 14px;
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-size: 14px;
  line-height: 1.5;
  letter-spacing: -0.2px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.4), 0 0 0 1px rgba(255, 255, 255, 0.06);
  user-select: none;
  white-space: nowrap;
  z-index: 1000;
  opacity: ${({ $visible }) => ($visible ? 1 : 0)};
  transition: opacity 0.15s ease;
`;

const TooltipLink = styled.a`
  color: #0073ff;
  text-decoration: underline;
  text-underline-offset: 2px;
  transition: color 0.15s ease;

  &:hover {
    color: #3399ff;
  }
`;

const IB = styled.span`
  display: inline-block;
`;
