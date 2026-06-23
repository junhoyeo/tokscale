"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";

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
  margin: 48px auto;
  padding: 24px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-default);
`;

const Title = styled.h1`
  margin: 0 0 8px;
  color: var(--color-fg-default);
  font-size: 28px;
  font-weight: 700;
`;

const Text = styled.p`
  margin: 0 0 16px;
  color: var(--color-fg-muted);
  line-height: 1.6;
`;

const Meta = styled.div`
  display: grid;
  gap: 8px;
  margin: 18px 0;
  color: var(--color-fg-muted);
  font-size: 14px;
`;

const Actions = styled.div`
  display: flex;
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
    opacity: 0.65;
    cursor: not-allowed;
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
        <ErrorText>{error || "This invite is invalid or expired."}</ErrorText>
        <Actions>
          <SecondaryLink href="/groups">Browse groups</SecondaryLink>
        </Actions>
      </Shell>
    );
  }

  return (
    <Shell>
      <Title>Join {preview.group.name}</Title>
      <Text>You were invited to join this group leaderboard.</Text>
      <Meta>
        <span>Role: {formatRole(preview.role)}</span>
        <span>Visibility: {preview.group.isPublic ? "Public" : "Private"}</span>
        {preview.invitedUsername && <span>For: @{preview.invitedUsername}</span>}
      </Meta>
      {error && <ErrorText>{error}</ErrorText>}
      <Actions>
        <Button onClick={acceptInvite} disabled={isJoining}>
          {isJoining ? "Joining..." : "Join group"}
        </Button>
        <SecondaryLink href="/groups">Cancel</SecondaryLink>
      </Actions>
    </Shell>
  );
}
