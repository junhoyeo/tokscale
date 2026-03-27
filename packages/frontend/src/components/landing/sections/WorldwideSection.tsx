"use client";
import { useState, useRef, useEffect, useCallback } from "react";
import Link from "next/link";
import Image from "next/image";
import styled from "styled-components";
import { useSquircleClip } from "../hooks";
import { SquircleBorder } from "../components";

import type { LeaderboardUser } from "@/lib/leaderboard/getLeaderboard";

interface WorldwideSectionProps {
  topUsersByCost?: LeaderboardUser[];
  topUsersByTokens?: LeaderboardUser[];
}

function formatCompactNumber(n: number): string {
  if (n >= 1_000_000_000)
    return (n / 1_000_000_000).toFixed(1).replace(/\.0$/, "") + "B";
  if (n >= 1_000_000)
    return (n / 1_000_000).toFixed(1).replace(/\.0$/, "") + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1).replace(/\.0$/, "") + "K";
  return n.toString();
}

function formatCompactCurrency(n: number): string {
  if (n >= 1_000)
    return "$" + (n / 1_000).toFixed(1).replace(/\.0$/, "") + "K";
  if (n >= 1) return "$" + n.toFixed(2);
  return "$" + n.toFixed(4);
}
export function WorldwideSection({
  topUsersByCost = [],
  topUsersByTokens = [],
}: WorldwideSectionProps) {
  const worldwideSection = useSquircleClip<HTMLDivElement>(32, 0.6, true, 1);
  const [activeTab, setActiveTab] = useState<"tokens" | "cost">("cost");
  const users = activeTab === "cost" ? topUsersByCost : topUsersByTokens;
  // Measure blue header bottom for border gradient transition
  const sectionElRef = useRef<HTMLDivElement>(null);
  const blueHeaderRef = useRef<HTMLDivElement>(null);
  const [gradientTransitionY, setGradientTransitionY] = useState(0);

  const sectionRef = useCallback(
    (node: HTMLDivElement | null) => {
      sectionElRef.current = node;
      worldwideSection.ref(node);
    },
    [worldwideSection.ref],
  );

  useEffect(() => {
    const section = sectionElRef.current;
    const header = blueHeaderRef.current;
    if (!section || !header) return;

    const update = () => {
      const sectionRect = section.getBoundingClientRect();
      const headerRect = header.getBoundingClientRect();
      setGradientTransitionY(Math.round(headerRect.bottom - sectionRect.top));
    };

    update();
    const ro = new ResizeObserver(update);
    ro.observe(section);
    return () => ro.disconnect();
  }, []);
  return (
    <>
      {/* SVG clip-path def for globe section */}
      {worldwideSection.svgDef && (
        <svg
          width="0"
          height="0"
          style={{ position: "absolute", overflow: "hidden" }}
          aria-hidden="true"
          role="presentation"
        >
          <defs>
            <clipPath id={worldwideSection.svgDef.id}>
              <path
                d={worldwideSection.svgDef.path}
                transform={`translate(0, -${worldwideSection.svgDef.cornerRadius})`}
              />
            </clipPath>
          </defs>
        </svg>
      )}
      {/* Separator before Globe */}
      <GlobeSeparatorBar />
      <GlobeSectionWrapper
        ref={sectionRef}
        style={{
          clipPath: worldwideSection.clipPath || undefined,
        }}
      >
        <SquircleBorder def={worldwideSection.borderDef} color="#0073FF" gradient={gradientTransitionY > 0 ? { colors: ["#0073FF", "#10233E"], transitionY: gradientTransitionY } : undefined} />
        <GlobeImageWrapper>
          <GlobeBackground />
          <GlobeFadeTop />
          <GlobeFadeBottom />
          <TrophyVideo
            autoPlay
            loop
            muted
            playsInline
            src="/assets/landing/trophy-cup-transparent.webm"
          />
        </GlobeImageWrapper>
        <GlobeContentStack>
          <GlobeBlueHeader ref={blueHeaderRef}>
            <GlobeHeaderText>
              THE LARGEST GROUP
              <br />
              OF TOKEN CONSUMERS
            </GlobeHeaderText>
          </GlobeBlueHeader>
          <GlobeTwoCol>
            <GlobeLeftCol>
              <GlobeLeftTitle>
                Tracking Trillions of
                <br />
                Tokens Worldwide
              </GlobeLeftTitle>
              <LeaderboardBtn href="/leaderboard">
                <LeaderboardBtnText>Leaderboard</LeaderboardBtnText>
              </LeaderboardBtn>
            </GlobeLeftCol>
            <GlobeRightCol>
              <LeaderboardWidget>
                <WidgetHeader>
                  <WidgetTitle>Top Users</WidgetTitle>
                  <TabSwitcher>
                    <Tab
                      $active={activeTab === "tokens"}
                      onClick={() => setActiveTab("tokens")}
                    >
                      Tokens
                    </Tab>
                    <Tab
                      $active={activeTab === "cost"}
                      onClick={() => setActiveTab("cost")}
                    >
                      Cost
                    </Tab>
                  </TabSwitcher>
                </WidgetHeader>
                <UserList>
                  {users.map((user) => (
                    <UserRow key={user.userId} href={`/u/${user.username}`}>
                      <RankBadge data-rank={user.rank}>
                        #{user.rank}
                      </RankBadge>
                      <UserAvatar
                        src={
                          user.avatarUrl ||
                          `https://github.com/${user.username}.png`
                        }
                        alt={user.displayName || user.username}
                        width={32}
                        height={32}
                        unoptimized
                      />
                      <UserInfo>
                        <UserName>
                          {user.displayName || user.username}
                        </UserName>
                        <UserHandle>@{user.username}</UserHandle>
                      </UserInfo>
                      <UserValue>
                        {activeTab === "tokens"
                          ? formatCompactNumber(user.totalTokens)
                          : formatCompactCurrency(user.totalCost)}
                      </UserValue>
                    </UserRow>
                  ))}
                </UserList>
              </LeaderboardWidget>
            </GlobeRightCol>
          </GlobeTwoCol>
        </GlobeContentStack>
      </GlobeSectionWrapper>
    </>
  );
}

