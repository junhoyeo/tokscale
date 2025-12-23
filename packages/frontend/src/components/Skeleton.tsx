"use client";

import styled, { keyframes } from "styled-components";

const pulse = keyframes`
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: .5;
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
  border-radius: ${props => props.$rounded || '0.25rem'};
  background-color: var(--color-border-default);
  
  width: ${props => props.$w};
  height: ${props => props.$h};
  margin-bottom: ${props => props.$mb};
  margin-left: ${props => props.$ml === 'auto' ? 'auto' : props.$ml};
`;

const LeaderboardContainer = styled.div`
  & > * + * {
    margin-top: 1rem;
  }
`;

const StatsGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;
  margin-bottom: 2.5rem;

  @media (min-width: 768px) {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
`;

const StatCard = styled.div`
  border-radius: 0.75rem;
  border: 1px solid;
  padding: 1rem;
`;

const TableContainer = styled.div`
  border-radius: 1rem;
  border: 1px solid;
  overflow: hidden;
`;

const TableHeader = styled.div`
  border-bottom: 1px solid;
  padding: 0.75rem 1.5rem;
`;

const HeaderContent = styled.div`
  display: flex;
  gap: 1.5rem;
`;

const TableRow = styled.div`
  padding: 1rem 1.5rem;
  border-bottom: 1px solid;

  &:last-child {
    border-bottom: 0;
  }
`;

const RowContent = styled.div`
  display: flex;
  align-items: center;
  gap: 1.5rem;
`;

const UserCell = styled.div`
  display: flex;
  align-items: center;
  gap: 0.75rem;
`;

export function LeaderboardSkeleton() {
  return (
    <LeaderboardContainer>
      <StatsGrid>
        {[...Array(4)].map((_, i) => (
          <StatCard
            key={i}
            style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
          >
            <Skeleton $h="1rem" $w="5rem" $mb="0.5rem" />
            <Skeleton $h="2rem" $w="6rem" />
          </StatCard>
        ))}
      </StatsGrid>

      <TableContainer
        style={{ backgroundColor: "#10121C", borderColor: "#1E2733" }}
      >
        <TableHeader
          style={{ backgroundColor: "var(--color-bg-elevated)", borderColor: "var(--color-border-default)" }}
        >
          <HeaderContent>
            <Skeleton $h="1rem" $w="3rem" />
            <Skeleton $h="1rem" $w="6rem" />
            <Skeleton $h="1rem" $w="4rem" $ml="auto" />
            <Skeleton $h="1rem" $w="4rem" />
          </HeaderContent>
        </TableHeader>
        {[...Array(10)].map((_, i) => (
          <TableRow
            key={i}
            style={{ borderColor: "var(--color-border-default)" }}
          >
            <RowContent>
              <Skeleton $h="1.5rem" $w="2rem" />
              <UserCell>
                <Skeleton $h="2.5rem" $w="2.5rem" $rounded="9999px" />
                <div>
                  <Skeleton $h="1rem" $w="8rem" $mb="0.25rem" />
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

const ProfileContainer = styled.div`
  & > * + * {
    margin-top: 2rem;
  }
`;

const ProfileHeader = styled.div`
  display: flex;
  align-items: flex-start;
  gap: 1.5rem;
  margin-bottom: 2rem;
`;

const ProfileInfo = styled.div`
  flex: 1 1 0%;
`;

const NameRow = styled.div`
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 0.5rem;
`;

const TagsRow = styled.div`
  display: flex;
  gap: 0.5rem;
`;

const ProfileStatsGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;

  @media (min-width: 768px) {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
`;

const ContentCard = styled.div`
  border-radius: 1rem;
  border: 1px solid;
  padding: 1.5rem;
`;

export function ProfileSkeleton() {
  return (
    <ProfileContainer>
      <ProfileHeader>
        <Skeleton $w="6rem" $h="6rem" $rounded="1rem" />
        <ProfileInfo>
          <NameRow>
            <Skeleton $h="2rem" $w="12rem" />
            <Skeleton $h="1.5rem" $w="3rem" $rounded="0.5rem" />
          </NameRow>
          <Skeleton $h="1rem" $w="6rem" $mb="0.75rem" />
          <TagsRow>
            <Skeleton $h="1.5rem" $w="4rem" $rounded="0.5rem" />
            <Skeleton $h="1.5rem" $w="5rem" $rounded="0.5rem" />
          </TagsRow>
        </ProfileInfo>
      </ProfileHeader>

      <ProfileStatsGrid>
        {[...Array(4)].map((_, i) => (
          <StatCard
            key={i}
            style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
          >
            <Skeleton $h="1rem" $w="5rem" $mb="0.5rem" />
            <Skeleton $h="2rem" $w="6rem" />
          </StatCard>
        ))}
      </ProfileStatsGrid>

      <ContentCard
        style={{ backgroundColor: "#10121C", borderColor: "#1E2733" }}
      >
        <Skeleton $h="1.5rem" $w="9rem" $mb="1rem" />
        <ProfileStatsGrid>
          {[...Array(4)].map((_, i) => (
            <div key={i}>
              <Skeleton $h="1rem" $w="4rem" $mb="0.25rem" />
              <Skeleton $h="1.5rem" $w="5rem" />
            </div>
          ))}
        </ProfileStatsGrid>
      </ContentCard>

      <ContentCard
        style={{ backgroundColor: "#10121C", borderColor: "#1E2733" }}
      >
        <Skeleton $h="1.5rem" $w="6rem" $mb="1rem" />
        <Skeleton $h="10rem" $w="100%" $rounded="0.5rem" />
      </ContentCard>
    </ProfileContainer>
  );
}

export function StatCardSkeleton() {
  return (
    <StatCard
      style={{ backgroundColor: "#10121C", borderColor: "#1E2733" }}
    >
      <Skeleton $h="1rem" $w="5rem" $mb="0.5rem" />
      <Skeleton $h="2rem" $w="6rem" />
    </StatCard>
  );
}

const SimpleTableRow = styled.div`
  padding: 1rem 1.5rem;
  border-bottom: 1px solid;
`;

export function TableRowSkeleton() {
  return (
    <SimpleTableRow
      style={{ borderColor: "#1E2733" }}
    >
      <RowContent>
        <Skeleton $h="1.5rem" $w="2rem" />
        <UserCell>
          <Skeleton $h="2.5rem" $w="2.5rem" $rounded="9999px" />
          <div>
            <Skeleton $h="1rem" $w="8rem" $mb="0.25rem" />
            <Skeleton $h="0.75rem" $w="5rem" />
          </div>
        </UserCell>
        <Skeleton $h="1.25rem" $w="4rem" $ml="auto" />
        <Skeleton $h="1.25rem" $w="3.5rem" />
      </RowContent>
    </SimpleTableRow>
  );
}
