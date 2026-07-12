"use client";

import { useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { SecondaryActionLink } from "@/components/leaderboard/RankingUI";

const Shell = styled.section`
  max-width: 640px;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.5rem;
  font-weight: 600;
  letter-spacing: -0.02em;
  text-wrap: balance;
`;

const Description = styled.p`
  margin: 6px 0 24px;
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
  gap: 16px;
  padding: 20px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
`;

const Field = styled.label`
  display: grid;
  gap: 8px;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Input = styled.input`
  min-height: 38px;
  padding: 0 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: var(--service-surface);
  color: var(--service-text);
  font: inherit;

  &:focus-visible {
    border-color: var(--service-focus);
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

const Textarea = styled.textarea`
  min-height: 96px;
  padding: 10px 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: var(--service-surface);
  color: var(--service-text);
  font: inherit;
  resize: vertical;

  &:focus-visible {
    border-color: var(--service-focus);
    outline: 2px solid var(--service-focus);
    outline-offset: -1px;
  }

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const CheckboxLabel = styled.label`
  display: flex;
  align-items: flex-start;
  gap: 10px;
  color: var(--service-text);
  font-size: 0.875rem;

  input {
    width: 18px;
    height: 18px;
    flex: 0 0 auto;
    margin: 1px 0 0;
    accent-color: var(--service-accent);
  }

  @media (max-width: 640px) {
    font-size: 1rem;

    input {
      width: 20px;
      height: 20px;
    }
  }
`;

const CheckboxCopy = styled.span`
  display: grid;
  gap: 2px;
`;

const FieldHint = styled.span`
  display: block;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 400;
`;

const Actions = styled.div`
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  flex-wrap: wrap;
`;

const Button = styled.button`
  min-height: 36px;
  padding: 0 12px;
  border-radius: 8px;
  border: 1px solid var(--service-accent);
  background: var(--service-accent);
  color: #fff;
  font-size: 0.875rem;
  font-weight: 600;

  &:hover:not(:disabled) {
    border-color: var(--service-accent-hover);
    background: var(--service-accent-hover);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.65;
  }

  @media (max-width: 640px) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

const ErrorText = styled.p`
  margin: 0;
  color: #ff8c85;
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
        <Field>
          Group name
          <Input
            type="text"
            name="group-name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            maxLength={100}
            required
            autoFocus
            placeholder="Team or workspace name"
          />
        </Field>
        <Field>
          Description
          <FieldHint>Optional context shown in the public group directory.</FieldHint>
          <Textarea
            name="group-description"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            maxLength={500}
            placeholder="What brings this group together?"
          />
        </Field>
        <CheckboxLabel>
          <input
            type="checkbox"
            name="group-public"
            checked={isPublic}
            onChange={(event) => setIsPublic(event.target.checked)}
          />
          <CheckboxCopy>
            <span>Make this group public</span>
            <FieldHint>Anyone can discover the group and view its ranking.</FieldHint>
          </CheckboxCopy>
        </CheckboxLabel>
        {error && <ErrorText role="alert">{error}</ErrorText>}
        <Actions>
          <SecondaryActionLink href="/leaderboard?view=groups">Cancel</SecondaryActionLink>
          <Button disabled={isSubmitting || !name.trim()} type="submit">
            {isSubmitting ? "Creating..." : "Create group"}
          </Button>
        </Actions>
      </Form>
    </Shell>
  );
}
