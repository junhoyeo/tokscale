"use client";

import { useState, useCallback } from "react";
import styled, { keyframes } from "styled-components";
import type { TokenContributionData } from "@/lib/types";
import { isValidContributionData } from "@/lib/utils";

const Container = styled.div`
  width: 100%;
  max-width: 56rem; /* max-w-4xl */
  margin-left: auto;
  margin-right: auto;
`;

const Header = styled.div`
  margin-bottom: 2rem; /* mb-8 */
`;

const Title = styled.h2`
  font-size: 1.875rem; /* text-3xl */
  line-height: 2.25rem;
  font-weight: 700; /* font-bold */
  letter-spacing: -0.025em; /* tracking-tight */
  margin-bottom: 0.75rem; /* mb-3 */
  color: var(--color-fg-default);
`;

const Description = styled.p`
  color: var(--color-fg-muted);
`;

const CodeSnippet = styled.code`
  padding: 0.25rem 0.5rem; /* py-1 px-2 */
  border-radius: 0.5rem; /* rounded-lg */
  font-size: 0.875rem; /* text-sm */
  line-height: 1.25rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace; /* font-mono */
  background-color: var(--color-bg-subtle);
`;

const InputWrapper = styled.div`
  margin-bottom: 1.5rem; /* mb-6 */
`;

const StyledTextarea = styled.textarea<{ $hasError?: boolean }>`
  width: 100%;
  height: 18rem; /* h-72 */
  padding: 1rem; /* p-4 */
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace; /* font-mono */
  font-size: 0.875rem; /* text-sm */
  border-radius: 1rem; /* rounded-2xl */
  border-width: 2px;
  resize: vertical; /* resize-y */
  outline: none; /* focus:outline-none */
  transition-property: all;
  transition-duration: 200ms;
  background-color: var(--color-bg-elevated);
  color: var(--color-fg-default);
  
  border-color: ${props => props.$hasError ? '#ef4444' : 'var(--color-border-default)'};

  &:focus {
    /* focus:ring-4 */
    box-shadow: ${props => props.$hasError ? '0 0 0 4px rgba(239, 68, 68, 0.2)' : 'none'};
    border-color: ${props => props.$hasError ? '#ef4444' : 'var(--color-primary)'};
  }
`;

const TipText = styled.p`
  margin-top: 0.5rem; /* mt-2 */
  font-size: 0.875rem; /* text-sm */
  color: var(--color-fg-muted);
`;

const ErrorBox = styled.div`
  margin-bottom: 1.5rem; /* mb-6 */
  padding: 1rem; /* p-4 */
  border-radius: 1rem; /* rounded-2xl */
  border-width: 1px;
  background-color: rgba(239, 68, 68, 0.1);
  border-color: rgba(239, 68, 68, 0.3);
`;

const ErrorText = styled.p`
  font-size: 0.875rem; /* text-sm */
  font-weight: 500; /* font-medium */
  color: #EF4444;
`;

const ButtonGroup = styled.div`
  display: flex;
  gap: 1rem; /* gap-4 */
`;

const PrimaryButton = styled.button`
  padding: 0.75rem 1.5rem; /* px-6 py-3 */
  border-radius: 9999px; /* rounded-full */
  font-weight: 600; /* font-semibold */
  font-size: 0.875rem; /* text-sm */
  color: white;
  transition-property: all;
  transition-duration: 200ms;
  background-color: var(--color-primary);
  box-shadow: 0 10px 15px -3px rgba(83, 209, 243, 0.25);

  &:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  &:not(:disabled):hover {
    box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.1), 0 4px 6px -2px rgba(0, 0, 0, 0.05); /* hover:shadow-lg */
    transform: translateY(-0.125rem); /* hover:-translate-y-0.5 */
    opacity: 0.9; /* hover:opacity-90 */
  }

  &:not(:disabled):active {
    transform: translateY(0); /* active:translate-y-0 */
  }
`;

const SecondaryButton = styled.button`
  padding: 0.75rem 1.5rem; /* px-6 py-3 */
  border-radius: 9999px; /* rounded-full */
  font-weight: 600; /* font-semibold */
  font-size: 0.875rem; /* text-sm */
  transition-property: all;
  transition-duration: 200ms;
  background-color: var(--color-bg-subtle);
  color: var(--color-fg-default);

  &:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  &:not(:disabled):hover {
    box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06); /* hover:shadow-md */
    transform: translateY(-0.125rem); /* hover:-translate-y-0.5 */
  }

  &:not(:disabled):active {
    transform: translateY(0); /* active:translate-y-0 */
  }
`;

const spin = keyframes`
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
`;

const LoadingWrapper = styled.span`
  display: flex;
  align-items: center;
  gap: 0.5rem; /* gap-2 */
`;

const SpinnerSvg = styled.svg`
  animation: ${spin} 1s linear infinite; /* animate-spin */
  height: 1rem; /* h-4 */
  width: 1rem; /* w-4 */
`;

