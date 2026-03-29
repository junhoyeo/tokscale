"use client";

import { useState, useEffect } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { KeyIcon } from "@/components/ui/Icons";
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
`;

const TokenInfo = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
`;

const IconWrapper = styled.div`
  color: #737373;
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

export default function SettingsClient() {
  const router = useRouter();
  const [user, setUser] = useState<User | null>(null);
  const [tokens, setTokens] = useState<ApiToken[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    fetch("/api/auth/session")
      .then((res) => res.json())
      .then((data) => {
        if (!data.user) {
          router.push("/api/auth/github?returnTo=/settings");
          return;
        }
        setUser(data.user);
        setIsLoading(false);
      })
      .catch(() => {
        router.push("/leaderboard");
      });

    fetch("/api/settings/tokens")
      .then((res) => res.json())
      .then((data) => {
        if (data.tokens) {
          setTokens(data.tokens);
        }
      })
      .catch(() => {});
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
            Tokens are created when you run{" "}
            <CodeText
              style={{ backgroundColor: "var(--color-bg-subtle)" }}
            >
              tokscale login
            </CodeText>{" "}
            from the CLI.
          </Description>

          {tokens.length === 0 ? (
            <EmptyState style={{ color: "var(--color-fg-muted)" }}>
              <EmptyIcon>
                <KeyIcon size={32} />
              </EmptyIcon>
              <p>No API tokens yet.</p>
              <EmptyText>
                Run{" "}
                <CodeText
                  style={{ backgroundColor: "var(--color-bg-subtle)" }}
                >
                  tokscale login
                </CodeText>{" "}
                to create one.
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


      </MainContent>

      <Footer />
    </PageWrapper>
  );
}
