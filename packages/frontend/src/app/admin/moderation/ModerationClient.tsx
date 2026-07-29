"use client";

import { useCallback, useEffect, useState } from "react";
import styled from "styled-components";
import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import { formatCompact } from "@/lib/format";
import type { CandidateSignal } from "@/lib/moderation/heuristics";

interface Candidate {
  userId: string;
  username: string;
  leaderboardHidden: boolean;
  totalTokens: number;
  totalCost: number;
  submitCount: number;
  hasBackfill: boolean;
  score: number;
  signals: CandidateSignal[];
}

export default function ModerationClient() {
  const [candidates, setCandidates] = useState<Candidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reasons, setReasons] = useState<Record<string, string>>({});
  const [pendingUser, setPendingUser] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const response = await fetch("/api/admin/moderation/candidates");
      if (!response.ok) {
        throw new Error("Failed to load candidates");
      }
      const data = await response.json();
      setCandidates(data.candidates ?? []);
    } catch {
      setError("Could not load the review queue.");
      setCandidates([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function applyAction(candidate: Candidate) {
    const reason = (reasons[candidate.username] ?? "").trim();

    // Enforced client-side as well as in the API so the reviewer is told what
    // is missing before a round trip, not after a 400.
    if (!reason) {
      setError(`A reason is required before changing @${candidate.username}.`);
      return;
    }

    setPendingUser(candidate.username);
    setError(null);

    try {
      const response = await fetch(
        `/api/admin/moderation/${encodeURIComponent(candidate.username)}`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            action: candidate.leaderboardHidden ? "unhide" : "hide",
            reason,
          }),
        }
      );

      if (!response.ok) {
        const body = await response.json().catch(() => null);
        throw new Error(body?.error ?? "Request failed");
      }

      setReasons((current) => ({ ...current, [candidate.username]: "" }));
      await load();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Request failed");
    } finally {
      setPendingUser(null);
    }
  }

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <Title>Leaderboard moderation</Title>
        <Intro>
          Hiding removes an account from the rankings only. Their profile, badge
          and embeds stay public, and their usage still counts toward site-wide
          totals. Every change is reversible and recorded.
        </Intro>
        <Caveat>
          A high score is a prompt to investigate, not a verdict. In particular,
          a stored total far above the sum of daily rows is the signature of
          ratchet inflation (#960) — our bug, not the user&apos;s.
        </Caveat>

        {error ? <ErrorText role="alert">{error}</ErrorText> : null}

        {candidates === null ? (
          <Muted>Loading…</Muted>
        ) : candidates.length === 0 ? (
          <Muted>Nothing flagged, and nobody currently hidden.</Muted>
        ) : (
          <List>
            {candidates.map((candidate) => (
              <Row key={candidate.userId} $hidden={candidate.leaderboardHidden}>
                <RowHead>
                  <Handle
                    href={`/u/${candidate.username}`}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    @{candidate.username}
                  </Handle>
                  <Score>score {Math.round(candidate.score)}</Score>
                  {candidate.leaderboardHidden ? <Badge>hidden</Badge> : null}
                  {candidate.hasBackfill ? <Badge $muted>backfill</Badge> : null}
                </RowHead>

                <Stats>
                  {formatCompact(candidate.totalTokens, "number")} tokens ·{" "}
                  {formatCompact(candidate.totalCost, "currency")} ·{" "}
                  {candidate.submitCount} submits
                </Stats>

                <Signals>
                  {candidate.signals.map((signal) => (
                    <li key={signal.key}>{signal.label}</li>
                  ))}
                </Signals>

                <ActionRow>
                  <ReasonInput
                    value={reasons[candidate.username] ?? ""}
                    placeholder="Reason (recorded permanently)"
                    onChange={(event) =>
                      setReasons((current) => ({
                        ...current,
                        [candidate.username]: event.target.value,
                      }))
                    }
                  />
                  <ActionButton
                    type="button"
                    $danger={!candidate.leaderboardHidden}
                    disabled={pendingUser === candidate.username}
                    onClick={() => void applyAction(candidate)}
                  >
                    {pendingUser === candidate.username
                      ? "Working…"
                      : candidate.leaderboardHidden
                        ? "Unhide"
                        : "Hide from rankings"}
                  </ActionButton>
                </ActionRow>
              </Row>
            ))}
          </List>
        )}
      </main>
      <ServiceFooter />
    </div>
  );
}

const Title = styled.h1`
  margin: 0 0 8px;
  font-size: 1.75rem;
  font-weight: 650;
  letter-spacing: -0.02em;
`;

const Intro = styled.p`
  max-width: 720px;
  margin: 0 0 8px;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
  line-height: 1.65;
`;

const Caveat = styled(Intro)`
  margin-bottom: 32px;
  color: var(--service-text-muted);
  font-style: italic;
`;

const Muted = styled.p`
  color: var(--service-text-muted);
  font-size: 0.9375rem;
`;

const ErrorText = styled.p`
  margin: 0 0 16px;
  color: #ff6b6b;
  font-size: 0.875rem;
`;

const List = styled.div`
  display: flex;
  flex-direction: column;
  gap: 12px;
`;

const Row = styled.div<{ $hidden: boolean }>`
  padding: 20px;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
  opacity: ${({ $hidden }) => ($hidden ? 0.62 : 1)};
`;

const RowHead = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
`;

const Handle = styled.a`
  color: var(--service-text);
  font-size: 1.0625rem;
  font-weight: 600;
  text-decoration: none;

  &:hover {
    text-decoration: underline;
  }
`;

const Score = styled.span`
  color: var(--service-text-muted);
  font-family: var(--font-mono), monospace;
  font-size: 0.75rem;
`;

const Badge = styled.span<{ $muted?: boolean }>`
  padding: 2px 8px;
  border-radius: 999px;
  background: ${({ $muted }) =>
    $muted ? "var(--service-surface-muted)" : "var(--service-accent-soft)"};
  color: ${({ $muted }) => ($muted ? "var(--service-text-muted)" : "var(--service-accent)")};
  font-size: 0.6875rem;
  text-transform: uppercase;
  letter-spacing: 0.06em;
`;

const Stats = styled.p`
  margin: 10px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
`;

const Signals = styled.ul`
  margin: 12px 0 0;
  padding-left: 18px;

  li {
    color: var(--service-text-muted);
    font-size: 0.875rem;
    line-height: 1.6;
  }
`;

const ActionRow = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  margin-top: 16px;
`;

const ReasonInput = styled.input`
  flex: 1;
  min-width: 240px;
  padding: 9px 12px;
  border: 1px solid var(--service-border);
  border-radius: 8px;
  background: var(--service-canvas);
  color: var(--service-text);
  font-size: 0.875rem;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }
`;

const ActionButton = styled.button<{ $danger: boolean }>`
  padding: 9px 16px;
  border: 1px solid
    ${({ $danger }) => ($danger ? "rgba(255, 107, 107, 0.5)" : "var(--service-border-strong)")};
  border-radius: 8px;
  background: transparent;
  color: ${({ $danger }) => ($danger ? "#ff6b6b" : "var(--service-text)")};
  font-size: 0.875rem;
  font-weight: 550;
  cursor: pointer;

  &:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
`;
