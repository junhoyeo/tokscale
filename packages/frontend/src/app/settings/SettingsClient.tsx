"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { CheckIcon, CopyIcon, KeyIcon } from "@/components/ui/Icons";
import { Navigation } from "@/components/layout/Navigation";
import { Footer } from "@/components/layout/Footer";

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
  expiresAt: string | null;
}

interface RevealedToken {
  id: string;
  name: string;
  plainTextToken: string;
  expiresAt: string | null;
}

type ExpiryPreset = "never" | "30d" | "90d";

const dateFormatter = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

function formatDate(value: string): string {
  return dateFormatter.format(new Date(value));
}

function getExpiryText(expiresAt: string | null): string {
  if (!expiresAt) {
    return "No expiry";
  }

  const expiresAtDate = new Date(expiresAt);
  if (expiresAtDate <= new Date()) {
    return `Expired ${formatDate(expiresAt)}`;
  }

  return `Expires ${formatDate(expiresAt)}`;
}

function getExpiresAtFromPreset(preset: ExpiryPreset): string | null {
  if (preset === "never") {
    return null;
  }

  const days = preset === "30d" ? 30 : 90;
  return new Date(Date.now() + days * 24 * 60 * 60 * 1000).toISOString();
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
  gap: 16px;
`;

const TokenInfo = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
`;

const IconWrapper = styled.div`
  color: #737373;
`;

const ButtonRow = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
`;

const TokenActions = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
`;

const DangerButton = styled.button`
  padding: 4px 12px;
  font-size: 12px;
  font-weight: 500;
  border-radius: 6px;
  border: 1px solid #f85149;
  background: transparent;
  color: #f85149;
  cursor: pointer;
  transition: all 150ms;

  &:hover {
    background: #f85149;
    color: #ffffff;
  }
`;

const PrimaryButton = styled.button`
  padding: 10px 16px;
  font-size: 14px;
  font-weight: 600;
  border-radius: 8px;
  border: 1px solid transparent;
  background: #2563eb;
  color: #ffffff;
  cursor: pointer;
  transition: opacity 150ms, transform 150ms;

  &:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
`;

const SecondaryButton = styled.button`
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  font-size: 13px;
  font-weight: 500;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-default);
  color: var(--color-fg-default);
  cursor: pointer;
`;

const InfoBanner = styled.div`
  padding: 12px 16px;
  border-radius: 6px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-subtle);
  color: var(--color-fg-muted);
  font-size: 14px;
`;

const ErrorBanner = styled(InfoBanner)`
  border-color: rgba(248, 81, 73, 0.5);
  color: #f85149;
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

const CreateForm = styled.form`
  display: grid;
  gap: 16px;
  margin-bottom: 16px;
`;

const FormRow = styled.div`
  display: grid;
  gap: 12px;
  grid-template-columns: minmax(0, 2fr) minmax(180px, 1fr) auto;

  @media (max-width: 720px) {
    grid-template-columns: 1fr;
  }
`;

const FormField = styled.label`
  display: grid;
  gap: 8px;
  font-size: 13px;
  font-weight: 500;
  color: var(--color-fg-default);
`;

const TextInput = styled.input`
  width: 100%;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-default);
  color: var(--color-fg-default);
  padding: 10px 12px;
  font-size: 14px;
`;

const SelectInput = styled.select`
  width: 100%;
  border-radius: 8px;
  border: 1px solid var(--color-border-default);
  background: var(--color-bg-default);
  color: var(--color-fg-default);
  padding: 10px 12px;
  font-size: 14px;
`;

const SubmitWrapper = styled.div`
  display: flex;
  align-items: end;
`;

const RevealPanel = styled.div`
  border-radius: 12px;
  border: 1px solid rgba(37, 99, 235, 0.4);
  background: rgba(37, 99, 235, 0.08);
  padding: 16px;
  display: grid;
  gap: 12px;
  margin-bottom: 16px;
`;

const RevealTitle = styled.p`
  font-size: 15px;
  font-weight: 600;
  color: var(--color-fg-default);
`;

const SecretValue = styled.code`
  display: block;
  width: 100%;
  overflow-x: auto;
  padding: 12px;
  border-radius: 8px;
  background: rgba(0, 0, 0, 0.25);
  color: var(--color-fg-default);
  font-size: 13px;
