"use client";

import styled from "styled-components";
import { PrimaryActionLink, SecondaryActionLink } from "@/components/leaderboard/RankingUI";
import { PageLead, PageTitle } from "./ExploreUI";

const Wrap = styled.div`
  display: grid;
  max-width: 560px;
  gap: 12px;
  padding: 48px 0;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 8px;
`;

export function ModelNotFound() {
  return (
    <Wrap>
      <PageTitle>Model not found</PageTitle>
      <PageLead>
        Tokscale has no public usage for this model key. Keys are lowercase and drop date suffixes, for example{" "}
        <code>claude-opus-5</code>.
      </PageLead>
      <Actions>
        <PrimaryActionLink href="/models">Browse all models</PrimaryActionLink>
        <SecondaryActionLink href="/leaderboard?view=models">Model leaderboard</SecondaryActionLink>
      </Actions>
    </Wrap>
  );
}
