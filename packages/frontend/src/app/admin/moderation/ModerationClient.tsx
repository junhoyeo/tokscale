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
  latestActorUsername?: string | null;
  latestAction?: string | null;
  latestActionReason?: string | null;
}

/**
 * Fixed reasons rather than free text, and deliberately vague ones.
 *
 * An earlier draft spelled out which signal fired ("reports invented model
 * names", "token total matches another account"). That is a detection
 * playbook: anyone who reads it learns exactly what to avoid next time. These
 * strings are stored, and anything stored can end up shown, so they say
 * nothing about how the account was found.
 *
 * Nothing is lost by being vague. The review screen recomputes the signals
 * live from current data, so the specific evidence is always reconstructible —
 * the audit row only needs to record the decision and who made it.
 *
 * The one distinction kept is ours-versus-theirs. It leaks no method, and
 * without it a hide caused by our own ratchet inflation would be recorded
 * identically to genuine abuse.
 */
const HIDE_REASONS = [
  "Abuse",
  "Under investigation",
  "Data issue on our side",
] as const;

/**
 * Mirrors the ours-versus-theirs distinction above. A single "Restored" made
 * the audit trail lossy in exactly the direction that matters: clearing an
 * account we hid because of our own ratchet inflation looked identical to
 * clearing one a human reviewed and disagreed with. Both are the same button
 * press; only the record can tell them apart later.
 */
const UNHIDE_REASONS = [
  "Restored after review",
  "Restored — data issue on our side",
] as const;

