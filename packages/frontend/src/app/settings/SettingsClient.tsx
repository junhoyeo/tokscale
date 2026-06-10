"use client";

import { useState, useEffect } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { KeyIcon } from "@/components/ui/Icons";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";
import { deviceDisplayLabel } from "@/lib/devices/shared";
import { formatNumber, formatCurrency } from "@/lib/utils";
import { formatRelativeTime } from "@/lib/format";

interface User {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  email: string | null;
}

interface ApiToken {
  id: string;
  name: string;
  createdAt: string;
  lastUsedAt: string | null;
}

interface CreatedApiToken extends ApiToken {
  token: string;
}

// Subset of GET /api/users/[username]/devices we render here. That public
// endpoint already aggregates usage per device, so settings reuses it with
// the session user's username instead of adding a private listing route.
interface SettingsDevice {
  id: string;
  deviceKey: string;
  /** Resolved label (custom name or fallback) — what we render. */
  displayName: string;
  /** Raw user-set name (null = never renamed) — what we edit. */
  customName: string | null;
  lastSubmittedAt: string | null;
  totalTokens: number;
  totalCost: number;
  activeDays: number;
}

// Mirror the server-side RenameBodySchema in
// /api/settings/devices/[deviceId]/route.ts (varchar(120), no control chars).
const DEVICE_NAME_MAX_LENGTH = 120;
const DEVICE_NAME_CONTROL_CHARS = /\p{C}/u;

function validateDeviceName(name: string): string | null {
  if (name.length > DEVICE_NAME_MAX_LENGTH) {
    return `Device name must be ${DEVICE_NAME_MAX_LENGTH} characters or fewer`;
  }
  if (DEVICE_NAME_CONTROL_CHARS.test(name)) {
    return "Device name must not contain control characters";
  }
  return null;
}

const PageWrapper = styled.div`
  min-height: 100vh;
  display: flex;
  flex-direction: column;
`;

const MainContent = styled.main`
  flex: 1;
  max-width: 768px;
  margin: 0 auto;
  padding: 40px 24px;
  width: 100%;
`;

const LoadingMain = styled.main`
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
`;

const Title = styled.h1`
  font-size: 30px;
  font-weight: bold;
  margin-bottom: 32px;
`;

const Section = styled.section`
  border-radius: 16px;
  border: 1px solid;
  padding: 24px;
  margin-bottom: 24px;
`;

const SectionTitle = styled.h2`
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 16px;
`;

const ProfileWrapper = styled.div`
  display: flex;
  align-items: center;
  gap: 16px;
`;

const ProfileText = styled.p`
  font-weight: 500;
`;

const SmallText = styled.p`
  font-size: 14px;
`;

const CodeText = styled.code`
  padding: 2px 4px;
  border-radius: 4px;
  font-size: 12px;
`;

const Description = styled.p`
  font-size: 14px;
  margin-bottom: 16px;
`;

const FieldLabel = styled.label`
  display: block;
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 8px;
`;

const ActionRow = styled.div`
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 12px;
  margin-bottom: 16px;

  @media (max-width: 560px) {
    grid-template-columns: 1fr;
  }
`;

const TextInput = styled.input`
  height: 40px;
  padding: 0 12px;
  border-radius: 6px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-default);
  color: var(--color-fg-default);
  font-size: 14px;
`;

const PrimaryButton = styled.button`
  height: 40px;
  padding: 0 14px;
  border-radius: 6px;
  border: 1px solid var(--color-accent-fg);
  background: var(--color-accent-fg);
  color: #ffffff;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 150ms;

  &:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
`;

const TokenReveal = styled.div`
  padding: 12px;
  border-radius: 6px;
  border: 1px solid var(--color-success-fg);
  background: rgba(63, 185, 80, 0.08);
  margin-bottom: 16px;
`;

const TokenCodeRow = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 8px;
  margin-top: 8px;

  @media (max-width: 560px) {
    grid-template-columns: 1fr;
  }
