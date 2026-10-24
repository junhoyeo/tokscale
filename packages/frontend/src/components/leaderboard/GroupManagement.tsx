"use client";

import { useEffect, useRef, useState } from "react";
import Image from "next/image";
import styled from "styled-components";
import { CompactBadge, SecondaryButton } from "./RankingUI";
import { getGroupMonogram } from "./presentation";
import { Select } from "@/components/ui/Select";
import { canManageGroupRole } from "@/lib/groups/roles";
import type { GroupRole } from "@/lib/db/schema";

interface Member {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  role: GroupRole;
}

interface GroupManagementProps {
  slug: string;
  userId: string;
  role: GroupRole;
  onChanged: () => Promise<void>;
}

const Panel = styled.section`
  display: grid;
  gap: 16px;
  margin-top: 24px;
  padding: 20px;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
  h2, p { margin: 0; }
  h2 { font-size: 0.9375rem; font-weight: 600; }
  p { color: var(--service-text-muted); font-size: 0.875rem; line-height: 1.6; }
`;

const Controls = styled.div`
  display: flex;
  max-width: 100%;
  flex-wrap: wrap;
  align-items: end;
  gap: 12px;
  button { max-width: 100%; white-space: normal; overflow-wrap: anywhere; }
`;

const MemberHeading = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
`;

const MemberColumns = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) 112px 44px;
  gap: 16px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--service-border);
  color: var(--service-text-muted);
  font-size: 0.75rem;

  @media (max-width: 520px) {
    grid-template-columns: minmax(0, 1fr) 88px 36px;
    gap: 8px;
  }
`;

const MemberRow = styled.li`
  display: grid;
  grid-template-columns: minmax(0, 1fr) 112px 44px;
  align-items: center;
  gap: 16px;
  min-height: 72px;
  padding: 12px 0;
  border-bottom: 1px solid var(--service-border);
  &:last-child { border-bottom: 0; }

  @media (max-width: 520px) {
    grid-template-columns: minmax(0, 1fr) 88px 36px;
    gap: 8px;
  }
`;

const MemberIdentity = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 12px;
  > div { min-width: 0; }
  strong, small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  strong { color: var(--service-text); font-size: 0.875rem; font-weight: 500; }
  small { margin-top: 4px; color: var(--service-text-muted); font-size: 0.75rem; }

  @media (max-width: 520px) { gap: 8px; }
`;

const MemberAvatar = styled.span`
  display: grid;
  width: 36px;
  height: 36px;
  flex: 0 0 auto;
  place-items: center;
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 50%;
  background: var(--service-surface-muted);
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-weight: 600;
  img { width: 100%; height: 100%; object-fit: cover; }

  @media (max-width: 520px) { width: 28px; height: 28px; }
`;

const RoleBadge = styled(CompactBadge)`
  justify-self: start;
  text-transform: capitalize;
`;

const ROLE_OPTIONS = [
  { value: "member", label: "Member" },
  { value: "admin", label: "Admin" },
];

const RoleSelect = styled(Select)`
  --select-width: 100%;
  --select-height: 36px;
  --select-padding-x: 8px;
  --select-font-size: 0.8125rem;
  --select-radius: 6px;
  --select-background: var(--service-surface);
  --select-border-color: var(--service-border);
`;

const MemberMenuButton = styled.button`
  display: grid;
  width: 36px;
  height: 36px;
  place-items: center;
  justify-self: end;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--service-text-muted);
  cursor: pointer;
  &:hover { border-color: var(--service-border); background: var(--service-surface-muted); color: var(--service-text); }
  &:focus-visible { outline: 2px solid var(--service-focus); outline-offset: 2px; }
  &:disabled { cursor: default; opacity: 0.5; }
  @media (pointer: coarse) { height: 44px; }
`;

const MemberActions = styled.div`
  position: fixed;
  inset: auto;
  width: 224px;
  max-width: calc(100vw - 32px);
  margin: 0;
  padding: 6px;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: var(--service-surface);
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.35);
`;

const MemberAction = styled.button<{ $danger?: boolean }>`
  display: block;
  width: 100%;
  min-height: 44px;
  padding: 10px 12px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: ${({ $danger }) => $danger ? "#f85149" : "var(--service-text)"};
  font-size: 0.8125rem;
  text-align: left;
  cursor: pointer;
  &:hover, &:focus-visible { background: var(--service-surface-muted); }
  &:focus-visible { outline: 2px solid var(--service-focus); outline-offset: -2px; }