export default function ModerationClient() {
  const [candidates, setCandidates] = useState<Candidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reasons, setReasons] = useState<Record<string, string>>({});
  const [pendingUser, setPendingUser] = useState<string | null>(null);

  const load = useCallback(async (signal?: AbortSignal) => {
    setError(null);
    try {
      const response = await fetch("/api/admin/moderation/candidates", { signal });
      if (!response.ok) {
        throw new Error("Failed to load candidates");
      }
      const data = await response.json();
      if (!signal?.aborted) {
        setCandidates(data.candidates ?? []);
      }
    } catch {
      if (!signal?.aborted) {
        setError("Could not load the review queue.");
        setCandidates([]);
      }
    }
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load]);

  async function applyAction(candidate: Candidate) {
    const reason = (reasons[candidate.username] ?? "").trim();

    // Enforced client-side as well as in the API so the reviewer is told what
    // is missing before a round trip, not after a 400.
    if (!reason) {
      setError(`Select a reason before changing @${candidate.username}.`);
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
          Hiding removes an account from both rankings and site-wide totals. Their
          profile, badge and embeds stay public. Every change is reversible and
          recorded.
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
            {candidates.map((candidate) => {
              const isSystemWithheld =
                candidate.leaderboardHidden &&
                candidate.latestActorUsername?.trim().toLowerCase() === "system" &&
                candidate.latestAction !== "unhide";
              const isAdminHidden =
                candidate.leaderboardHidden && !isSystemWithheld;

              return (
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
                    {isSystemWithheld ? (
                      <Badge
                        $variant="system"
                        role="status"
                        data-status="system-withheld"
                        aria-label="System Withheld"
                        title="Automatically withheld by system"
                      >
                        system withheld
                      </Badge>
                    ) : isAdminHidden ? (
                      <Badge
                        $variant="admin"
                        role="status"
                        data-status="admin-hidden"
                        aria-label="Admin Hidden"
                        title={
                          candidate.latestActorUsername
                            ? `Hidden manually by @${candidate.latestActorUsername}`
                            : "Hidden manually by administrator"
                        }
                      >
                        admin hidden
                      </Badge>
                    ) : null}
                    {candidate.hasBackfill ? <Badge $muted>backfill</Badge> : null}
                  </RowHead>

                  <Stats>
                    {formatCompact(candidate.totalTokens, "number")} tokens ·{" "}
                    {formatCompact(candidate.totalCost, "currency")} ·{" "}
                    {candidate.submitCount} submits
                  </Stats>

                  {candidate.leaderboardHidden ? (
                    <ModerationInfo $system={isSystemWithheld}>
                      <ModerationTag $system={isSystemWithheld}>
                        {isSystemWithheld ? "[System Withheld]" : "[Admin Hidden]"}
                      </ModerationTag>
                      <span>
                        {isSystemWithheld
                          ? "Automatically withheld by system"
                          : candidate.latestActorUsername
                            ? `Hidden manually by @${candidate.latestActorUsername}`
                            : "Hidden manually by administrator"}
                        {candidate.latestActionReason &&
                        candidate.latestAction !== "unhide" ? (
                          <>
                            {" "}
                            &mdash; Reason:{" "}
                            <ModerationReason>
                              &ldquo;{candidate.latestActionReason}&rdquo;
                            </ModerationReason>
                          </>
                        ) : null}
                      </span>
                    </ModerationInfo>
                  ) : null}

                  <Signals>
                    {candidate.signals.map((signal) => (
                      <li key={signal.key}>{signal.label}</li>
                    ))}
                  </Signals>

                <ActionRow>
                  <ReasonSelect
                    aria-label={`Reason for changing @${candidate.username}`}
                    value={reasons[candidate.username] ?? ""}
                    onChange={(event) =>
                      setReasons((current) => ({
                        ...current,
                        [candidate.username]: event.target.value,
                      }))
                    }
                  >
                    <option value="">Select a reason (recorded permanently)…</option>
                    {(candidate.leaderboardHidden ? UNHIDE_REASONS : HIDE_REASONS).map(
                      (reason) => (
                        <option key={reason} value={reason}>
                          {reason}
                        </option>
                      )
                    )}
                  </ReasonSelect>
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
            );
          })}
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

const Badge = styled.span<{
  $muted?: boolean;
  $variant?: "admin" | "system" | "muted";
}>`
  display: inline-flex;
  align-items: center;
  padding: 1px 7px;
  border-radius: 999px;
  font-size: 0.6875rem;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  font-weight: 550;
  border: 1px solid transparent;
  ${({ $muted, $variant }) => {
    if ($variant === "system") {
      return `
        background: rgba(245, 158, 11, 0.14);
        color: #f59e0b;
        border-color: rgba(245, 158, 11, 0.32);
      `;
    }
    if ($variant === "admin") {
      return `
        background: rgba(255, 107, 107, 0.14);
        color: #ff6b6b;
        border-color: rgba(255, 107, 107, 0.32);
      `;
    }
    if ($muted || $variant === "muted") {
      return `
        background: var(--service-surface-muted);
        color: var(--service-text-muted);
      `;
    }
    return `
      background: var(--service-accent-soft);
      color: var(--service-accent);
    `;
  }}
`;

const ModerationInfo = styled.div<{ $system?: boolean }>`
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
  padding: 8px 12px;
  border-radius: 8px;
  background: ${({ $system }) =>
    $system ? "rgba(245, 158, 11, 0.07)" : "rgba(255, 107, 107, 0.07)"};
  border: 1px solid
    ${({ $system }) =>
      $system ? "rgba(245, 158, 11, 0.2)" : "rgba(255, 107, 107, 0.2)"};
  color: var(--service-text);
  font-size: 0.8125rem;
  line-height: 1.4;
`;

const ModerationTag = styled.span<{ $system?: boolean }>`
  font-weight: 650;
  font-size: 0.75rem;
  font-family: var(--font-mono), monospace;
  color: ${({ $system }) => ($system ? "#f59e0b" : "#ff6b6b")};
`;

const ModerationReason = styled.span`
  color: var(--service-text-muted);
  font-style: italic;
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

const ReasonSelect = styled.select`
  flex: 1;
  min-width: 320px;
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
