"use client";

import Link from "next/link";
import styled from "styled-components";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";

const Container = styled.div`
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  background-color: #10121C;
`;

const Main = styled.main`
  flex: 1;
  max-width: 800px;
  margin: 0 auto;
  padding: 40px 24px;
  width: 100%;
`;

const ContentWrapper = styled.div`
  text-align: center;
  padding-top: 80px;
  padding-bottom: 80px;
`;

const Title = styled.h1`
  font-size: 24px;
  font-weight: bold;
  color: white;
  margin-bottom: 8px;
`;

const Description = styled.p`
  margin-bottom: 24px;
  color: #4B6486;
`;

const BackButton = styled(Link)`
  display: inline-flex;
  align-items: center;
  padding: 8px 16px;
  color: white;
  font-weight: 500;
  border-radius: 8px;
  background: #0073FF;
  transition: opacity 0.2s;

  &:hover {
    opacity: 0.9;
  }
`;

export default function ProfileNotFound() {
  return (
    <Container>
      <Navigation />
      <Main>
        <ContentWrapper>
          <Title>User Not Found</Title>
          <Description>
            This user doesn&apos;t exist or hasn&apos;t submitted any data yet.
          </Description>
          <BackButton href="/">
            Back to Leaderboard
          </BackButton>
        </ContentWrapper>
      </Main>
      <Footer />
    </Container>
  );
}
