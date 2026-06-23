"use client";

import { useState, useEffect } from "react";
import styled from "styled-components";

interface User {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
}

const Container = styled.div`
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding-left: 16px;
  padding-right: 16px;
`;

const LoadingContainer = styled.div`
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
`;

const LoadingText = styled.div`
  color: var(--color-fg-muted);
`;

const CardWrapper = styled.div`
  max-width: 448px;
  width: 100%;
`;

const Card = styled.div`
  border-radius: 16px;
  border: 1px solid;
  border-color: var(--color-border-default);
  padding: 32px;
  background-color: var(--color-bg-default);
`;

const Header = styled.div`
  text-align: center;
  margin-bottom: 32px;
`;

const IconBox = styled.div`
  width: 64px;
  height: 64px;
  margin-left: auto;
  margin-right: auto;
  margin-bottom: 16px;
  border-radius: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(to bottom right, #53d1f3, #3bc4e8);
  box-shadow: 0 10px 15px -3px rgba(83, 209, 243, 0.25);
`;

const Icon = styled.svg`
  width: 32px;
  height: 32px;
  color: white;
`;

const Title = styled.h1`
  font-size: 24px;
  font-weight: bold;
  color: var(--color-fg-default);
`;

const Subtitle = styled.p`
  margin-top: 8px;
  color: var(--color-fg-muted);
`;

const SignInContainer = styled.div`
  text-align: center;
`;

const SignInText = styled.p`
  margin-bottom: 24px;
  color: var(--color-fg-muted);
`;

const SignInButton = styled.a`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px 24px;
  font-weight: 500;
  border-radius: 12px;
  background-color: var(--color-fg-default);
  color: var(--color-bg-default);
  transition: opacity 0.2s;
  text-decoration: none;

  &:hover {
    opacity: 0.8;
  }
`;

const GitHubIcon = styled.svg`
  width: 20px;
  height: 20px;
`;

const SuccessContainer = styled.div`
  text-align: center;
`;

const SuccessIconBox = styled.div`
  width: 64px;
  height: 64px;
  margin-left: auto;
  margin-right: auto;
  margin-bottom: 16px;
  border-radius: 9999px;
  display: flex;
  align-items: center;
  justify-content: center;
  background-color: rgba(83, 209, 243, 0.1);
`;

const SuccessIcon = styled.svg`
  width: 32px;
  height: 32px;
  color: #53d1f3;
`;

const SuccessTitle = styled.h2`
  font-size: 20px;
  font-weight: 600;
  margin-bottom: 8px;
  color: var(--color-fg-default);
`;

const SuccessText = styled.p`
  color: var(--color-fg-muted);
`;

const Form = styled.form``;

const FormGroup = styled.div`
  margin-bottom: 16px;
`;

const FormText = styled.p`
  text-align: center;
  margin-bottom: 16px;
  color: var(--color-fg-muted);
`;

const Input = styled.input`
  width: 100%;
  padding: 16px;
  text-align: center;
  font-size: 24px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  letter-spacing: 0.3em;
  border: 1px solid;
  border-color: var(--color-border-default);
  border-radius: 12px;
  background-color: var(--color-bg-elevated);
  color: var(--color-fg-default);

  &:focus {
    outline: none;
    box-shadow: 0 0 0 2px var(--color-primary);
    border-color: transparent;
  }

  &::placeholder {
    opacity: 0.5;
  }
`;

const ErrorText = styled.p`
  color: #ef4444;
  font-size: 14px;
  text-align: center;
  margin-bottom: 16px;
`;

const SubmitButton = styled.button`
  width: 100%;
  padding: 12px 24px;
  color: white;
  font-weight: 500;
  border-radius: 12px;
  background-color: var(--color-primary);
  border: none;
  cursor: pointer;
  transition: opacity 0.2s;

  &:hover:not(:disabled) {
    opacity: 0.9;
  }

  &:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
`;

const UserInfoText = styled.p`
  text-align: center;
  font-size: 14px;
  margin-top: 16px;
  color: var(--color-fg-muted);
`;

const Username = styled.span`
  font-weight: 500;
  color: var(--color-fg-muted);
`;

export default function DeviceClient() {
  const [user, setUser] = useState<User | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [code, setCode] = useState("");
  const [status, setStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [error, setError] = useState("");

  useEffect(() => {
    fetch("/api/auth/session")
      .then((res) => res.json())
      .then((data) => {
        setUser(data.user);
        setIsLoading(false);
      })
      .catch(() => {
        setIsLoading(false);
      });
  }, []);

  const handleCodeChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    let value = e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, "");

    if (value.length > 4) {
      value = value.slice(0, 4) + "-" + value.slice(4, 8);
    }

    setCode(value);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setStatus("loading");
    setError("");

    try {
      const response = await fetch("/api/auth/device/authorize", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ userCode: code }),
      });

      if (!response.ok) {
        const data = await response.json();
        throw new Error(data.error || "Invalid code");
      }

      setStatus("success");
    } catch (err) {
      setStatus("error");
      setError(err instanceof Error ? err.message : "Something went wrong");
    }
  };

  if (isLoading) {
    return (
      <LoadingContainer style={{ backgroundColor: "var(--color-bg-default)" }}>
        <LoadingText>Loading...</LoadingText>
      </LoadingContainer>
    );
  }

  return (
    <Container style={{ backgroundColor: "var(--color-bg-default)" }}>
      <CardWrapper>
        <Card>
          <Header>
            <IconBox>
              <Icon
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
                />
              </Icon>
            </IconBox>
            <Title>Authorize CLI</Title>
            <Subtitle>Connect your terminal to Tokscale</Subtitle>
          </Header>

          {!user ? (
            <SignInContainer>
              <SignInText>
                Sign in with GitHub to authorize the CLI.
              </SignInText>
              <SignInButton href="/api/auth/github?returnTo=/device">
                <GitHubIcon fill="currentColor" viewBox="0 0 24 24">
                  <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
                </GitHubIcon>
                Sign in with GitHub
              </SignInButton>
            </SignInContainer>
          ) : status === "success" ? (
            <SuccessContainer>
              <SuccessIconBox>
                <SuccessIcon
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M5 13l4 4L19 7"
                  />
                </SuccessIcon>
              </SuccessIconBox>
              <SuccessTitle>Device Authorized!</SuccessTitle>
              <SuccessText>
                You can close this window and return to your terminal.
              </SuccessText>
            </SuccessContainer>
          ) : (
            <Form onSubmit={handleSubmit}>
              <FormGroup>
                <FormText>
                  Enter the code shown in your terminal:
                </FormText>
                <Input
                  type="text"
                  value={code}
                  onChange={handleCodeChange}
                  placeholder="XXXX-XXXX"
                  maxLength={9}
                  autoFocus
                />
              </FormGroup>

              {error && (
                <ErrorText>{error}</ErrorText>
              )}

              <SubmitButton
                type="submit"
                disabled={code.length < 9 || status === "loading"}
              >
                {status === "loading" ? "Authorizing..." : "Authorize Device"}
              </SubmitButton>

              <UserInfoText>
                Signed in as{" "}
                <Username>{user.username}</Username>
              </UserInfoText>
            </Form>
          )}
        </Card>
      </CardWrapper>
    </Container>
  );
}
