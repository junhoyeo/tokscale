"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import Link from "next/link";
import Image from "next/image";
import { usePathname } from "next/navigation";
import styled, { css, keyframes } from "styled-components";
import { GearIcon, PersonIcon, SignOutIcon } from "@/components/ui/Icons";

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

const pulse = keyframes`
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
`;

const focusRing = css`
  &:focus-visible {
    outline: 2px solid var(--v2-focus, #75b6ff);
    outline-offset: 2px;
  }
`;

const Bar = styled.nav`
  position: fixed;
  top: 0;
  right: 0;
  left: 0;
  z-index: 50;
  border-bottom: 1px solid var(--v2-rule, rgba(255, 255, 255, 0.1));
  background: rgba(0, 0, 0, 0.78);
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
`;

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
  align-items: center;
  height: 40px;
  border-radius: 6px;
  ${focusRing}
`;

const Links = styled.ul`
  display: flex;
  flex: 1;
  align-items: center;
  gap: 2px;
  min-width: 0;
  margin: 0;
  padding: 0;
  list-style: none;

  @media (max-width: ${COLLAPSE_BELOW}) {
    display: none;
  }
`;

const linkBase = css`
  display: flex;
  align-items: center;
  height: 32px;
  padding: 0 10px;
  border-radius: 6px;
  font-size: 14px;
  font-weight: 500;
  letter-spacing: -0.005em;
  white-space: nowrap;
  text-decoration: none;
  transition: color 120ms ease, background-color 120ms ease;
  ${focusRing}

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const NavLink = styled(Link)<{ $active: boolean }>`
  ${linkBase}
  color: ${({ $active }) => ($active ? "var(--v2-text, #f4f7fb)" : "var(--v2-text-muted, #a3adbd)")};
  background: ${({ $active }) => ($active ? "rgba(255, 255, 255, 0.08)" : "transparent")};

  &:hover {
    color: var(--v2-text, #f4f7fb);
    background: rgba(255, 255, 255, 0.06);
  }
`;

const Right = styled.div`
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 8px;
  margin-left: auto;
`;

const DesktopOnly = styled.div`
  display: flex;
  align-items: center;

  @media (max-width: ${COLLAPSE_BELOW}) {
    display: none;
  }
`;