const GlobeSeparatorBar = styled.div`
  width: 100%;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  background-image: url("/assets/landing/separator-pattern-slash@blue.svg");
  background-size: 24px 24px;
  background-repeat: repeat;
  border-top: 1px solid #0073FF;
  border-left: 1px solid #0073FF;
  border-right: 1px solid #0073FF;
  border-bottom: none;
`;

const GlobeSectionWrapper = styled.div`
  position: relative;
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  background: #010a15;
  overflow: hidden;
`;

const GlobeImageWrapper = styled.div`
  position: relative;
  width: 100%;
  height: 348px;
  overflow: hidden;
`;

const GlobeBackground = styled.div`
  position: absolute;
  inset: 0;
  background-color: #010101;
  background-image: url("/assets/landing/worldwide-section-bg.svg");
  background-size: cover;
  background-position: center;
  background-repeat: no-repeat;

  @media (max-width: 900px) {
    background-image: url("/assets/landing/worldwide-section-bg@mobile.svg");
  }
`;

const GlobeFadeTop = styled.div`
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 70px;
  background: linear-gradient(180deg, rgba(1, 1, 1, 1) 0%, rgba(1, 1, 1, 0) 100%);
  pointer-events: none;
  z-index: 1;
`;

const GlobeFadeBottom = styled.div`
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 70px;
  background: linear-gradient(0deg, rgba(1, 1, 1, 1) 0%, rgba(1, 1, 1, 0) 100%);
  pointer-events: none;
  z-index: 1;
`;

const GlobeContentStack = styled.div`
  position: relative;
  z-index: 1;
  width: 100%;
  display: flex;
  flex-direction: column;
  margin-top: -24px;
`;

const GlobeBlueHeader = styled.div`
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px 32px;
  background: #0073ff;
  border-left: 1px solid #0073FF;
  border-right: 1px solid #0073FF;
`;

const GlobeHeaderText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 20px;
  line-height: 1em;
  text-transform: uppercase;
  text-align: center;
  color: #ffffff;
`;

const GlobeTwoCol = styled.div`
  width: 100%;
  display: flex;
  flex-direction: row;
  align-items: stretch;
  background: #01070f;

  @media (max-width: 768px) {
    flex-direction: column;
  }
`;

const GlobeLeftCol = styled.div`
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 32px 32px 56px;
  border-right: 1px solid #10233e;

  @media (max-width: 768px) {
    border-right: none;
    border-bottom: 1px solid #10233e;
  }
`;

const GlobeLeftTitle = styled.h2`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 40px;
  line-height: 1.2em;
  letter-spacing: -0.03em;
  color: #ffffff;

  @media (max-width: 768px) {
    font-size: 32px;
  }

  @media (max-width: 480px) {
    font-size: 26px;
  }