`;

export default function SettingsClient() {
  const router = useRouter();
  const [user, setUser] = useState<User | null>(null);
  const [tokens, setTokens] = useState<ApiToken[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [tokenName, setTokenName] = useState("");
  const [expiryPreset, setExpiryPreset] = useState<ExpiryPreset>("never");
  const [isCreating, setIsCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [revealedToken, setRevealedToken] = useState<RevealedToken | null>(null);
  const [copiedToken, setCopiedToken] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [revokeCandidateId, setRevokeCandidateId] = useState<string | null>(null);
  const [revokingTokenId, setRevokingTokenId] = useState<string | null>(null);
  const [revokeError, setRevokeError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const sessionResponse = await fetch("/api/auth/session");
        const sessionData = await sessionResponse.json();

        if (!sessionData.user) {
          if (cancelled) {
            return;
          }

          router.push("/api/auth/github?returnTo=/settings");
          return;
        }

        if (cancelled) {
          return;
        }

        setUser(sessionData.user);

        const tokensResponse = await fetch("/api/settings/tokens");
        const tokensData = await tokensResponse.json().catch(() => null);

        if (tokensResponse.status === 401) {
          if (cancelled) {
            return;
          }

          router.push("/api/auth/github?returnTo=/settings");
          return;
        }

        if (!tokensResponse.ok) {
          if (cancelled) {
            return;
          }

          setTokens([]);
          setLoadError(tokensData?.error ?? "Failed to load API tokens");
          setIsLoading(false);
          return;
        }

        if (cancelled) {
          return;
        }

        setTokens(tokensData.tokens ?? []);
        setLoadError(null);
        setIsLoading(false);
      } catch {
        if (!cancelled) {
          router.push("/leaderboard");
        }
      }
    }

    load();

    return () => {
      cancelled = true;
    };
  }, [router]);

  const handleCreateToken = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    setCreateError(null);
    setRevealedToken(null);
    setCopiedToken(false);
    setRevokeCandidateId(null);
    setIsCreating(true);

    try {
      const response = await fetch("/api/settings/tokens", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          name: tokenName,
          expiresAt: getExpiresAtFromPreset(expiryPreset),
        }),
      });

      const data = await response.json();

      if (!response.ok) {
        if (response.status === 401) {
          router.push("/api/auth/github?returnTo=/settings");
          return;
        }

        setCreateError(data.error ?? "Failed to create token");
        return;
      }

      const createdToken = data.token as ApiToken;
      const plainTextToken = data.plainTextToken as string;

      setTokens((current) => [createdToken, ...current.filter((token) => token.id !== createdToken.id)]);
      setRevealedToken({
        id: createdToken.id,
        name: createdToken.name,
        plainTextToken,
        expiresAt: createdToken.expiresAt,
      });
      setLoadError(null);
      setRevokeError(null);
      setTokenName("");
      setExpiryPreset("never");
      setCopiedToken(false);
    } catch {
      setCreateError("Failed to create token");
    } finally {
      setIsCreating(false);
    }
  };

  const handleCopyToken = async () => {
    if (!revealedToken) {
      return;
    }

    try {
      await navigator.clipboard.writeText(revealedToken.plainTextToken);
      setCopiedToken(true);
      setRevealedToken(null);
      setTimeout(() => setCopiedToken(false), 2000);
    } catch {
      setCreateError("Failed to copy token");
    }
  };

  const handleRequestRevoke = (tokenId: string) => {
    setRevokeError(null);
    setRevokeCandidateId(tokenId);
  };

  const handleCancelRevoke = () => {
    setRevokeCandidateId(null);
  };

  const handleConfirmRevoke = async (tokenId: string) => {
    setRevokeError(null);
    setRevokingTokenId(tokenId);
    try {
      const revokesRevealedToken = revealedToken?.id === tokenId;
      const response = await fetch(`/api/settings/tokens/${tokenId}`, {
        method: "DELETE",
      });

      if (!response.ok) {
        if (response.status === 401) {
          router.push("/api/auth/github?returnTo=/settings");
          return;
        }

        const data = await response.json().catch(() => null);
        setRevokeError(data?.error ?? "Failed to revoke token");
        return;
      }

      setTokens((current) => current.filter((token) => token.id !== tokenId));
      setRevokeCandidateId((current) => (current === tokenId ? null : current));
      if (revokesRevealedToken) {
        setRevealedToken(null);
        setCopiedToken(false);
      }
    } catch {
      setRevokeError("Failed to revoke token");
    } finally {
      setRevokingTokenId(null);
    }
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
            Create a token here for CI or other headless flows, or keep using{" "}
            <CodeText style={{ backgroundColor: "var(--color-bg-subtle)" }}>
              tokscale login
            </CodeText>{" "}
            from the CLI.
          </Description>

          <CreateForm onSubmit={handleCreateToken}>
            <FormRow>
              <FormField htmlFor="token-name">
                Token name
                <TextInput
                  id="token-name"
                  value={tokenName}
                  onChange={(event) => setTokenName(event.target.value)}
                  placeholder="GitHub Actions CI"
                  maxLength={100}
                />
              </FormField>

              <FormField htmlFor="token-expiry">
                Expiration
                <SelectInput
                  id="token-expiry"
                  value={expiryPreset}
                  onChange={(event) => setExpiryPreset(event.target.value as ExpiryPreset)}
                >
                  <option value="never">No expiry</option>
                  <option value="30d">30 days</option>
                  <option value="90d">90 days</option>
                </SelectInput>
              </FormField>

              <SubmitWrapper>
                <PrimaryButton
                  type="submit"
                  disabled={isCreating || tokenName.trim().length === 0}
                >
                  {isCreating ? "Creating..." : "Create token"}
                </PrimaryButton>
              </SubmitWrapper>
            </FormRow>
          </CreateForm>

          {createError && (
            <ErrorBanner style={{ marginBottom: 16 }}>
              {createError}
            </ErrorBanner>
          )}

          {loadError && (
            <ErrorBanner style={{ marginBottom: 16 }}>
              {loadError}
            </ErrorBanner>
          )}

          {revokeError && (
            <ErrorBanner style={{ marginBottom: 16 }}>
              {revokeError}
            </ErrorBanner>
          )}

          {revealedToken && (
            <RevealPanel>
              <div>
                <RevealTitle>
                  Save this token now
                </RevealTitle>
                <SmallText style={{ color: "var(--color-fg-muted)", marginTop: 4 }}>
                  This is the only time the full token will be shown for{" "}
                  <CodeText style={{ backgroundColor: "rgba(255, 255, 255, 0.08)" }}>
                    {revealedToken.name}
                  </CodeText>
                  {" - "}
                  {getExpiryText(revealedToken.expiresAt)}
                </SmallText>
              </div>

              <SecretValue>{revealedToken.plainTextToken}</SecretValue>

              <ButtonRow>
                <SecondaryButton type="button" onClick={handleCopyToken}>
                  {copiedToken ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
                  {copiedToken ? "Copied" : "Copy token"}
                </SecondaryButton>
              </ButtonRow>
            </RevealPanel>
          )}

          {tokens.length === 0 ? (
            <EmptyState style={{ color: "var(--color-fg-muted)" }}>
              <EmptyIcon>
                <KeyIcon size={32} />
              </EmptyIcon>
              <p>No API tokens yet.</p>
              <EmptyText>
                Create one above to use tokscale from CI or another headless environment.
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
                        Created {formatDate(token.createdAt)}
                        {token.lastUsedAt && (
                          <> - Last used {formatDate(token.lastUsedAt)}</>
                        )}
                        <> - {getExpiryText(token.expiresAt)}</>
                      </SmallText>
                    </div>
                  </TokenInfo>
                  {revokeCandidateId === token.id ? (
                    <TokenActions>
                      <DangerButton
                        onClick={() => handleConfirmRevoke(token.id)}
                        disabled={revokingTokenId === token.id}
                      >
                        {revokingTokenId === token.id ? "Revoking..." : "Confirm revoke"}
                      </DangerButton>
                      <SecondaryButton
                        type="button"
                        onClick={handleCancelRevoke}
                        disabled={revokingTokenId === token.id}
                      >
                        Cancel
                      </SecondaryButton>
                    </TokenActions>
                  ) : (
                    <DangerButton
                      onClick={() => handleRequestRevoke(token.id)}
                      disabled={revokingTokenId !== null}
                    >
                      Revoke
                    </DangerButton>
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
