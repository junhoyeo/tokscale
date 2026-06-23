"use client";

import { useState } from "react";
import styled from "styled-components";
import type { TokenContributionData } from "@/lib/types";
import { DataInput } from "@/components/DataInput";
import { GraphContainer } from "@/components/GraphContainer";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";

const Container = styled.div`
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  background-color: var(--color-bg-default);
`;

const Main = styled.main`
  flex: 1;
  max-width: 1280px;
  margin: 0 auto;
  padding: 40px 24px;
  width: 100%;
`;

const HeaderSection = styled.div`
  margin-bottom: 32px;
`;

const Title = styled.h1`
  font-size: 30px;
  font-weight: bold;
  margin-bottom: 8px;
  color: var(--color-fg-default);
`;

const Description = styled.p`
  color: var(--color-fg-muted);
`;

const ContentWrapper = styled.div`
  display: flex;
  flex-direction: column;
  gap: 32px;
`;

const InfoCard = styled.div`
  border-radius: 16px;
  border: 1px solid var(--color-border-default);
  padding: 20px;
  background-color: var(--color-bg-default);
`;

const InfoContent = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;
  font-size: 14px;
`;

const MutedText = styled.span`
  color: var(--color-fg-muted);
`;

const BoldText = styled.span`
  font-weight: 600;
  color: var(--color-fg-default);
`;

const Separator = styled.span`
  color: var(--color-border-default);
`;

const PrimaryText = styled.span`
  font-weight: 600;
  color: var(--color-primary);
`;

const LoadButton = styled.button`
  margin-left: auto;
  padding: 8px 16px;
  font-size: 14px;
  font-weight: 500;
  border-radius: 8px;
  transition: opacity 0.2s;
  color: var(--color-fg-muted);
  
  &:hover {
    opacity: 0.8;
  }
`;

export default function LocalClient() {
  const [data, setData] = useState<TokenContributionData | null>(null);

  return (
    <Container>
      <Navigation />

      <Main>
        <HeaderSection>
          <Title>Local Viewer</Title>
          <Description>
            View your token usage data locally without submitting
          </Description>
        </HeaderSection>

        {!data ? (
          <DataInput onDataLoaded={setData} />
        ) : (
          <ContentWrapper>
            <InfoCard>
              <InfoContent>
                <MutedText>Data loaded:</MutedText>
                <BoldText>
                  {data.meta.dateRange.start} - {data.meta.dateRange.end}
                </BoldText>
                <Separator>|</Separator>
                <PrimaryText>
                  ${data.summary.totalCost.toFixed(2)} total
                </PrimaryText>
                <Separator>|</Separator>
                <MutedText>
                  {data.summary.activeDays} active days
                </MutedText>
                <LoadButton onClick={() => setData(null)}>
                  Load Different Data
                </LoadButton>
              </InfoContent>
            </InfoCard>
            <GraphContainer data={data} />
          </ContentWrapper>
        )}
      </Main>

      <Footer />
    </Container>
  );
}
