"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";
import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import { Select } from "@/components/ui/Select";
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

interface HistoryEntry {
  id: string;
  action: "hide" | "unhide";
  reason: string;
  createdAt: string;
  actorUsername: string;
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
 * live from current data, so the specific evidence is always reconstructible:
 * the audit row only needs to record the decision and who made it.
 *
 * The one distinction kept is ours-versus-theirs. It leaks no method, and
 * without it a hide caused by our own ratchet inflation would be recorded
 * identically to genuine abuse.
 */
const HIDE_REASONS = ["Abuse", "Under investigation", "Data issue on our side"] as const;

/**
 * Mirrors the ours-versus-theirs distinction above. A single "Restored" made
 * the audit trail lossy in exactly the direction that matters: clearing an
 * account we hid because of our own ratchet inflation looked identical to
 * clearing one a human reviewed and disagreed with. Both are the same button
 * press; only the record can tell them apart later.
 */
const UNHIDE_REASONS = ["Restored after review", "Restored — data issue on our side"] as const;

const actionDateFormatter = new Intl.DateTimeFormat("en-US", {
  dateStyle: "medium",
  timeStyle: "short",
});

export default function ModerationClient() {
  const [candidates, setCandidates] = useState<Candidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reasons, setReasons] = useState<Record<string, string>>({});
  const [pendingUser, setPendingUser] = useState<string | null>(null);
  const [selectedUsername, setSelectedUsername] = useState<string | null>(null);
  const [history, setHistory] = useState<HistoryEntry[] | null>(null);
  const [historyError, setHistoryError] = useState(false);
  // On a phone the queue and the detail are one region at a time: choosing a
  // row pushes the detail, and a back control returns to the list.
  const [detailOpen, setDetailOpen] = useState(false);

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

  const selected = useMemo(
    () => candidates?.find((candidate) => candidate.username === selectedUsername) ?? null,
    [candidates, selectedUsername],
  );

  // Start on the highest-scoring candidate, and fall back to it when the
  // selected account leaves the queue after a change.
  useEffect(() => {
    if (!candidates || candidates.length === 0) {
      setSelectedUsername(null);
      return;
    }
    if (!candidates.some((candidate) => candidate.username === selectedUsername)) {
      setSelectedUsername(candidates[0].username);
    }
  }, [candidates, selectedUsername]);

