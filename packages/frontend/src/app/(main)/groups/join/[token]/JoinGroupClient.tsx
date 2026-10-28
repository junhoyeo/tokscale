"use client";

import { useEffect, useState } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { DitherAvatar } from "dither-kit/DitherAvatar";
import { PrimaryButton, SecondaryActionLink } from "@/components/leaderboard/RankingUI";

interface InvitePreview {
  group: {
    name: string;
    slug: string;
    isPublic: boolean;
  };
  role: "admin" | "member";
  invitedUsername: string | null;
  expiresAt: string;
  remainingUses: number;
}

const Shell = styled.section`
  max-width: 600px;
  margin: 0 auto;
`;

const Identity = styled.div`
  display: flex;
  align-items: flex-start;
  gap: 16px;
  margin-bottom: 16px;
`;

const AvatarFrame = styled.span`
  display: grid;
  width: 48px;
  height: 48px;
  flex: 0 0 auto;
  overflow: hidden;
  margin-top: 2px;
  border-radius: var(--radius, 2px);
  background: var(--service-surface-muted);
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

const Text = styled.p`
  margin: 6px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

/* The invite's terms as a divided key-value strip: label above value. */
const Meta = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
  margin: 20px 0;
  padding: 16px 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);

  /* Terms are short except the expiry date, so the columns take the width
   * they need instead of splitting the strip into four equal parts. */
  @media (min-width: 560px) {
    grid-template-columns: repeat(4, max-content);
    column-gap: 40px;
  }
`;

const MetaItem = styled.div`
  min-width: 0;
`;

const MetaLabel = styled.dt`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const MetaValue = styled.dd`
  margin: 4px 0 0;
  overflow-wrap: anywhere;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-variant-numeric: tabular-nums;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;
`;

const ErrorText = styled.p`
  margin: 0 0 16px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
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
        <Text>Loading invite…</Text>
      </Shell>
    );
  }

  if (!preview) {
    return (
      <Shell>
        <Title>Invite unavailable</Title>
        {error && <ErrorText role="alert">{error}</ErrorText>}
        <Actions style={{ marginTop: 16 }}>
          <SecondaryActionLink href="/leaderboard?view=groups">Browse groups</SecondaryActionLink>
        </Actions>
      </Shell>
    );
  }

  return (
    <Shell>
      <Identity>
        <AvatarFrame aria-hidden="true">
          <DitherAvatar
            name={preview.group.name}
            color="blue"
            size={48}
            grid={8}
            cellPx={6}
            density={-0.1}
            animate={false}
            bloom="off"
          />
        </AvatarFrame>
        <div>
          <Title>Join {preview.group.name}</Title>
          <Text>You were invited to join this group leaderboard. Each account can use this link once.</Text>
        </div>
      </Identity>
      <Meta>
        <MetaItem>
          <MetaLabel>Role</MetaLabel>
          <MetaValue>{formatRole(preview.role)}</MetaValue>
        </MetaItem>
        <MetaItem>
          <MetaLabel>Visibility</MetaLabel>
          <MetaValue>{preview.group.isPublic ? "Public" : "Private"}</MetaValue>
        </MetaItem>
        {preview.invitedUsername && (
          <MetaItem>
            <MetaLabel>Invited account</MetaLabel>
            <MetaValue>@{preview.invitedUsername}</MetaValue>
          </MetaItem>
        )}
        <MetaItem>
          <MetaLabel>Joins remaining</MetaLabel>
          <MetaValue>{preview.remainingUses}</MetaValue>
        </MetaItem>
        <MetaItem>
          <MetaLabel>Expires</MetaLabel>
          <MetaValue>
            {new Date(preview.expiresAt).toLocaleString(undefined, {
              dateStyle: "medium",
              timeStyle: "short",
            })}
          </MetaValue>
        </MetaItem>
      </Meta>
      {error && <ErrorText role="alert">{error}</ErrorText>}
      <Actions>
        <PrimaryButton type="button" onClick={acceptInvite} disabled={isJoining}>
          {isJoining ? "Joining…" : "Join group"}
        </PrimaryButton>
        <SecondaryActionLink href="/leaderboard?view=groups">Cancel</SecondaryActionLink>
      </Actions>
    </Shell>
  );
}