const SignInLink = styled.a`
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 12px 0 10px;
  border: 1px solid var(--v2-rule-strong, rgba(255, 255, 255, 0.18));
  border-radius: 8px;
  color: var(--v2-text, #f4f7fb);
  font-size: 14px;
  font-weight: 500;
  white-space: nowrap;
  text-decoration: none;
  transition: background-color 120ms ease, border-color 120ms ease;
  ${focusRing}

  &:hover {
    border-color: rgba(255, 255, 255, 0.32);
    background: rgba(255, 255, 255, 0.06);
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const LoadingSkeleton = styled.div`
  width: 32px;
  height: 32px;
  border-radius: 999px;
  background-color: rgba(255, 255, 255, 0.1);
  animation: ${pulse} 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;

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
  border: 1px solid var(--v2-rule-strong, rgba(255, 255, 255, 0.18));
  border-radius: 999px;
  background: var(--v2-surface-raised, #0d1016);
  ${focusRing}
`;

const Avatar = styled.img`
  width: 100%;
  height: 100%;
  border-radius: 50%;
  object-fit: cover;
`;

const UserMenuPanel = styled.div`
  position: absolute;
  top: calc(100% + 10px);
  right: 0;
  z-index: 100;
  width: 224px;
  overflow: hidden;
  border: 1px solid var(--v2-rule-strong, rgba(255, 255, 255, 0.18));
  border-radius: 12px;
  background: #0b0d12;
`;

const UserInfo = styled.div`
  padding: 12px;
  border-bottom: 1px solid var(--v2-rule, rgba(255, 255, 255, 0.1));
`;

const UserName = styled.p`
  overflow: hidden;
  color: var(--v2-text, #f4f7fb);
  font-size: 14px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const UserHandle = styled.p`
  overflow: hidden;
  color: var(--v2-text-muted, #a3adbd);
  font-size: 12px;
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
  color: ${({ $danger }) => ($danger ? "#ff7b72" : "var(--v2-text, #f4f7fb)")};
  font-size: 14px;
  text-align: left;
  text-decoration: none;

  svg {
    flex-shrink: 0;
    color: ${({ $danger }) => ($danger ? "#ff7b72" : "var(--v2-text-muted, #a3adbd)")};
  }

  &:hover {
    background: ${({ $danger }) => ($danger ? "rgba(255, 123, 114, 0.1)" : "rgba(255, 255, 255, 0.06)")};
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus, #75b6ff);
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
  padding: 4px 0;

  & + & {
    border-top: 1px solid var(--v2-rule, rgba(255, 255, 255, 0.1));
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
  border-radius: 8px;
  background: transparent;
  color: var(--v2-text, #f4f7fb);
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
    border-top: 1px solid var(--v2-rule, rgba(255, 255, 255, 0.1));
    background: #000;
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
  border-bottom: 1px solid var(--v2-rule, rgba(255, 255, 255, 0.1));
  color: ${({ $active }) => ($active ? "var(--v2-text, #f4f7fb)" : "var(--v2-text-muted, #a3adbd)")};
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

const MobileAvatar = styled.img`
  width: 32px;
  height: 32px;
  border-radius: 50%;
`;

const MobileAction = styled(Link)<{ $danger?: boolean }>`
  ${menuItem}
  min-height: 44px;
  padding: 0 4px;
  border-radius: 6px;
`;

const MobileActionButton = styled.button<{ $danger?: boolean }>`
  ${menuItem}
  min-height: 44px;
  padding: 0 4px;
  border-radius: 6px;
`;

const MobileSignIn = styled.a`
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 44px;
  border-radius: 8px;
  background: var(--v2-accent-strong, #0066e6);
  color: #fff;
  font-size: 15px;
  font-weight: 600;
  text-decoration: none;
  ${focusRing}
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

function avatarSrc(user: User): string {
  return user.avatarUrl || `https://github.com/${user.username}.png`;
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
        <Avatar src={avatarSrc(user)} alt="" width={32} height={32} />
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
  const [mobileOpen, setMobileOpen] = useState(false);
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
        setMobileOpen(false);
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
          <Image src="/assets/hero-logo.svg" alt="" width={147} height={28} priority />
        </LogoLink>

        <Links>
          {NAV_ITEMS.map((item) => {
            const active = isNavItemActive(item, pathname);
            return (
              <li key={item.href}>
                <NavLink href={item.href} $active={active} aria-current={active ? "page" : undefined}>
                  {item.label}
                </NavLink>
              </li>
            );
          })}
        </Links>

        <Right>
          <DesktopOnly>
            {isLoading ? (
              <LoadingSkeleton aria-hidden="true" />
            ) : user ? (
              <UserMenu user={user} />
            ) : (
              <SignInLink href="/api/auth/github">
                <GitHubMark />
                Sign in
              </SignInLink>
            )}
          </DesktopOnly>
          <MenuToggle
            ref={toggleRef}
            type="button"
            aria-label={mobileOpen ? "Close menu" : "Open menu"}
            aria-controls="mobile-navigation-menu"
            aria-expanded={mobileOpen}
            onClick={() => setMobileOpen((value) => !value)}
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
                <MobileAvatar src={avatarSrc(user)} alt="" width={32} height={32} />
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
          ) : (
            <MobileSignIn href="/api/auth/github">
              <GitHubMark size={18} />
              Sign in with GitHub
            </MobileSignIn>
          )}
        </MobileAuth>
      </MobilePanel>
    </Bar>
  );
}
