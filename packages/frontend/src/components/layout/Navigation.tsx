"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import styled, { css } from "styled-components";
import { DitherAvatar } from "dither-kit/DitherAvatar";
import { GearIcon, PersonIcon, SignOutIcon } from "@/components/ui/Icons";
import { TokscaleMark, TokscaleWordmark } from "@/components/layout/TokscaleMark";
import { DitherUnderline } from "@/components/ui/DitherUnderline";

interface User {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
}

interface NavItem {
  href: string;
  label: string;
  /** Path prefixes that mark this item as the current section. */
  match: string[];
}

export const NAV_ITEMS: NavItem[] = [
  { href: "/leaderboard", label: "Leaderboards", match: ["/leaderboard", "/groups"] },
  { href: "/models", label: "Models", match: ["/models", "/benchmarks"] },
  { href: "/capital", label: "Capital", match: ["/capital"] },
  { href: "/tracker", label: "Tracker", match: ["/tracker"] },
];

export function isNavItemActive(item: NavItem, pathname: string | null): boolean {
  if (!pathname) return false;
  return item.match.some((prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`));
}

/** Below this width the links move into the menu panel. */
const COLLAPSE_BELOW = "720px";
export const NAV_HEIGHT = 56;

const focusRing = css`
  &:focus-visible {
    outline: 2px solid var(--v2-focus, #75b6ff);
    outline-offset: 2px;
  }
`;

/* A plain bar on the page ground: no chrome, no blur, no pill. */
const Bar = styled.nav`
  position: fixed;
  top: 0;
  right: 0;
  left: 0;
  z-index: 50;
  background: var(--v2-canvas);
`;

/* One baseline across the bar. The wordmark has no descenders, so its bottom
   edge is its baseline; the mark and the wordmark share that bottom
   (`align-items: flex-end`), and the 14px labels move down by
   LABEL_BASELINE_SHIFT so their baseline lands on the same y (36px of the
   56px bar). Measured in the browser: re-measure if the mark, the wordmark
   height or the label size changes. */
const LABEL_BASELINE_SHIFT = "4.5px";

const BarInner = styled.div`
  display: flex;
  align-items: center;
  gap: 24px;
  width: 100%;
  max-width: 1500px;
  height: ${NAV_HEIGHT}px;
  margin: 0 auto;
  padding: 0 32px;

  @media (max-width: 520px) {
    gap: 12px;
    padding: 0 16px;
  }
`;

const LogoLink = styled(Link)`
  display: flex;
  flex-shrink: 0;
  align-items: flex-end;
  gap: 9px;
  transform: translateY(-3px);
  ${focusRing}
`;

const Links = styled.ul`
  display: flex;
  flex: 1;
  align-items: center;
  gap: 4px;
  min-width: 0;
  margin: 0;
  padding: 0;
  list-style: none;

  @media (max-width: ${COLLAPSE_BELOW}) {
    display: none;
  }
`;

/* Selection is the dithered accent line under the label, as wide as the label. */
const NavLink = styled(Link)<{ $active: boolean }>`
  display: flex;
  align-items: center;
  height: 36px;
  padding: 0 8px;
  line-height: 1.2;
  color: ${({ $active }) => ($active ? "var(--v2-text)" : "var(--v2-text-muted)")};
  font-size: 14px;
  font-weight: 500;
  letter-spacing: -0.005em;
  white-space: nowrap;
  text-decoration: none;
  ${focusRing}

  &:hover {
    color: var(--v2-text);
  }
`;

const NavLinkLabel = styled.span<{ $active: boolean }>`
  position: relative;
  display: inline-flex;
  line-height: 1.2;
  transform: translateY(${LABEL_BASELINE_SHIFT});
  color: inherit;
  font-weight: ${({ $active }) => ($active ? 600 : 500)};
`;

/* Absolutely placed 4px under the baseline, so the line never moves the label
   off the shared baseline; the canvas never feeds its own width back into the
   label. */
const NavLinkUnderline = styled.span`
  position: absolute;
  bottom: -5px;
  left: 0;
  display: block;
  width: 100%;
  height: 2px;
  pointer-events: none;

  canvas {
    display: block;
    image-rendering: pixelated;
  }
`;

const Right = styled.div`
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 8px;
  margin-left: auto;
`;

const pulse = css`
  @keyframes nav-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.5;
    }
  }
`;

const LoadingSkeleton = styled.div`
  ${pulse}
  width: 32px;
  height: 32px;
  border-radius: 2px;
  background-color: var(--v2-surface-raised);
  animation: nav-pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;

  @media (prefers-reduced-motion: reduce) {
    animation: none;
  }
`;

const MenuWrapper = styled.div`
  position: relative;
`;

const AvatarButton = styled.button`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  padding: 0;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface-raised);
  ${focusRing}
`;

const Avatar = styled.img`
  display: block;
  width: 100%;
  height: 100%;
  border-radius: var(--radius, 2px);
  object-fit: cover;
`;

/* A floating menu: one of the few things allowed to lift off the page. */
const UserMenuPanel = styled.div`
  position: absolute;
  top: calc(100% + 10px);
  right: 0;
  z-index: 100;
  width: 224px;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface-raised);
  box-shadow: 0 18px 40px rgba(0, 0, 0, 0.55); /* taste-skill-ignore: floating account menu */
