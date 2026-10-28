"use client";

import { useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { DitherCheckbox } from "dither-kit/DitherCheckbox";
import { DitherField } from "dither-kit/DitherField";
import { DitherInput } from "dither-kit/DitherInput";
import { DitherTextarea } from "dither-kit/DitherTextarea";
import { PrimaryButton, SecondaryActionLink } from "@/components/leaderboard/RankingUI";

/* A focused form on the page's centred column. */
const Shell = styled.section`
  max-width: 680px;
  margin: 0 auto;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.75rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
  text-wrap: balance;

  @media (max-width: 640px) {
    font-size: 1.5rem;
  }
`;

const Description = styled.p`
  margin: 6px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Form = styled.form`
  display: grid;
  gap: 20px;
  margin-top: 24px;
  padding-top: 24px;
  border-top: 1px solid var(--service-border);

  /* Field labels are 14px sentence case; the kit's help line is 12px. */
  label {
    font-size: 0.875rem;
    font-weight: 500;
  }

  label + p,
  p[id$="-help"] {
    font-size: 0.8125rem;
  }
`;

const PublicRow = styled.div`
  display: grid;
  gap: 4px;
  justify-items: start;
`;

const PublicNote = styled.p`
  margin: 0;
  padding-left: 32px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;
  padding-top: 4px;
`;

const ErrorText = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

export default function CreateGroupClient() {
  const router = useRouter();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [isPublic, setIsPublic] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setIsSubmitting(true);
    setError(null);

    try {
      const response = await fetch("/api/groups", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name,
          description,
          isPublic,
        }),
      });
      const payload = await response.json();

      if (!response.ok) {
        throw new Error(payload.error || "Failed to create group");
      }

      router.push(`/groups/${payload.slug}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create group");
      setIsSubmitting(false);
    }
  }

  return (
    <Shell>
      <Title>Create group</Title>
      <Description>
        Start a scoped leaderboard and invite people by link or GitHub username.
      </Description>

      <Form onSubmit={handleSubmit}>
        <DitherField label="Group name" description="Shown in the directory and on the group's ranking.">
          <DitherInput
            value={name}
            onChange={setName}
            name="group-name"
            maxLength={100}
            required
            autoFocus
            placeholder="Team or workspace name"
          />
        </DitherField>
        <DitherField label="Description" description="Optional context shown in the public group directory.">
          <DitherTextarea
            value={description}
            onChange={setDescription}
            name="group-description"
            maxLength={500}
            rows={3}
            placeholder="What brings this group together?"
          />
        </DitherField>
        <PublicRow>
          <DitherCheckbox value={isPublic} onChange={setIsPublic}>
            Make this group public
          </DitherCheckbox>
          <PublicNote>Anyone can discover the group and view its ranking.</PublicNote>
        </PublicRow>
        {error && <ErrorText role="alert">{error}</ErrorText>}
        <Actions>
          <PrimaryButton type="submit" disabled={isSubmitting || !name.trim()}>
            {isSubmitting ? "Creating…" : "Create group"}
          </PrimaryButton>
          <SecondaryActionLink href="/leaderboard?view=groups">Cancel</SecondaryActionLink>
        </Actions>
      </Form>
    </Shell>
  );
}