  useEffect(() => {
    if (!selectedUsername) {
      setHistory(null);
      return;
    }
    const controller = new AbortController();
    setHistory(null);
    setHistoryError(false);
    fetch(`/api/admin/moderation/${encodeURIComponent(selectedUsername)}`, { signal: controller.signal })
      .then((response) => (response.ok ? response.json() : Promise.reject(new Error("history"))))
      .then((data) => {
        if (!controller.signal.aborted) setHistory(Array.isArray(data.history) ? data.history : []);
      })
      .catch(() => {
        if (!controller.signal.aborted) setHistoryError(true);
      });
    return () => controller.abort();
  }, [selectedUsername]);

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
      const response = await fetch(`/api/admin/moderation/${encodeURIComponent(candidate.username)}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          action: candidate.leaderboardHidden ? "unhide" : "hide",
          reason,
        }),
      });

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

  const reasonOptions = selected
    ? (selected.leaderboardHidden ? UNHIDE_REASONS : HIDE_REASONS).map((reason) => ({
        value: reason,
        label: reason,
      }))
    : [];

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <PageHead>
          <Title>Leaderboard moderation</Title>
          <Lead>
            Hiding removes an account from the rankings only. Their profile, badge and embeds stay
            public, and their usage still counts toward site-wide totals. Every change is reversible
            and recorded with its reason.
          </Lead>
          <Caveat>
            A high score is a prompt to investigate, not a verdict. In particular, a stored total far
            above the sum of its daily rows is the signature of ratchet inflation (#960), our bug
            rather than the user&apos;s.
          </Caveat>
        </PageHead>

        {error ? <ErrorText role="alert">{error}</ErrorText> : null}

        {candidates === null ? (
          <Muted>Loading…</Muted>
        ) : candidates.length === 0 ? (
          <Muted>Nothing flagged, and nobody currently hidden.</Muted>
        ) : (
          <Workspace>
            <Queue aria-label="Review queue" $detailOpen={detailOpen}>
              <QueueHead>
                <QueueTitle>Queue</QueueTitle>
                <QueueCount>
                  {candidates.length} {candidates.length === 1 ? "account" : "accounts"}
                </QueueCount>
              </QueueHead>
              <QueueList>
                {candidates.map((candidate) => (
                  <QueueRow
                    key={candidate.userId}
                    type="button"
                    $selected={candidate.username === selectedUsername}
                    aria-pressed={candidate.username === selectedUsername}
                    onClick={() => {
                      setSelectedUsername(candidate.username);
                      setDetailOpen(true);
                    }}
                  >
                    <QueueName>@{candidate.username}</QueueName>
                    <QueueFigures>
                      <span>{Math.round(candidate.score)} score</span>
                      <span>{formatCompact(candidate.totalTokens, "number")} tokens</span>
                      <span>{formatCompact(candidate.totalCost, "currency")}</span>
                    </QueueFigures>
                    {candidate.leaderboardHidden ? <StatusMark>Hidden</StatusMark> : null}
                  </QueueRow>
                ))}
              </QueueList>
            </Queue>

            {selected ? (
              <Detail aria-label={`Review ${selected.username}`} $detailOpen={detailOpen}>
                <BackButton type="button" onClick={() => setDetailOpen(false)}>
                  Back to the queue
                </BackButton>
                <DetailHead>
                  <div>
                    <DetailTitle>
                      <Handle href={`/u/${selected.username}`} target="_blank" rel="noopener noreferrer">
                        @{selected.username}
                      </Handle>
                    </DetailTitle>
                    <DetailMeta>
                      Score {Math.round(selected.score)}
                      {selected.hasBackfill ? " · Backfill submitted" : ""}
                      {selected.leaderboardHidden ? " · Hidden from rankings" : ""}
                    </DetailMeta>
                  </div>
                </DetailHead>

                <FigureStrip>
                  <Figure>
                    <dt>Tokens</dt>
                    <dd>{formatCompact(selected.totalTokens, "number")}</dd>
                  </Figure>
                  <Figure>
                    <dt>Cost</dt>
                    <dd>{formatCompact(selected.totalCost, "currency")}</dd>
                  </Figure>
                  <Figure>
                    <dt>Submits</dt>
                    <dd>{selected.submitCount.toLocaleString("en-US")}</dd>
                  </Figure>
                </FigureStrip>

                <Block>
                  <BlockTitle>Signals</BlockTitle>
                  {selected.signals.length === 0 ? (
                    <Muted>No signal fired on the current data.</Muted>
                  ) : (
                    <SignalList>
                      {selected.signals.map((signal) => (
                        <li key={signal.key}>
                          <SignalLabel>{signal.label}</SignalLabel>
                          <SignalWeight>weight {signal.weight}</SignalWeight>
                        </li>
                      ))}
                    </SignalList>
                  )}
                </Block>

                <Block>
                  <BlockTitle>Recorded decisions</BlockTitle>
                  {historyError ? (
                    <Muted>Could not load the decision history.</Muted>
                  ) : history === null ? (
                    <Muted>Loading…</Muted>
                  ) : history.length === 0 ? (
                    <Muted>No decision has been recorded for this account.</Muted>
                  ) : (
                    <HistoryList>
                      {history.map((entry) => (
                        <li key={entry.id}>
                          <HistoryAction $hide={entry.action === "hide"}>
                            {entry.action === "hide" ? "Hidden" : "Restored"}
                          </HistoryAction>
                          <HistoryReason>{entry.reason}</HistoryReason>
                          <HistoryMeta>
                            @{entry.actorUsername} · {actionDateFormatter.format(new Date(entry.createdAt))}
                          </HistoryMeta>
                        </li>
                      ))}
                    </HistoryList>
                  )}
                </Block>

                <Block>
                  <BlockTitle>Decision</BlockTitle>
                  <DecisionRow>
                    <ReasonSelect
                      aria-label={`Reason for changing @${selected.username}`}
                      placeholder="Select a reason"
                      value={reasons[selected.username] ?? ""}
                      options={reasonOptions}
                      onChange={(next) =>
                        setReasons((current) => ({ ...current, [selected.username]: next }))
                      }
                    />
                    <DitherButton
                      color={selected.leaderboardHidden ? "blue" : "red"}
                      variant="solid"
                      disabled={pendingUser === selected.username || !(reasons[selected.username] ?? "").trim()}
                      onClick={() => void applyAction(selected)}
                      style={{ minHeight: 40, padding: "0 16px", borderRadius: 2, fontSize: "0.8125rem" }}
                    >
                      {pendingUser === selected.username
                        ? "Working…"
                        : selected.leaderboardHidden
                          ? "Restore to rankings"
                          : "Hide from rankings"}
                    </DitherButton>
                  </DecisionRow>
                  <Hint>
                    The reason is recorded permanently and shown in the history above, so write it
                    for the next reviewer.
                  </Hint>
                </Block>
              </Detail>
            ) : null}
          </Workspace>
        )}
      </main>
      <ServiceFooter />
    </div>
  );
}

const PageHead = styled.header`
  max-width: 760px;
  margin-bottom: 32px;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.75rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
`;

const Lead = styled.p`
  margin: 12px 0 0;
  color: var(--service-text-muted);
  font-size: 1rem;
  line-height: 1.6;
`;

const Caveat = styled.p`
  margin: 12px 0 0;
  padding: 12px 14px;
  border: 1px solid var(--service-border);
  border-radius: 2px;
  background: var(--service-surface);
  color: var(--service-text);
  font-size: 0.875rem;
  line-height: 1.6;
`;

const Muted = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.6;
`;

const ErrorText = styled.p`
  margin: 0 0 16px;
  color: #ff9b9b;
  font-size: 0.875rem;
  line-height: 1.5;
`;

/* List on the left, the selected account's evidence and decision on the right. */
const Workspace = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 400px) minmax(0, 1fr);
  align-items: start;
  gap: 32px;