`;

const UserInfo = styled.div`
  padding: 12px;
`;

const UserName = styled.p`
  overflow: hidden;
  color: var(--v2-text);
  font-size: 14px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const UserHandle = styled.p`
  overflow: hidden;
  color: var(--v2-text-muted);
  font-size: 13px;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const menuItem = css<{ $danger?: boolean }>`
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 36px;
  padding: 8px 12px;
  border: 0;
  background: transparent;
  color: ${({ $danger }) => ($danger ? "#ff7b72" : "var(--v2-text)")};
  font-size: 14px;
  text-align: left;
  text-decoration: none;

  svg {
    flex-shrink: 0;
    color: ${({ $danger }) => ($danger ? "#ff7b72" : "var(--v2-text-muted)")};
  }

  &:hover {
    color: ${({ $danger }) => ($danger ? "#ff7b72" : "var(--v2-text)")};
    text-decoration: underline;
    text-decoration-color: currentColor;
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: -2px;
  }
`;

const MenuLink = styled(Link)<{ $danger?: boolean }>`
  ${menuItem}
`;

const MenuButton = styled.button<{ $danger?: boolean }>`
  ${menuItem}
`;

const MenuGroup = styled.div`
  padding: 6px 0;

  & + & {
    padding-top: 10px;
  }
`;

const MenuToggle = styled.button`
  position: relative;
  display: none;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  margin-right: -8px;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--v2-text);
  ${focusRing}

  @media (max-width: ${COLLAPSE_BELOW}) {
    display: flex;
  }

  @media (pointer: coarse) {
    width: 44px;
    height: 44px;
  }
`;

const MobilePanel = styled.div<{ $open: boolean }>`
  display: none;

  @media (max-width: ${COLLAPSE_BELOW}) {
    display: ${({ $open }) => ($open ? "block" : "none")};
    max-height: calc(100dvh - ${NAV_HEIGHT}px);
    overflow-y: auto;
    background: var(--v2-canvas);
  }
`;

const MobileList = styled.ul`
  margin: 0;
  padding: 8px 16px;
  list-style: none;
`;

const MobileNavLink = styled(Link)<{ $active: boolean }>`
  display: flex;
  align-items: center;
  min-height: 48px;
  padding: 0 4px;
  color: ${({ $active }) => ($active ? "var(--v2-text)" : "var(--v2-text-muted)")};
  font-size: 17px;
  font-weight: ${({ $active }) => ($active ? 600 : 500)};
  text-decoration: none;
  ${focusRing}
`;

const MobileAuth = styled.div`
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 16px 20px;
`;

const MobileUserRow = styled.div`
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 4px;
`;

const MobileAvatarFrame = styled.span`
  display: block;
  width: 32px;
  height: 32px;
  overflow: hidden;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface-raised);
`;

const MobileAction = styled(Link)<{ $danger?: boolean }>`
  ${menuItem}
  min-height: 44px;
  padding: 0 4px;
`;

const MobileActionButton = styled.button<{ $danger?: boolean }>`
  ${menuItem}
  min-height: 44px;
  padding: 0 4px;