`;

const Confirmation = styled.dialog`
  max-width: min(480px, calc(100vw - 32px));
  padding: 24px;
  border: 1px solid var(--service-border-strong);
  border-radius: 12px;
  background: var(--service-surface);
  color: var(--service-text);
  &::backdrop { background: rgba(0, 0, 0, 0.65); }
  h3 { margin-top: 0; }
  p { margin-bottom: 20px; }
`;

export function GroupManagement({ slug, userId, role, onChanged }: GroupManagementProps) {
  const [members, setMembers] = useState<Member[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<{ member: Member; action: "remove" | "transfer" } | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  const menuScrollPosition = useRef({ x: 0, y: 0 });

  useEffect(() => {
    const closeMenu = (event: Event) => {
      if (event.type === "scroll" && window.scrollX === menuScrollPosition.current.x && window.scrollY === menuScrollPosition.current.y) return;
      panelRef.current?.querySelector<HTMLElement>("[popover]:popover-open")?.hidePopover();
    };
    window.addEventListener("scroll", closeMenu, true);
    window.addEventListener("resize", closeMenu);
    return () => {
      window.removeEventListener("scroll", closeMenu, true);
      window.removeEventListener("resize", closeMenu);
    };
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    fetch(`/api/groups/${slug}/members`, { signal: controller.signal, cache: "no-store" })
      .then(async (response) => {
        const payload = await response.json();
        if (!response.ok) throw new Error(payload.error || "Failed to load members");
        setMembers(payload.members);
        setLoadError(null);
      })
      .catch((err) => {
        if (!controller.signal.aborted) setLoadError(err instanceof Error ? err.message : "Failed to load members");
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [slug, reload]);

  useEffect(() => {
    if (confirmation && !dialogRef.current?.open) dialogRef.current?.showModal();
  }, [confirmation]);

  function closeConfirmation() {
    dialogRef.current?.close();
    setConfirmation(null);
  }

  async function mutate(path: string, method: string, body?: object): Promise<boolean> {
    if (pendingRef.current) return false;
    pendingRef.current = true;
    setPending(true);
    setError(null);
    setNotice(null);
    try {
      const response = await fetch(`/api/groups/${slug}${path}`, {
        method,
        headers: body ? { "Content-Type": "application/json" } : undefined,
        body: body ? JSON.stringify(body) : undefined,
      });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error || "Could not update group");
      setNotice("Changes saved.");
      setReload((value) => value + 1);
      try {
        await onChanged();
      } catch {
        setError("Changes were saved, but the group could not be refreshed. Reload this page before making another change.");
      }
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not update group");
      return false;
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }

  async function confirmAction() {
    if (!confirmation) return;
    const { member, action } = confirmation;
    const saved = action === "remove"
      ? await mutate(`/members?userId=${encodeURIComponent(member.userId)}`, "DELETE")
      : await mutate("/transfer-ownership", "POST", { targetUserId: member.userId });
    if (saved) closeConfirmation();
  }

  return (
    <Panel ref={panelRef} aria-label="Group management">
      <div>
        <MemberHeading>
          <h2>Members</h2>
          {!loading && !loadError && <CompactBadge>{members.length}</CompactBadge>}
        </MemberHeading>
        {loading && <p role="status">Loading members...</p>}
        {loadError && <div role="alert">{loadError} <SecondaryButton type="button" disabled={pending} onClick={() => { setLoading(true); setReload((value) => value + 1); }}>Retry</SecondaryButton></div>}
        {!loading && !loadError && <>
          <MemberColumns aria-hidden="true"><span>Member</span><span>Role</span><span /></MemberColumns>
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {members.map((member) => {
              const canManage = member.userId !== userId && canManageGroupRole(role, member.role);
              const menuId = `member-actions-${member.userId}`;
              return <MemberRow key={member.userId}>
                <MemberIdentity>
                  <MemberAvatar aria-hidden="true">
                    {member.avatarUrl ? <Image src={member.avatarUrl} alt="" width={36} height={36} /> : getGroupMonogram(member.displayName || member.username)}
                  </MemberAvatar>
                  <div>
                    <strong title={member.displayName || member.username}>{member.displayName || member.username}</strong>
                    <small title={`@${member.username}`}>@{member.username}{member.userId === userId ? " · You" : ""}</small>
                  </div>
                </MemberIdentity>
                {canManage && role === "owner"
                  ? <RoleSelect aria-label={`Role for @${member.username}`} value={member.role} disabled={pending} options={ROLE_OPTIONS} onChange={(nextRole) => {
                      void mutate(`/members/${encodeURIComponent(member.userId)}/role`, "PATCH", { role: nextRole });
                    }} />
                  : <RoleBadge>{member.role}</RoleBadge>}
                {canManage ? <>
                  <MemberMenuButton type="button" aria-label={`Manage @${member.username}`} popoverTarget={menuId} disabled={pending} onClick={(event) => {
                    const menu = document.getElementById(menuId);
                    if (!menu) return;
                    const bounds = event.currentTarget.getBoundingClientRect();
                    menuScrollPosition.current = { x: window.scrollX, y: window.scrollY };
                    const menuHeight = (role === "owner" ? 2 : 1) * 44 + 14;
                    menu.style.left = `${Math.max(16, Math.min(bounds.right - 224, window.innerWidth - 240))}px`;
                    menu.style.top = `${bounds.bottom + menuHeight + 8 <= window.innerHeight ? bounds.bottom + 6 : Math.max(8, bounds.top - menuHeight - 6)}px`;
                  }}>
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.75" /><circle cx="12" cy="12" r="1.75" /><circle cx="19" cy="12" r="1.75" /></svg>
                  </MemberMenuButton>
                  <MemberActions id={menuId} popover="auto" role="group" aria-label={`Actions for @${member.username}`} onToggle={(event) => {
                    if (event.newState === "open") event.currentTarget.querySelector<HTMLButtonElement>("button")?.focus({ preventScroll: true });
                  }}>
                    {role === "owner" && <MemberAction type="button" disabled={pending} aria-label={`Transfer ownership to @${member.username}`} onClick={(event) => {
                      event.currentTarget.closest<HTMLElement>("[popover]")?.hidePopover();
                      setError(null); setConfirmation({ member, action: "transfer" });
                    }}>Transfer ownership</MemberAction>}
                    <MemberAction type="button" $danger disabled={pending} aria-label={`Remove @${member.username}`} onClick={(event) => {
                      event.currentTarget.closest<HTMLElement>("[popover]")?.hidePopover();
                      setError(null); setConfirmation({ member, action: "remove" });
                    }}>Remove member</MemberAction>
                  </MemberActions>
                </> : <span />}
              </MemberRow>;
            })}
          </ul>
        </>}
      </div>
      {notice && <p role="status">{notice}</p>}
      {error && <p role="alert">{error}</p>}
      {confirmation && <Confirmation ref={dialogRef} aria-labelledby="group-confirm-title" aria-describedby="group-confirm-description" onCancel={(event) => { if (pending) event.preventDefault(); else closeConfirmation(); }} onClose={() => setConfirmation(null)}>
        <h3 id="group-confirm-title">{confirmation.action === "remove" ? "Remove member" : "Transfer ownership"}</h3>
        <p id="group-confirm-description">{confirmation.action === "remove"
          ? `Remove @${confirmation.member.username} from this group? Their personal usage remains, but historical group totals can change.`
          : `Transfer ownership to @${confirmation.member.username}? Your role will become admin.`}</p>
        {error && <p role="alert">{error}</p>}
        <Controls>
          <SecondaryButton type="button" disabled={pending} autoFocus onClick={closeConfirmation}>Cancel</SecondaryButton>
          <SecondaryButton type="button" disabled={pending} onClick={() => void confirmAction()}>{pending ? "Saving..." : "Confirm"}</SecondaryButton>
        </Controls>
      </Confirmation>}
    </Panel>
  );
}