const SpinnerCircle = styled.circle`
  opacity: 0.25;
`;

const SpinnerPath = styled.path`
  opacity: 0.75;
`;

const InstructionsBox = styled.div`
  margin-top: 2.5rem; /* mt-10 */
  padding: 1.5rem; /* p-6 */
  border-radius: 1rem; /* rounded-2xl */
  border-width: 1px;
  background-color: var(--color-card-bg);
  border-color: var(--color-border-default);
`;

const InstructionsTitle = styled.h3`
  font-size: 1rem; /* text-base */
  font-weight: 700; /* font-bold */
  margin-bottom: 1rem; /* mb-4 */
  color: var(--color-fg-default);
`;

const InstructionsList = styled.ol`
  font-size: 0.875rem; /* text-sm */
  list-style-type: decimal; /* list-decimal */
  list-style-position: inside; /* list-inside */
  color: var(--color-fg-muted);
  
  & > li + li {
    margin-top: 0.75rem; /* space-y-3 */
  }
`;

const ListItem = styled.li`
  line-height: 1.625; /* leading-relaxed */
`;

const SmallCodeSnippet = styled.code`
  padding: 0.25rem 0.5rem; /* px-2 py-1 */
  border-radius: 0.5rem; /* rounded-lg */
  font-size: 0.75rem; /* text-xs */
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace; /* font-mono */
  background-color: var(--color-bg-subtle);
`;

interface DataInputProps {
  onDataLoaded: (data: TokenContributionData) => void;
}

export function DataInput({ onDataLoaded }: DataInputProps) {
  const [rawJson, setRawJson] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  const parseJson = useCallback(() => {
    setError(null);

    if (!rawJson.trim()) {
      setError("Please enter JSON data");
      return;
    }

    try {
      const parsed = JSON.parse(rawJson);

      if (!isValidContributionData(parsed)) {
        setError("Invalid data format. Expected TokenContributionData structure with meta, summary, years, and contributions.");
        return;
      }

      onDataLoaded(parsed);
    } catch (err) {
      setError(`Invalid JSON: ${err instanceof Error ? err.message : "Unknown error"}`);
    }
  }, [rawJson, onDataLoaded]);

  const loadSampleData = useCallback(async () => {
    setIsLoading(true);
    setError(null);

    try {
      const response = await fetch("/sample-data.json");
      if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`);

      const data = await response.json();
      if (!isValidContributionData(data)) throw new Error("Sample data has invalid format");

      setRawJson(JSON.stringify(data, null, 2));
      onDataLoaded(data);
    } catch (err) {
      setError(`Failed to load sample data: ${err instanceof Error ? err.message : "Unknown error"}`);
    } finally {
      setIsLoading(false);
    }
  }, [onDataLoaded]);

  return (
    <Container>
      <Header>
        <Title>
          Load Token Usage Data
        </Title>
        <Description>
          Paste JSON from{" "}
          <CodeSnippet>
            tokscale graph
          </CodeSnippet>{" "}
          command, or load sample data.
        </Description>
      </Header>

      <InputWrapper>
        <StyledTextarea
          value={rawJson}
          onChange={(e) => {
            setRawJson(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              parseJson();
            }
          }}
          placeholder='{"meta": {...}, "summary": {...}, "contributions": [...]}'
          $hasError={!!error}
        />
        <TipText>
          Tip: Press Ctrl+Enter (Cmd+Enter on Mac) to parse
        </TipText>
      </InputWrapper>

      {error && (
        <ErrorBox>
          <ErrorText>
            {error}
          </ErrorText>
        </ErrorBox>
      )}

      <ButtonGroup>
        <PrimaryButton
          onClick={parseJson}
          disabled={isLoading || !rawJson.trim()}
        >
          Parse JSON
        </PrimaryButton>

        <SecondaryButton
          onClick={loadSampleData}
          disabled={isLoading}
        >
          {isLoading ? (
            <LoadingWrapper>
              <SpinnerSvg
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <SpinnerCircle
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <SpinnerPath
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </SpinnerSvg>
              Loading...
            </LoadingWrapper>
          ) : (
            "Load Sample Data"
          )}
        </SecondaryButton>
      </ButtonGroup>

      <InstructionsBox>
        <InstructionsTitle>
          How to get your data
        </InstructionsTitle>
        <InstructionsList>
          <ListItem>
            Install tokscale:{" "}
            <SmallCodeSnippet>
              bunx tokscale
            </SmallCodeSnippet>
          </ListItem>
          <ListItem>
            Run the graph command:{" "}
            <SmallCodeSnippet>
              tokscale graph
            </SmallCodeSnippet>
          </ListItem>
          <ListItem>Copy the JSON output and paste it above</ListItem>
        </InstructionsList>
      </InstructionsBox>
    </Container>
  );
}