`;

function GitHubMark({ size = 16 }: { size?: number }) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 0C5.37 0 0 5.37 0 12c0 5.3 3.44 9.8 8.21 11.39.6.11.79-.26.79-.58v-2.23c-3.34.73-4.03-1.42-4.03-1.42-.55-1.39-1.33-1.76-1.33-1.76-1.09-.74.08-.73.08-.73 1.2.08 1.84 1.24 1.84 1.24 1.07 1.83 2.81 1.3 3.49 1 .11-.78.42-1.31.76-1.61-2.67-.3-5.47-1.33-5.47-5.93 0-1.31.47-2.38 1.24-3.22-.12-.3-.54-1.52.12-3.18 0 0 1.01-.32 3.3 1.23a11.5 11.5 0 0 1 6.01 0c2.29-1.55 3.3-1.23 3.3-1.23.65 1.66.24 2.88.12 3.18.77.84 1.23 1.91 1.23 3.22 0 4.61-2.81 5.62-5.48 5.92.43.37.82 1.1.82 2.22v3.29c0 .32.19.69.8.58C20.57 21.8 24 17.3 24 12c0-6.63-5.37-12-12-12Z" />
    </svg>
  );
}

function MenuIcon({ open }: { open: boolean }) {
  return (
    <svg aria-hidden="true" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round">
      {open ? (
        <>
          <path d="M6 6l12 12" />
          <path d="M18 6L6 18" />
        </>
      ) : (
        <>
          <path d="M4 7h16" />
          <path d="M4 12h16" />
          <path d="M4 17h16" />
        </>
      )}
    </svg>
  );
}

async function signOut() {
  await fetch("/api/auth/logout", { method: "POST" });
  window.location.href = "/leaderboard";
}

/**
 * A real GitHub avatar when the account has one; the seeded dither avatar
 * otherwise, or when the picture fails to load (a dev seed account has no
 * picture, and asking github.com for one 404s).
 */
function UserAvatar({ user, size = 32 }: { user: User; size?: number }) {
  const [failed, setFailed] = useState(false);
  if (!user.avatarUrl || failed) {
    return (
      <DitherAvatar
        name={user.username}
        color="blue"
        size={size}
        grid={8}
        cellPx={4}
        animate={false}
        bloom="off"
      />
    );
  }
  return <Avatar src={user.avatarUrl} alt="" width={size} height={size} onError={() => setFailed(true)} />;
}

function UserMenu({ user }: { user: User }) {
  const [open, setOpen] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const close = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    if (restoreFocus) buttonRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: MouseEvent) => {
      if (wrapperRef.current && !wrapperRef.current.contains(event.target as Node)) close(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close(true);
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, close]);

  return (
    <MenuWrapper ref={wrapperRef}>
      <AvatarButton
        ref={buttonRef}
        type="button"
        aria-label={`Account menu for @${user.username}`}
        aria-expanded={open}
        aria-controls="account-menu"
        onClick={() => setOpen((value) => !value)}
      >
        <UserAvatar user={user} />
      </AvatarButton>
      {open && (
        <UserMenuPanel id="account-menu">
          <UserInfo>
            <UserName>{user.displayName || user.username}</UserName>
            <UserHandle>@{user.username}</UserHandle>
          </UserInfo>
          <MenuGroup>
            <MenuLink href={`/u/${user.username}`} onClick={() => close(false)}>
              <PersonIcon />
              Your profile
            </MenuLink>
            <MenuLink href="/settings" onClick={() => close(false)}>
              <GearIcon />
              Settings
            </MenuLink>
          </MenuGroup>
          <MenuGroup>
            <MenuButton type="button" $danger onClick={() => void signOut()}>
              <SignOutIcon />
              Sign out
            </MenuButton>
          </MenuGroup>
        </UserMenuPanel>
      )}
    </MenuWrapper>
  );
}

export function Navigation() {
  const pathname = usePathname();
  const [user, setUser] = useState<User | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  // The fold is open only while it was opened on the current path, so a
  // navigation closes it without syncing state from an effect.
  const [mobileOpenAt, setMobileOpenAt] = useState<string | null>(null);
  const mobileOpen = mobileOpenAt === pathname;
  const setMobileOpen = (open: boolean) => setMobileOpenAt(open ? pathname : null);
  const toggleRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/api/auth/session")
      .then((res) => res.json())
      .then((data) => {
        if (cancelled) return;
        setUser(data.user || null);
        setIsLoading(false);
      })
      .catch(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!mobileOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMobileOpenAt(null);
        toggleRef.current?.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [mobileOpen]);

  const closeMobile = () => setMobileOpen(false);

  return (
    <Bar aria-label="Main navigation">
      <BarInner>
        <LogoLink href="/" aria-label="Tokscale home">
          <TokscaleMark height={22} />
          <TokscaleWordmark height={17} />
        </LogoLink>

        <Links>
          {NAV_ITEMS.map((item) => {
            const active = isNavItemActive(item, pathname);
            return (
              <li key={item.href}>
                <NavLink href={item.href} $active={active} aria-current={active ? "page" : undefined}>
                  <NavLinkLabel $active={active}>
                    {item.label}
                    <NavLinkUnderline>{active ? <DitherUnderline /> : null}</NavLinkUnderline>
                  </NavLinkLabel>
                </NavLink>
              </li>
            );
          })}
        </Links>

        <Right>
          {isLoading ? (
            <LoadingSkeleton aria-hidden="true" />
          ) : user ? (
            <UserMenu user={user} />
          ) : (
            <SignInLink href="/api/auth/github">
              <SignInLabel>
                <GitHubMark size={14} />
                Sign in
                <SignInUnderline>
                  <DitherUnderline />
                </SignInUnderline>
              </SignInLabel>
            </SignInLink>
          )}
          <MenuToggle
            ref={toggleRef}
            type="button"
            aria-label={mobileOpen ? "Close menu" : "Open menu"}
            aria-controls="mobile-navigation-menu"
            aria-expanded={mobileOpen}
            onClick={() => setMobileOpen(!mobileOpen)}
          >
            <MenuIcon open={mobileOpen} />
          </MenuToggle>
        </Right>
      </BarInner>

      <MobilePanel id="mobile-navigation-menu" $open={mobileOpen}>
        <MobileList>
          {NAV_ITEMS.map((item) => {
            const active = isNavItemActive(item, pathname);
            return (
              <li key={item.href}>
                <MobileNavLink
                  href={item.href}
                  $active={active}
                  aria-current={active ? "page" : undefined}
                  onClick={closeMobile}
                >
                  {item.label}
                </MobileNavLink>
              </li>
            );
          })}
        </MobileList>
        <MobileAuth>
          {isLoading ? null : user ? (
            <>
              <MobileUserRow>
                <MobileAvatarFrame>
                  <UserAvatar user={user} />
                </MobileAvatarFrame>
                <div>
                  <UserName>{user.displayName || user.username}</UserName>
                  <UserHandle>@{user.username}</UserHandle>
                </div>
              </MobileUserRow>
              <MobileAction href={`/u/${user.username}`} onClick={closeMobile}>
                <PersonIcon />
                Your profile
              </MobileAction>
              <MobileAction href="/settings" onClick={closeMobile}>
                <GearIcon />
                Settings
              </MobileAction>
              <MobileActionButton type="button" $danger onClick={() => void signOut()}>
                <SignOutIcon />
                Sign out
              </MobileActionButton>
            </>
          ) : null}
        </MobileAuth>
      </MobilePanel>
    </Bar>
  );
}

/* The nav's action is quiet: it must never out-shout the screen's own primary
   action. Same size as the links, the dithered accent line on hover/focus. */
const SignInLink = styled.a`
  display: inline-flex;
  align-items: center;
  height: 36px;
  padding: 0 8px;
  color: var(--v2-text-muted);
  font-size: 14px;
  font-weight: 500;
  line-height: 1.2;
  white-space: nowrap;
  text-decoration: none;
  ${focusRing}

  &:hover,
  &:focus-visible {
    color: var(--v2-text);
  }

  /* The dithered line appears under the label; its canvas paints once. */
  &:hover > span > span:last-child,
  &:focus-visible > span > span:last-child {
    opacity: 1;
  }
`;

const SignInLabel = styled.span`
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  line-height: 1.2;
  transform: translateY(${LABEL_BASELINE_SHIFT});

  svg {
    color: currentColor;
  }
`;

const SignInUnderline = styled.span`
  position: absolute;
  bottom: -5px;
  left: 0;
  display: block;
  width: 100%;
  height: 2px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 120ms ease;

  canvas {
    display: block;
    image-rendering: pixelated;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;