`;

const LeaderboardBtn = styled(Link)`
  display: inline-flex;
  justify-content: center;
  align-items: center;
  gap: 4px;
  padding: 9px 28px;
  background: #ffffff;
  border-radius: 32px;
  text-decoration: none;
  width: fit-content;
  transition: opacity 0.15s;

  &:hover {
    opacity: 0.9;
  }
`;

const LeaderboardBtnText = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 23px;
  line-height: 1.2em;
  color: #000000;
`;

const GlobeRightCol = styled.div`
  flex: 1;
  align-self: stretch;
  display: flex;
  align-items: stretch;
  justify-content: flex-start;
  padding: 0 16px;
  background: #020f1e;
  overflow: hidden;
  @media (max-width: 768px) {
    padding: 0 16px;
  }
`;

const LeaderboardWidget = styled.div`
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px 0;
`;

const WidgetHeader = styled.div`
  padding-left: 8px;
  padding-right: 0;

  display: flex;
  align-items: center;
  justify-content: space-between;
`;

const WidgetTitle = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 17px;
  color: #5c7ba4;
  text-transform: uppercase;
`;

const TabSwitcher = styled.div`
  display: flex;
  gap: 2px;
  background: #0a1929;
  border-radius: 8px;
  padding: 2px;
`;

const Tab = styled.button<{ $active: boolean }>`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 600;
  font-size: 13px;
  padding: 5px 14px;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s;
  color: ${(p) => (p.$active ? "#ffffff" : "#6b7a90")};
  background: ${(p) => (p.$active ? "#0073FF" : "transparent")};

  &:hover {
    color: #ffffff;
  }
`;

const UserList = styled.div`
  display: flex;
  flex-direction: column;
`;

const UserRow = styled(Link)`
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  padding-right: 14px;
  border-radius: 10px;
  text-decoration: none;
  transition: background 0.15s;

  &:hover {
    background: rgba(255, 255, 255, 0.04);
  }
`;

const RankBadge = styled.span`
  width: 28px;
  height: 28px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-size: 12px;
  font-weight: 700;
  color: #6b7a90;
  background: linear-gradient(
    135deg,
    #01070f 0%,
    color-mix(in srgb, #0073ff 15%, #01070f) 50%,
    color-mix(in srgb, #0073ff 25%, #01070f) 100%
  );
  border: 1px solid #10233e;

  &[data-rank="1"] {
    color: #eab308;
    border-color: color-mix(in srgb, #eab308 30%, #10233e);
    background: linear-gradient(
      135deg,
      #01070f 0%,
      color-mix(in srgb, #eab308 10%, #01070f) 50%,
      color-mix(in srgb, #eab308 20%, #01070f) 100%
    );
  }

  &[data-rank="2"] {
    color: #9ca3af;
    border-color: color-mix(in srgb, #9ca3af 30%, #10233e);
    background: linear-gradient(
      135deg,
      #01070f 0%,
      color-mix(in srgb, #9ca3af 10%, #01070f) 50%,
      color-mix(in srgb, #9ca3af 20%, #01070f) 100%
    );
  }

  &[data-rank="3"] {
    color: #d97706;
    border-color: color-mix(in srgb, #d97706 30%, #10233e);
    background: linear-gradient(
      135deg,
      #01070f 0%,
      color-mix(in srgb, #d97706 10%, #01070f) 50%,
      color-mix(in srgb, #d97706 20%, #01070f) 100%
    );
  }
`;

const UserAvatar = styled(Image)`
  border-radius: 50%;
  flex-shrink: 0;
`;

const UserInfo = styled.div`
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1;
`;

const UserName = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 600;
  font-size: 14px;
  color: #ffffff;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const UserHandle = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 400;
  font-size: 12px;
  color: #6b7a90;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const UserValue = styled.span`
  font-family: var(--font-figtree), "Figtree", sans-serif;
  font-weight: 700;
  font-size: 15px;
  color: #0073FF;
  flex-shrink: 0;
  margin-left: auto;
`;

const TrophyVideo = styled.video`
  position: absolute;

  width: 396px;
  height: 396px;
  min-width: 396px;
  min-height: 396px;
  max-width: 396px;
  max-height: 396px;

  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  object-fit: contain;
  pointer-events: none;
  z-index: 2;
`;