  @media (max-width: 1024px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const Queue = styled.section<{ $detailOpen: boolean }>`
  min-width: 0;

  /* A phone works one region at a time: choosing a row pushes the detail. */
  @media (max-width: 1024px) {
    display: ${({ $detailOpen }) => ($detailOpen ? "none" : "block")};
  }
`;

const QueueHead = styled.div`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding-bottom: 12px;
`;

const QueueTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
`;

const QueueCount = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
`;

const QueueList = styled.div`
  display: grid;
  border-top: 1px solid var(--service-border);
`;

/* The selected row carries a 1px accent line on its leading edge and a raised
   fill, the same mark the site uses for every selection. */
const QueueRow = styled.button<{ $selected: boolean }>`
  display: grid;
  gap: 4px;
  min-width: 0;
  padding: 12px 14px;
  border: 0;
  border-left: 1px solid ${({ $selected }) => ($selected ? "var(--service-accent)" : "transparent")};
  border-bottom: 1px solid var(--service-border);
  background: ${({ $selected }) => ($selected ? "var(--service-surface-raised)" : "transparent")};
  color: var(--service-text);
  text-align: left;
  font-variant-numeric: tabular-nums;

  &:hover {
    background: var(--service-surface-raised);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: -2px;
  }
`;

const QueueName = styled.span`
  overflow: hidden;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const QueueFigures = styled.span`
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;

`;

const StatusMark = styled.span`
  justify-self: start;
  color: var(--service-text-muted);
  font-size: 0.75rem;
`;

/* Only a phone needs the way back; on a wide screen both panes are on screen. */
const BackButton = styled.button`
  display: none;
  justify-self: start;
  min-height: 32px;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;

  &:hover {
    color: var(--service-text);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (max-width: 1024px) {
    display: inline-flex;
    align-items: center;
    min-height: 44px;
  }
`;

const Detail = styled.section<{ $detailOpen: boolean }>`
  display: grid;
  min-width: 0;
  gap: 24px;
  padding: 20px;
  border: 1px solid var(--service-border);
  border-radius: 2px;
  background: var(--service-surface);
  font-variant-numeric: tabular-nums;

  @media (max-width: 1024px) {
    display: ${({ $detailOpen }) => ($detailOpen ? "grid" : "none")};
  }

  @media (max-width: 520px) {
    padding: 16px;
  }
`;

const DetailHead = styled.div`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
`;

const DetailTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1.25rem;
  font-weight: 500;
  letter-spacing: -0.01em;
`;

const Handle = styled.a`
  color: var(--service-text);
  text-decoration: none;

  &:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 3px;
    border-radius: 2px;
  }
`;

const DetailMeta = styled.p`
  margin: 4px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const FigureStrip = styled.dl`
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  margin: 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);

  @media (max-width: 520px) {
    padding: 0;
  }
`;

const Figure = styled.div`
  min-width: 0;
  padding: 10px 12px 10px 0;

  & + & {
    padding-left: 12px;
    border-left: 1px solid var(--service-border);
  }

  dt {
    color: var(--service-text-muted);
    font-size: 0.8125rem;
  }

  dd {
    margin: 2px 0 0;
    color: var(--service-text);
    font-size: 1.25rem;
    font-weight: 500;
  }

  @media (max-width: 420px) {
    dd {
      font-size: 1.0625rem;
    }
  }
`;

const Block = styled.div`
  display: grid;
  gap: 8px;
  min-width: 0;
`;

const BlockTitle = styled.h3`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
`;

const SignalList = styled.ul`
  display: grid;
  margin: 0;
  padding: 0;
  list-style: none;

  li {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 0;
    border-bottom: 1px solid var(--service-border);
    font-size: 0.875rem;
  }

  li:last-child {
    border-bottom: 0;
  }
`;

const SignalLabel = styled.span`
  color: var(--service-text);
  line-height: 1.5;
`;

const SignalWeight = styled.span`
  flex: 0 0 auto;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const HistoryList = styled.ol`
  display: grid;
  margin: 0;
  padding: 0;
  list-style: none;

  li {
    display: grid;
    gap: 2px;
    padding: 10px 0;
    border-bottom: 1px solid var(--service-border);
  }

  li:last-child {
    border-bottom: 0;
  }
`;

const HistoryAction = styled.span<{ $hide: boolean }>`
  color: ${({ $hide }) => ($hide ? "#ff9b9b" : "var(--service-text)")};
  font-size: 0.875rem;
  font-weight: 500;
`;

const HistoryReason = styled.span`
  color: var(--service-text);
  font-size: 0.875rem;
  line-height: 1.5;
`;

const HistoryMeta = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const DecisionRow = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
`;

const ReasonSelect = styled(Select)`
  --select-height: 40px;
  --select-radius: 2px;
  --select-background: var(--service-surface-muted);
  --select-border-color: var(--service-border-strong);
  --select-font-size: 0.875rem;
`;

const Hint = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
`;