`;

const TokenCode = styled.code`
  display: block;
  overflow-x: auto;
  white-space: nowrap;
  padding: 10px 12px;
  border-radius: 6px;
  background: var(--color-bg-default);
  font-size: 13px;
`;

const SecondaryButton = styled.button`
  height: 38px;
  padding: 0 12px;
  border-radius: 6px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-default);
  color: var(--color-fg-default);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
`;

const ErrorText = styled.p`
  color: #f85149;
  font-size: 13px;
  margin: -4px 0 16px;
`;

const EmptyState = styled.div`
  padding: 32px 0;
  text-align: center;
`;

const EmptyIcon = styled.div`
  margin: 0 auto 12px;
  opacity: 0.5;
`;

const EmptyText = styled.p`
  font-size: 14px;
  margin-top: 8px;
`;

const TokenList = styled.div`
  display: flex;
  flex-direction: column;
  gap: 12px;
`;

const TokenItem = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px;
  border-radius: 12px;
`;

const TokenInfo = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
`;

const IconWrapper = styled.div`
  color: var(--color-fg-muted);
`;

const DeviceEditRow = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  gap: 8px;
  align-items: center;
  width: 100%;

  @media (max-width: 560px) {
    grid-template-columns: 1fr;
  }
`;


const DangerButton = styled.button`
  padding: 4px 12px;
  font-size: 12px;
  font-weight: 500;
  border-radius: 6px;
  border: 1px solid #F85149;
  background: transparent;
  color: #F85149;
  cursor: pointer;
  transition: all 150ms;
  &:hover { background: #F85149; color: #FFFFFF; }
`;

const InfoBanner = styled.div`
  padding: 12px 16px;
  border-radius: 6px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-subtle);
  color: var(--color-fg-muted);
  font-size: 14px;
`;

const AvatarImg = styled.img`
  border-radius: 6px;
  object-fit: cover;
  flex-shrink: 0;
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.1);
`;

const TokenName = styled.p`
  font-weight: 500;
`;

function apiTokenListItem(token: CreatedApiToken): ApiToken {
  return {
    id: token.id,
    name: token.name,
    createdAt: token.createdAt,
    lastUsedAt: token.lastUsedAt,
  };
}

function prependApiToken(tokens: ApiToken[], token: ApiToken): ApiToken[] {
  return [token, ...tokens.filter((item) => item.id !== token.id)];
}

function mergeApiTokenList(
  serverTokens: ApiToken[],
  currentTokens: ApiToken[]
): ApiToken[] {
  const serverTokenIds = new Set(serverTokens.map((token) => token.id));
  const localTokens = currentTokens.filter(
    (token) => !serverTokenIds.has(token.id)
  );
  return [...localTokens, ...serverTokens];
}

async function fetchApiTokens(): Promise<ApiToken[]> {
  const tokensResponse = await fetch("/api/settings/tokens");
  const tokensData = await tokensResponse.json();
  return Array.isArray(tokensData.tokens) ? tokensData.tokens : [];
}

async function fetchDevices(username: string): Promise<SettingsDevice[]> {
  const devicesResponse = await fetch(
    `/api/users/${encodeURIComponent(username)}/devices`
  );
  if (!devicesResponse.ok) return [];
  const devicesData = await devicesResponse.json();
  return Array.isArray(devicesData.devices) ? devicesData.devices : [];
}

