"use client";

import { useState, useEffect, useId } from "react";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";
import { DitherInput } from "dither-kit/DitherInput";

interface User {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
}

const Container = styled.div`
  min-height: 100dvh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 112px 32px 64px;
  background: var(--service-canvas);
  color: var(--service-text);

  @media (max-width: 640px) {
    padding: 96px 20px 48px;
  }
`;

/* The task on the left, what the code is and where it comes from on the right:
   a one-task screen still uses the width it is given. */
const Layout = styled.div`
  display: grid;
  width: 100%;
  max-width: 1000px;
  grid-template-columns: minmax(0, 1fr) minmax(280px, 360px);
  align-items: start;
  gap: 64px;

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 40px;
    max-width: 560px;
  }
`;

const Column = styled.div`
  min-width: 0;
`;

const Context = styled.section`
  min-width: 0;
  padding: 20px 24px 24px;
  border: 1px solid var(--service-border);
  border-radius: 2px;
  background: var(--service-surface);
`;

const ContextTitle = styled.h2`
  margin: 0 0 12px;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
`;

const ContextList = styled.ol`
  display: grid;
  gap: 14px;
  margin: 0;
  padding: 0 0 0 18px;

  li {
    color: var(--service-text-muted);
    font-size: 0.875rem;
    line-height: 1.55;
  }

  li::marker {
    color: var(--service-text-muted);
    font-variant-numeric: tabular-nums;
  }
`;

const ContextCode = styled.code`
  display: block;
  width: fit-content;
  max-width: 100%;
  overflow-x: auto;
  margin-top: 6px;
  color: var(--service-text);
  font-size: 0.8125rem;
`;

const ContextNote = styled.p`
  margin: 16px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
`;

const LoadingText = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.75rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
`;

const Subtitle = styled.p`
  margin: 10px 0 32px;
  color: var(--service-text-muted);
  font-size: 1rem;
  line-height: 1.6;
`;

const Block = styled.div`
  display: grid;
  gap: 16px;
`;

const SignInText = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
  line-height: 1.6;
`;

const Form = styled.form`
  display: grid;
  max-width: 420px;
  gap: 16px;
`;

const FieldLabel = styled.label`
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
`;

/* The code the terminal shows is an identifier, not prose: set it wide and
   tracked so it can be read off a laptop screen at arm's length. */
const CodeInput = styled(DitherInput)`
  height: 56px;
  border-radius: 2px;
  font-size: 1.375rem;
  font-weight: 500;
  letter-spacing: 0.24em;
  text-align: center;
  text-transform: uppercase;

  &::placeholder {
    letter-spacing: 0.24em;
    color: var(--v2-text-subtle);
  }
`;

const ErrorText = styled.p`
  margin: 0;
  color: #ff9b9b;
  font-size: 0.875rem;
  line-height: 1.5;
`;

const Submit = styled(DitherButton)`
  width: 100%;
  min-height: 44px;
  border-radius: 2px;
  font-size: 0.875rem;
`;

const UserInfoText = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const Username = styled.span`
  color: var(--service-text);
  font-weight: 500;
`;

const SuccessTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1.25rem;
  font-weight: 500;
  letter-spacing: -0.01em;
`;

const SuccessText = styled.p`
  margin: 8px 0 0;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
  line-height: 1.6;
`;

const SignInButton = styled(DitherButton)`
  justify-self: start;
  min-height: 44px;
  padding: 0 20px;
  border-radius: 2px;
  font-size: 0.875rem;
`;

export default function DeviceClient() {
  const [user, setUser] = useState<User | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [code, setCode] = useState("");
  const [status, setStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [error, setError] = useState("");
  const codeId = useId();
  const errorId = useId();
  const contextId = useId();

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

  const handleCodeChange = (value: string) => {
    const cleaned = value.toUpperCase().replace(/[^A-Z0-9]/g, "");
    setCode(cleaned.length > 4 ? `${cleaned.slice(0, 4)}-${cleaned.slice(4, 8)}` : cleaned);
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
      <Container>
        <LoadingText>Loading…</LoadingText>
      </Container>
    );
  }

  return (
    <Container>
      <Layout>
        <Column>
          <Title>Authorize the CLI</Title>
          <Subtitle>Connect your terminal to Tokscale.</Subtitle>

          {!user ? (
            <Block>
              <SignInText>Sign in with GitHub to authorize the CLI.</SignInText>
              <SignInButton
                color="blue"
                variant="solid"
                nativeButton={false}
                role="link"
                render={<a href="/api/auth/github?returnTo=/device" />}
              >
                Sign in with GitHub
              </SignInButton>
            </Block>
          ) : status === "success" ? (
            <Block>
              <SuccessTitle>Device authorized</SuccessTitle>
              <SuccessText>You can close this window and return to your terminal.</SuccessText>
            </Block>
          ) : (
            <Form onSubmit={handleSubmit}>
              <FieldLabel htmlFor={codeId}>Device code</FieldLabel>
              <CodeInput
                id={codeId}
                type="text"
                value={code}
                onChange={handleCodeChange}
                placeholder="XXXX-XXXX"
                maxLength={9}
                autoFocus
                autoComplete="one-time-code"
                aria-invalid={status === "error" || undefined}
                aria-describedby={status === "error" ? errorId : undefined}
                invalid={status === "error"}
              />

              {error && (
                <ErrorText id={errorId} role="alert">
                  {error}
                </ErrorText>
              )}

              <Submit type="submit" color="blue" variant="solid" disabled={code.length < 9 || status === "loading"}>
                {status === "loading" ? "Authorizing…" : "Authorize device"}
              </Submit>

              <UserInfoText>
                Signed in as <Username>{user.username}</Username>
              </UserInfoText>
            </Form>
          )}
        </Column>

        <Context aria-labelledby={contextId}>
          <ContextTitle id={contextId}>Where the code comes from</ContextTitle>
          <ContextList>
            <li>
              Run the login command in your terminal.
              <ContextCode>bunx tokscale login</ContextCode>
            </li>
            <li>It prints an eight-character code and opens this page.</li>
            <li>Enter the code here and approve. The terminal signs in when this page confirms.</li>
          </ContextList>
          <ContextNote>A code is good for 15 minutes and can be approved once.</ContextNote>
        </Context>
      </Layout>
    </Container>
  );
}
