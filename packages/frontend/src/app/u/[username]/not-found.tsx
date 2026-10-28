"use client";

import Link from "next/link";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";

const Wrap = styled.div`
  max-width: 46rem;
  padding: clamp(2rem, 8vh, 5rem) 0;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: clamp(1.625rem, 3.4vw, 2rem);
  font-weight: 600;
  letter-spacing: -0.02em;
  line-height: 1.15;
`;

const Copy = styled.p`
  max-width: 46ch;
  margin: 0.875rem 0 1.5rem;
  color: var(--service-text-muted);
  font-size: 1rem;
  line-height: 1.55;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 1.25rem;

  button.profile-not-found-action {
    padding-right: 1.125rem;
    padding-left: 1.125rem;
    font-size: 0.875rem;
  }
`;

export default function ProfileNotFound() {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <Wrap>
          <Title>Profile not found</Title>
          <Copy>
            This account does not exist, or it has not submitted any usage yet.
          </Copy>
          <Actions>
            <DitherButton
              nativeButton={false}
              role="link"
              render={<Link href="/leaderboard" />}
              color="blue"
              variant="solid"
              className="profile-not-found-action"
            >
              Browse the leaderboard
            </DitherButton>
          </Actions>
        </Wrap>
      </main>
      <PublicFooter />
    </div>
  );
}