export default function SettingsClient() {
  const router = useRouter();
  const [user, setUser] = useState<User | null>(null);
  const [tokens, setTokens] = useState<ApiToken[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [tokenName, setTokenName] = useState("CI token");
  const [createdToken, setCreatedToken] = useState<CreatedApiToken | null>(null);
  const [isCreatingToken, setIsCreatingToken] = useState(false);
  const [createTokenError, setCreateTokenError] = useState<string | null>(null);
  const [devices, setDevices] = useState<SettingsDevice[]>([]);
  const [editingDeviceId, setEditingDeviceId] = useState<string | null>(null);
  const [editingDeviceName, setEditingDeviceName] = useState("");
  const [isSavingDeviceName, setIsSavingDeviceName] = useState(false);
  const [deviceError, setDeviceError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadSettings() {
      try {
        const sessionResponse = await fetch("/api/auth/session");
        const sessionData = await sessionResponse.json();
        if (cancelled) return;

        if (!sessionData.user) {
          router.push("/api/auth/github?returnTo=/settings");
          return;
        }

        const [loadedTokens, loadedDevices] = await Promise.all([
          fetchApiTokens().catch(() => []),
          fetchDevices(sessionData.user.username).catch(
            () => [] as SettingsDevice[]
          ),
        ]);

        if (!cancelled) {
          setUser(sessionData.user);
          setTokens((current) => mergeApiTokenList(loadedTokens, current));
          setDevices(loadedDevices);
          setIsLoading(false);
        }
      } catch {
        if (!cancelled) {
          router.push("/leaderboard");
        }
      }
    }

    loadSettings();
    return () => {
      cancelled = true;
    };
  }, [router]);

  const handleRevokeToken = async (tokenId: string) => {
    if (!confirm("Are you sure you want to revoke this token?")) return;

    try {
      const response = await fetch(`/api/settings/tokens/${tokenId}`, {
        method: "DELETE",
      });

      if (response.ok) {
        setTokens(tokens.filter((t) => t.id !== tokenId));
      }
    } catch {
      alert("Failed to revoke token");
    }
  };

  const handleCreateToken = async () => {
    setIsCreatingToken(true);
    setCreateTokenError(null);

    try {
      const response = await fetch("/api/settings/tokens", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: tokenName }),
      });

      const data = await response.json();
      if (!response.ok || !data.token) {
        throw new Error(data.error || "Failed to create token");
      }

      setCreatedToken(data.token);
      setTokens((current) =>
        prependApiToken(current, apiTokenListItem(data.token))
      );
    } catch (error) {
      setCreateTokenError(error instanceof Error ? error.message : "Failed to create token");
    } finally {
      setIsCreatingToken(false);
    }
  };

  const startEditingDevice = (device: SettingsDevice) => {
    setEditingDeviceId(device.id);
    setDeviceError(null);
    // Pre-fill from the raw custom name, not the resolved display label, so
    // an unnamed device starts empty and a custom name that happens to equal
    // the fallback label ("Unnamed device" etc.) is preserved.
    setEditingDeviceName(device.customName ?? "");
  };

  const cancelEditingDevice = () => {
    setEditingDeviceId(null);
    setEditingDeviceName("");
    setDeviceError(null);
  };

  const handleSaveDeviceName = async (device: SettingsDevice) => {
    const trimmed = editingDeviceName.trim();
    const validationError = validateDeviceName(trimmed);
    if (validationError) {
      setDeviceError(validationError);
      return;
    }

    setIsSavingDeviceName(true);
    setDeviceError(null);

    try {
      const response = await fetch(`/api/settings/devices/${device.id}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        // Empty input clears the custom name; server stores null and the
        // display label falls back via deviceDisplayLabel.
        body: JSON.stringify({ name: trimmed === "" ? null : trimmed }),
      });

      const data = await response.json();
      if (!response.ok || !data.device) {
        throw new Error(data.error || "Failed to rename device");
      }

      setDevices((current) =>
        current.map((item) =>
          item.id === device.id
            ? {
                ...item,
                displayName: deviceDisplayLabel(
                  data.device.deviceKey,
                  data.device.displayName
                ),
                customName: data.device.displayName ?? null,
              }
            : item
        )
      );
      setEditingDeviceId(null);
      setEditingDeviceName("");
    } catch (error) {
      setDeviceError(
        error instanceof Error ? error.message : "Failed to rename device"
      );
    } finally {
      setIsSavingDeviceName(false);
    }
  };

  const handleCopyCreatedToken = async () => {
    if (!createdToken) return;
    await navigator.clipboard.writeText(createdToken.token);
    // The raw token is shown once and only once. After the user has copied
    // it we drop it from React state so it no longer lives in the component
    // tree (and thus no longer in any DevTools / extension snapshot of it).
    // Users who haven't copied yet still have the value in the reveal panel
    // until they navigate away.
    setCreatedToken(null);
  };

  if (isLoading) {
    return (
      <PageWrapper style={{ backgroundColor: "var(--color-bg-default)" }}>
        <Navigation />
        <LoadingMain>
          <div style={{ color: "var(--color-fg-muted)" }}>Loading...</div>
        </LoadingMain>
        <Footer />
      </PageWrapper>
    );
  }

  if (!user) {
    return null;
  }

  return (
    <PageWrapper style={{ backgroundColor: "var(--color-bg-default)" }}>
      <Navigation />

      <MainContent>
        <Title style={{ color: "var(--color-fg-default)" }}>
          Settings
        </Title>

        <Section
          style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
        >
          <SectionTitle style={{ color: "var(--color-fg-default)" }}>
            Profile
          </SectionTitle>
          <ProfileWrapper>
            <AvatarImg
              src={user.avatarUrl || `https://github.com/${user.username}.png`}
              alt={user.username}
              width={64}
              height={64}
            />
            <div>
              <ProfileText style={{ color: "var(--color-fg-default)" }}>
                {user.displayName || user.username}
              </ProfileText>
              <SmallText style={{ color: "var(--color-fg-muted)" }}>
                @{user.username}
              </SmallText>
              {user.email && (
                <SmallText style={{ color: "var(--color-fg-muted)" }}>
                  {user.email}
                </SmallText>
              )}
            </div>
          </ProfileWrapper>
          <InfoBanner style={{ marginTop: 16 }}>
            Profile information is synced from GitHub and cannot be edited here.
          </InfoBanner>
        </Section>

        <Section
          style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
        >
          <SectionTitle style={{ color: "var(--color-fg-default)" }}>
            API Tokens
          </SectionTitle>
          <Description style={{ color: "var(--color-fg-muted)" }}>
            Create a token for CI or use one generated by{" "}
            <CodeText
              style={{ backgroundColor: "var(--color-bg-subtle)" }}
            >
              tokscale login
            </CodeText>{" "}
            from the CLI.
          </Description>

          <FieldLabel
            htmlFor="token-name"
            style={{ color: "var(--color-fg-default)" }}
          >
            Token name
          </FieldLabel>
          <ActionRow>
            <TextInput
              id="token-name"
              value={tokenName}
              onChange={(event) => setTokenName(event.target.value)}
              maxLength={100}
            />
            <PrimaryButton
              type="button"
              disabled={isCreatingToken}
              onClick={handleCreateToken}
            >
              {isCreatingToken ? "Creating..." : "Create token"}
            </PrimaryButton>
          </ActionRow>

          {createTokenError && <ErrorText>{createTokenError}</ErrorText>}

          {createdToken && (
            <TokenReveal>
              <SmallText style={{ color: "var(--color-fg-default)", fontWeight: 600 }}>
                Copy this token now. It will not be shown again.
              </SmallText>
              <TokenCodeRow>
                <TokenCode style={{ color: "var(--color-fg-default)" }}>
                  {createdToken.token}
                </TokenCode>
                <SecondaryButton type="button" onClick={handleCopyCreatedToken}>
                  Copy
                </SecondaryButton>
              </TokenCodeRow>
            </TokenReveal>
          )}

          {tokens.length === 0 ? (
            <EmptyState style={{ color: "var(--color-fg-muted)" }}>
              <EmptyIcon>
                <KeyIcon size={32} />
              </EmptyIcon>
              <p>No API tokens yet.</p>
              <EmptyText>
                Create one here or run{" "}
                <CodeText
                  style={{ backgroundColor: "var(--color-bg-subtle)" }}
                >
                  tokscale login
                </CodeText>{" "}
                from the CLI.
              </EmptyText>
            </EmptyState>
          ) : (
            <TokenList>
              {tokens.map((token) => (
                <TokenItem
                  key={token.id}
                  style={{ backgroundColor: "var(--color-bg-elevated)" }}
                >
                  <TokenInfo>
                    <IconWrapper>
                      <KeyIcon size={20} />
                    </IconWrapper>
                    <div>
                      <TokenName style={{ color: "var(--color-fg-default)" }}>
                        {token.name}
                      </TokenName>
                      <SmallText style={{ color: "var(--color-fg-muted)" }}>
                        Created {new Date(token.createdAt).toLocaleDateString()}
                        {token.lastUsedAt && (
                          <> - Last used {new Date(token.lastUsedAt).toLocaleDateString()}</>
                        )}
                      </SmallText>
                    </div>
                  </TokenInfo>
                  <DangerButton
                    onClick={() => handleRevokeToken(token.id)}
                  >
                    Revoke
                  </DangerButton>
                </TokenItem>
              ))}
            </TokenList>
          )}
        </Section>

        <Section
          style={{ backgroundColor: "var(--color-bg-default)", borderColor: "var(--color-border-default)" }}
        >
          <SectionTitle style={{ color: "var(--color-fg-default)" }}>
            Devices
          </SectionTitle>
          <Description style={{ color: "var(--color-fg-muted)" }}>
            Machines that have submitted usage data. Rename a device to tell
            your machines apart — the name is shown on your public profile.
          </Description>

          {deviceError && <ErrorText>{deviceError}</ErrorText>}

          {devices.length === 0 ? (
            <EmptyState style={{ color: "var(--color-fg-muted)" }}>
              <p>No devices yet.</p>
              <EmptyText>
                Run{" "}
                <CodeText
                  style={{ backgroundColor: "var(--color-bg-subtle)" }}
                >
                  bunx tokscale submit
                </CodeText>{" "}
                to register this machine.
              </EmptyText>
            </EmptyState>
          ) : (
            <TokenList>
              {devices.map((device) => (
                <TokenItem
                  key={device.id}
                  style={{ backgroundColor: "var(--color-bg-elevated)" }}
                >
                  {editingDeviceId === device.id ? (
                    <DeviceEditRow>
                      <TextInput
                        aria-label="Device name"
                        value={editingDeviceName}
                        maxLength={DEVICE_NAME_MAX_LENGTH}
                        placeholder="Device name (empty to reset)"
                        autoFocus
                        disabled={isSavingDeviceName}
                        onChange={(event) =>
                          setEditingDeviceName(event.target.value)
                        }
                        onKeyDown={(event) => {
                          if (event.key === "Enter") {
                            event.preventDefault();
                            handleSaveDeviceName(device);
                          } else if (event.key === "Escape") {
                            cancelEditingDevice();
                          }
                        }}
                      />
                      <PrimaryButton
                        type="button"
                        disabled={isSavingDeviceName}
                        onClick={() => handleSaveDeviceName(device)}
                      >
                        {isSavingDeviceName ? "Saving..." : "Save"}
                      </PrimaryButton>
                      <SecondaryButton
                        type="button"
                        disabled={isSavingDeviceName}
                        onClick={cancelEditingDevice}
                      >
                        Cancel
                      </SecondaryButton>
                    </DeviceEditRow>
                  ) : (
                    <>
                      <TokenInfo>
                        <div>
                          <TokenName style={{ color: "var(--color-fg-default)" }}>
                            {device.displayName}
                          </TokenName>
                          <SmallText style={{ color: "var(--color-fg-muted)" }}>
                            {formatNumber(device.totalTokens)} tokens
                            {" · "}
                            {formatCurrency(device.totalCost)}
                            {" · "}
                            {device.activeDays} active{" "}
                            {device.activeDays === 1 ? "day" : "days"}
                            {" · "}
                            Last submit {formatRelativeTime(device.lastSubmittedAt)}
                          </SmallText>
                        </div>
                      </TokenInfo>
                      <SecondaryButton
                        type="button"
                        onClick={() => startEditingDevice(device)}
                      >
                        Rename
                      </SecondaryButton>
                    </>
                  )}
                </TokenItem>
              ))}
            </TokenList>
          )}
        </Section>

      </MainContent>

      <Footer />
    </PageWrapper>
  );
}
