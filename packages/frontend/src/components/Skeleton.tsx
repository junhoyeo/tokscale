"use client";

import styled, { keyframes } from "styled-components";

/**
 * A 4×4 Bayer tile at half density, as an inline SVG: every skeleton is a
 * field of dither cells rather than a shimmering bar, and the field is real
 * markup, so it renders in the first paint with no canvas and no JS.
 */
const DITHER_TILE =
  "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4' height='4'%3E%3Cg fill='%23ffffff' fill-opacity='0.13'%3E%3Crect x='0' y='0' width='1' height='1'/%3E%3Crect x='2' y='0' width='1' height='1'/%3E%3Crect x='1' y='1' width='1' height='1'/%3E%3Crect x='3' y='1' width='1' height='1'/%3E%3Crect x='0' y='2' width='1' height='1'/%3E%3Crect x='2' y='2' width='1' height='1'/%3E%3Crect x='1' y='3' width='1' height='1'/%3E%3Crect x='3' y='3' width='1' height='1'/%3E%3C/g%3E%3C/svg%3E\")";

const pulse = keyframes`
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.55;
  }
`;

interface SizedSkeletonProps {
  $w?: string;
  $h?: string;
  $rounded?: string;
  $mb?: string;
  $ml?: string;
}

export const Skeleton = styled.div<SizedSkeletonProps>`
  animation: ${pulse} 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  border-radius: ${(props) => props.$rounded || "var(--radius, 2px)"}; /* taste-skill-ignore: radius, not a box */
  background-color: var(--v2-surface-raised);
  background-image: ${DITHER_TILE};
  background-size: 4px 4px;

  width: ${(props) => props.$w};
  height: ${(props) => props.$h};
  margin-bottom: ${(props) => props.$mb};
  margin-left: ${(props) => (props.$ml === "auto" ? "auto" : props.$ml)};

  @media (prefers-reduced-motion: reduce) {
    animation: none;
  }
`;

const LeaderboardContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: 32px;
`;

const StatsGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1px;
  background: var(--v2-rule);

  @media (min-width: 768px) {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
`;

const StatCell = styled.div`
  padding: 16px;
  background: var(--v2-canvas);
`;

const TableContainer = styled.div`
  display: flex;
  flex-direction: column;
`;

const TableHeader = styled.div`
  padding: 0 0 12px;
`;

const HeaderContent = styled.div`
  display: flex;
  gap: 24px;
`;

const TableRow = styled.div`
  padding: 12px 0;
`;

const RowContent = styled.div`
  display: flex;
  align-items: center;
  gap: 24px;
`;

const UserCell = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
`;

export function LeaderboardSkeleton() {
  return (
    <LeaderboardContainer aria-busy="true">
      <StatsGrid>
        {[0, 1, 2, 3].map((cell) => (
          <StatCell key={cell}>
            <Skeleton $h="1rem" $w="5rem" $mb="0.5rem" />
            <Skeleton $h="2rem" $w="6rem" />
          </StatCell>
        ))}
      </StatsGrid>

      <TableContainer>
        <TableHeader>
          <HeaderContent>
            <Skeleton $h="1rem" $w="3rem" />
            <Skeleton $h="1rem" $w="6rem" />
            <Skeleton $h="1rem" $w="4rem" $ml="auto" />
            <Skeleton $h="1rem" $w="4rem" />
          </HeaderContent>
        </TableHeader>
        {[0, 1, 2, 3, 4, 5, 6, 7].map((row) => (
          <TableRow key={row}>
            <RowContent>
              <Skeleton $h="1.5rem" $w="2rem" />
              <UserCell>
                <Skeleton $h="2rem" $w="2rem" $rounded="9999px" />
                <div>
                  <Skeleton $h="1rem" $w="8rem" $mb="0.375rem" />
                  <Skeleton $h="0.75rem" $w="5rem" />
                </div>
              </UserCell>
              <Skeleton $h="1.25rem" $w="4rem" $ml="auto" />
              <Skeleton $h="1.25rem" $w="3.5rem" />
            </RowContent>
          </TableRow>
        ))}
      </TableContainer>
    </LeaderboardContainer>
  );
}
