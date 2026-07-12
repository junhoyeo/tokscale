"use client";

import { useEffect, useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { SecondaryActionLink } from "@/components/leaderboard/RankingUI";

interface InvitePreview {
  group: {
    name: string;
    slug: string;
    isPublic: boolean;
  };
  role: "admin" | "member";
  invitedUsername: string | null;
  expiresAt: string;
}

const Shell = styled.section`
  max-width: 620px;
  margin: 0 auto;
  padding: 20px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.5rem;
  font-weight: 600;
  letter-spacing: -0.02em;
  text-wrap: balance;
`;

const Text = styled.p`
  margin: 6px 0 16px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Meta = styled.dl`
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 8px 16px;
  margin: 18px 0;
  padding: 14px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
`;

const MetaLabel = styled.dt`
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 500;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const MetaValue = styled.dd`
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

const Actions = styled.div`
  display: flex;
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
    opacity: 0.65;
    cursor: not-allowed;
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

function formatRole(role: InvitePreview["role"]): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

export default function JoinGroupClient({ token }: { token: string }) {
  const router = useRouter();
  const [preview, setPreview] = useState<InvitePreview | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isJoining, setIsJoining] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const abortController = new AbortController();
    fetch(`/api/groups/join/${token}`, { signal: abortController.signal })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then(setPreview)
      .catch((err) => {
        if (err.name !== "AbortError") {
          setError("This invite is invalid or expired.");
        }
      })
      .finally(() => {
        if (!abortController.signal.aborted) {
          setIsLoading(false);
        }
      });

    return () => abortController.abort();
  }, [token]);

  async function acceptInvite() {
    setIsJoining(true);
    setError(null);

    try {
      const response = await fetch(`/api/groups/join/${token}`, { method: "POST" });
      const payload = await response.json();

      if (response.status === 401) {
        window.location.href = `/api/auth/github?returnTo=/groups/join/${token}`;
        return;
      }

      if (!response.ok) {
        throw new Error(payload.error || "Failed to join group");
      }

      router.push(`/groups/${payload.group.slug}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to join group");
      setIsJoining(false);
    }
  }

  if (isLoading) {
    return (
      <Shell>
        <Text>Loading invite...</Text>
      </Shell>
    );
  }

  if (!preview) {
    return (
      <Shell>
        <Title>Invite unavailable</Title>
        <ErrorText role="alert">{error || "This invite is invalid or expired."}</ErrorText>
        <Actions>
          <SecondaryActionLink href="/leaderboard?view=groups">Browse groups</SecondaryActionLink>
        </Actions>
      </Shell>
    );
  }

  return (
    <Shell>
      <Title>Join {preview.group.name}</Title>
      <Text>You were invited to join this group leaderboard.</Text>
      <Meta>
        <MetaLabel>Role</MetaLabel>
        <MetaValue>{formatRole(preview.role)}</MetaValue>
        <MetaLabel>Visibility</MetaLabel>
        <MetaValue>{preview.group.isPublic ? "Public" : "Private"}</MetaValue>
        {preview.invitedUsername && (
          <>
            <MetaLabel>Invited account</MetaLabel>
            <MetaValue>@{preview.invitedUsername}</MetaValue>
          </>
        )}
        <MetaLabel>Expires</MetaLabel>
        <MetaValue>
          {new Date(preview.expiresAt).toLocaleString(undefined, {
            dateStyle: "medium",
            timeStyle: "short",
          })}
        </MetaValue>
      </Meta>
      {error && <ErrorText role="alert">{error}</ErrorText>}
      <Actions>
        <Button type="button" onClick={acceptInvite} disabled={isJoining}>
          {isJoining ? "Joining..." : "Join group"}
        </Button>
        <SecondaryActionLink href="/leaderboard?view=groups">Cancel</SecondaryActionLink>
      </Actions>
    </Shell>
  );
}
