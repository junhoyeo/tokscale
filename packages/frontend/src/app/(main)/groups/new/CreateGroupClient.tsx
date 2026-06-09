"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";

const Shell = styled.section`
  max-width: 680px;
  margin: 32px 0;
`;

const Title = styled.h1`
  margin: 0 0 8px;
  color: var(--color-fg-default);
  font-size: 30px;
  font-weight: 700;
`;

const Description = styled.p`
  margin: 0 0 24px;
  color: var(--color-fg-muted);
  line-height: 1.6;
`;

const Form = styled.form`
  display: grid;
  gap: 16px;
  padding: 20px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-default);
`;

const Field = styled.label`
  display: grid;
  gap: 8px;
  color: var(--color-fg-default);
  font-size: 14px;
  font-weight: 600;
`;

const Input = styled.input`
  min-height: 40px;
  padding: 0 12px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
  color: var(--color-fg-default);
  font: inherit;
`;

const Textarea = styled.textarea`
  min-height: 96px;
  padding: 10px 12px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
  color: var(--color-fg-default);
  font: inherit;
  resize: vertical;
`;

const CheckboxLabel = styled.label`
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--color-fg-default);
  font-size: 14px;
`;

const Actions = styled.div`
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  flex-wrap: wrap;
`;

const Button = styled.button`
  min-height: 40px;
  padding: 0 16px;
  border-radius: 8px;
  border: 1px solid var(--color-primary);
  background: var(--color-primary);
  color: #fff;
  font-weight: 600;
  cursor: pointer;

  &:disabled {
    cursor: not-allowed;
    opacity: 0.65;
  }
`;

const SecondaryLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  min-height: 40px;
  padding: 0 16px;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  color: var(--color-fg-default);
  text-decoration: none;
`;

const ErrorText = styled.p`
  margin: 0;
  color: var(--color-danger-fg, #f85149);
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
            value={name}
            onChange={(event) => setName(event.target.value)}
            maxLength={100}
            required
            autoFocus
          />
        </Field>
        <Field>
          Description
          <Textarea
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            maxLength={500}
          />
        </Field>
        <CheckboxLabel>
          <input
            type="checkbox"
            checked={isPublic}
            onChange={(event) => setIsPublic(event.target.checked)}
          />
          Make this group public
        </CheckboxLabel>
        {error && <ErrorText>{error}</ErrorText>}
        <Actions>
          <SecondaryLink href="/groups">Cancel</SecondaryLink>
          <Button disabled={isSubmitting || !name.trim()} type="submit">
            {isSubmitting ? "Creating..." : "Create group"}
          </Button>
        </Actions>
      </Form>
    </Shell>
  );
}
