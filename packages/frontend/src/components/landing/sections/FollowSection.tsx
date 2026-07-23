"use client";

import Image from "next/image";
import styled from "styled-components";

export function FollowSection() {
  return (
    <FollowSectionWrapper>
      <CardOuter>
        <TopBar />

        <MiddleContentOuter>
          <MiddleContentInner>
            <Avatar3D>
              <Image
                src="/assets/landing/follow-3d-avatar.webp"
                alt="@junhoyeo"
                width={268}
                height={268}
                style={{ width: 268, height: 268, objectFit: "cover" }}
              />
            </Avatar3D>

            <GlowEllipse />

            <TextGroup>
              <HeadingText>
                I drop new open-source work every week.
                <br />
                Don&#39;t miss the next one.
              </HeadingText>
              <Actions>
                <FollowLink
                  href="https://github.com/junhoyeo"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Follow @junhoyeo on GitHub
                </FollowLink>
                <SponsorLink
                  href="https://github.com/sponsors/junhoyeo"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Sponsor Tokscale
                </SponsorLink>
              </Actions>
            </TextGroup>
          </MiddleContentInner>
        </MiddleContentOuter>

        <BottomGradientWrapper>
          <BottomInner>
            <BlueBanner>
              <RepoNameText>junhoyeo/tokscale</RepoNameText>
            </BlueBanner>
          </BottomInner>
        </BottomGradientWrapper>
      </CardOuter>
    </FollowSectionWrapper>
  );
}

/* ── Follow Section Styled Components ── */

const FollowSectionWrapper = styled.div`
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 0 0 64px;
`;

const CardOuter = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  max-width: 1200px;
  border-left: 1px solid #0073ff;
  border-right: 1px solid #0073ff;
`;

const TopBar = styled.div`
  width: 100%;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;

  background-image: url("/assets/landing/separator-pattern-slash@blue.svg");
  background-size: 24px 24px;
  background-repeat: repeat;
  
  border-top: 1px solid #0073FF;
  border-bottom: 1px solid #0073FF;
`;

const MiddleContentOuter = styled.div`
  width: 100%;
  padding: 0 8px;
  display: flex;
  justify-content: center;
`;

const MiddleContentInner = styled.div`
  position: relative;
  overflow: visible;
  width: 100%;
  background: #01070f;
  border-left: 1px solid #0073ff;
  border-right: 1px solid #0073ff;
  padding: 148px 32px 40px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 40px;

  @media (max-width: 768px) {
    padding: 120px 24px 32px;
    gap: 32px;
  }

  @media (max-width: 480px) {
    padding: 100px 16px 24px;
    gap: 24px;
  }
`;

const Avatar3D = styled.div`
  position: absolute;
  top: -26px;
  left: 50%;
  transform: translateX(-50%);
  width: 268px;
  height: 268px;
  z-index: 1;

  @media (max-width: 480px) {
    width: 200px;
    height: 200px;
    top: -20px;

    img {
      width: 200px !important;
      height: 200px !important;
    }
  }
`;

const GlowEllipse = styled.div`
  position: absolute;
  width: 200px;
  height: 200px;
  left: 500px;
  top: 242px;
  background: rgba(21, 131, 199, 0.26);
  border-radius: 50%;
  filter: blur(170.5px);
  pointer-events: none;

  @media (max-width: 960px) {
    left: 50%;
    transform: translateX(-50%);
    top: 200px;
  }
`;

const TextGroup = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  z-index: 1;
`;

const HeadingText = styled.p`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 36px;
  line-height: 1.1em;
  letter-spacing: -0.02em;
  text-align: center;
  color: #92e7ff;

  @media (max-width: 768px) {
    font-size: 28px;
  }

  @media (max-width: 480px) {
    font-size: 22px;
  }
`;

const Actions = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  min-width: 0;

  @media (max-width: 480px) {
    flex-direction: column;
    width: 100%;
  }
`;

const FollowLink = styled.a`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 44px;
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 600;
  font-size: 24px;
  line-height: 1.2em;
  letter-spacing: -0.03em;
  text-align: center;
  color: #b6c0d4;
  text-decoration: none;

  &:hover {
    color: #ffffff;
  }

  &:focus-visible {
    outline: 2px solid #75b6ff;
    outline-offset: 3px;
  }

  @media (max-width: 768px) {
    font-size: 20px;
  }

  @media (max-width: 480px) {
    min-height: 48px;
    font-size: 18px;
  }
`;

const SponsorLink = styled.a`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 44px;
  padding: 0 18px;
  flex-shrink: 0;
  border: 1px solid rgba(135, 240, 242, 0.32);
  border-radius: 12px;
  background: rgba(0, 115, 255, 0.08);
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 600;
  font-size: 16px;
  line-height: 1.2em;
  color: #d8f8ff;
  text-decoration: none;

  &:hover {
    border-color: rgba(135, 240, 242, 0.56);
    background: rgba(0, 115, 255, 0.16);
  }

  &:focus-visible {
    outline: 2px solid #75b6ff;
    outline-offset: 3px;
  }

  @media (max-width: 480px) {
    width: 100%;
    max-width: 240px;
    min-height: 48px;
  }
`;

const BottomGradientWrapper = styled.div`
  width: calc(100% + 2px);
  margin: 0 -1px;
  padding: 0 1px 1px;
  background: linear-gradient(
    180deg,
    rgba(0, 115, 255, 1) 0%,
    rgba(10, 25, 45, 1) 100%
  );
`;

const BottomInner = styled.div`
  background: #010a15;
  width: 100%;
`;

const BlueBanner = styled.div`
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px 32px;
  background: #0073ff;
`;

const RepoNameText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 20px;
  line-height: 1em;
  text-transform: uppercase;
  text-align: center;
  color: #ffffff;
`;
