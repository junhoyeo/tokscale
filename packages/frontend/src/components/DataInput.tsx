"use client";

import { useCallback, useEffect, useId, useRef, useState, type DragEvent } from "react";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";
import { DitherTextarea } from "dither-kit/DitherTextarea";
import type { TokenContributionData } from "@/lib/types";
import { isValidContributionData, normalizeContributionData } from "@/lib/utils";
import { SecondaryButton } from "@/components/leaderboard/RankingUI";

interface DataInputProps {
  onDataLoaded: (data: TokenContributionData) => void;
}

// Graph exports contain daily client/model aggregates rather than raw messages. 64 MiB leaves ample
// room for heavy multi-year histories while bounding the memory used by readAsText and JSON.parse.
const MAX_LOCAL_EXPORT_BYTES = 64 * 1024 * 1024;

/** Read a chosen or dropped file as text. FileReader works in every browser the site supports. */
function readFileText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(typeof reader.result === "string" ? reader.result : "");
    reader.onerror = () => reject(reader.error ?? new Error("The file could not be read"));
    reader.readAsText(file);
  });
}

export function DataInput({ onDataLoaded }: DataInputProps) {
  const [rawJson, setRawJson] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const loadGenerationRef = useRef(0);
  const mountedRef = useRef(true);
  const titleId = useId();
  const textareaId = useId();
  const tipId = useId();
  const errorId = useId();
  const stepsId = useId();

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      loadGenerationRef.current += 1;
    };
  }, []);

  const parseText = useCallback((text: string) => {
    setError(null);

    if (!text.trim()) {
      setError("Please enter JSON data");
      return;
    }

    try {
      const parsed = normalizeContributionData(JSON.parse(text));

      if (!isValidContributionData(parsed)) {
        setError("Invalid data format. Expected TokenContributionData structure with meta, summary, years, and contributions.");
        return;
      }

      onDataLoaded(parsed);
    } catch (err) {
      setError(`Invalid JSON: ${err instanceof Error ? err.message : "Unknown error"}`);
    }
  }, [onDataLoaded]);

  const parseJson = useCallback(() => {
    loadGenerationRef.current += 1;
    setIsLoading(false);
    parseText(rawJson);
  }, [parseText, rawJson]);

  // A file is read in the browser, like pasted text: nothing leaves the machine.
  const loadFile = useCallback(async (file: File) => {
    const generation = ++loadGenerationRef.current;
    setIsLoading(false);
    setError(null);
    if (!/\.json$/i.test(file.name) && file.type !== "application/json") {
      setError(`${file.name} is not a .json file. Save the export with tokscale graph --output usage.json.`);
      return;
    }
    if (file.size > MAX_LOCAL_EXPORT_BYTES) {
      setError(`${file.name} is too large. Choose a tokscale graph export under 64 MiB.`);
      return;
    }
    try {
      const text = await readFileText(file);
      if (!mountedRef.current || generation !== loadGenerationRef.current) return;
      setRawJson(text);
      parseText(text);
    } catch (err) {
      if (!mountedRef.current || generation !== loadGenerationRef.current) return;
      setError(`Failed to read ${file.name}: ${err instanceof Error ? err.message : "Unknown error"}`);
    }
  }, [parseText]);

  const draggingFiles = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes("Files");

  const loadSampleData = useCallback(async () => {
    const generation = ++loadGenerationRef.current;
    setIsLoading(true);
    setError(null);

    try {
      const response = await fetch("/sample-data.json");
      if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`);

      const data = normalizeContributionData(await response.json());
      if (!isValidContributionData(data)) throw new Error("Sample data has invalid format");
      if (!mountedRef.current || generation !== loadGenerationRef.current) return;

      setRawJson(JSON.stringify(data, null, 2));
      onDataLoaded(data);
    } catch (err) {
      if (!mountedRef.current || generation !== loadGenerationRef.current) return;
      setError(`Failed to load sample data: ${err instanceof Error ? err.message : "Unknown error"}`);
    } finally {
      if (mountedRef.current && generation === loadGenerationRef.current) setIsLoading(false);
    }
  }, [onDataLoaded]);

  return (
    <Layout>
      <InputCard
        aria-labelledby={titleId}
        $dragging={isDragging}
        onDragEnter={(event) => {
          if (!draggingFiles(event)) return;
          event.preventDefault();
          setIsDragging(true);
        }}
        onDragOver={(event) => {
          if (!draggingFiles(event)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "copy";
        }}
        onDragLeave={(event) => {
          if (event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget)) return;
          setIsDragging(false);
        }}
        onDrop={(event) => {
          if (!draggingFiles(event)) return;
          event.preventDefault();
          setIsDragging(false);
          const file = event.dataTransfer.files[0];
          if (file) void loadFile(file);
        }}
      >
        <CardHeader>
          <CardTitle id={titleId}>Load token usage data</CardTitle>
          <CardLead>
            Paste JSON from <InlineCode>tokscale graph</InlineCode>, choose or drop a saved export, or load sample data.
          </CardLead>
        </CardHeader>

        <CardBody>
          <FieldLabel htmlFor={textareaId}>Export JSON</FieldLabel>
          <JsonTextarea
            id={textareaId}
            aria-describedby={error ? `${tipId} ${errorId}` : tipId}
            invalid={!!error}
            value={rawJson}
            onChange={(value) => {
              loadGenerationRef.current += 1;
              setIsLoading(false);
              setRawJson(value);
              setError(null);
            }}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                parseJson();
              }
            }}
            placeholder='{"meta": {...}, "summary": {...}, "contributions": [...]}'
            spellCheck={false}
          />
          <Tip id={tipId}>Tip: Press Ctrl+Enter (Cmd+Enter on Mac) to parse. You can also drop a .json file on this card.</Tip>

          {error && (
            <ErrorBox id={errorId} role="alert">
              {error}
            </ErrorBox>
          )}

          <Actions>
            <ParseButton
              type="button"
              color="blue"
              variant="solid"
              onClick={parseJson}
              disabled={isLoading || !rawJson.trim()}
            >
              Parse JSON
            </ParseButton>

            <SecondaryButton type="button" onClick={() => fileInputRef.current?.click()} disabled={isLoading}>
              Choose file
            </SecondaryButton>
            <input
              ref={fileInputRef}
              type="file"
              accept=".json,application/json"
              hidden
              onChange={(event) => {
                const file = event.target.files?.[0];
                // Clear it so choosing the same file again still loads it.
                event.target.value = "";
                if (file) void loadFile(file);
              }}
            />

            <SecondaryButton type="button" onClick={loadSampleData} disabled={isLoading}>
              {isLoading ? "Loading..." : "Load sample data"}
            </SecondaryButton>
          </Actions>
        </CardBody>

        {isDragging && <DropHint aria-hidden="true">Drop the export to load it</DropHint>}
      </InputCard>

      <Steps aria-labelledby={stepsId}>
        <StepsTitle id={stepsId}>How to get your data</StepsTitle>
        <StepList>
          <li>
            Install tokscale
            <Command>bunx tokscale</Command>
          </li>
          <li>
            Run the graph command
            <Command>tokscale graph</Command>
          </li>
          <li>
            Copy the JSON output and paste it into the box, or save it to a file and choose or drop it here.
            <Command>tokscale graph --output usage.json</Command>
          </li>
        </StepList>
      </Steps>
    </Layout>
  );
}

const Layout = styled.div`
  display: grid;
  max-width: 1120px;
  gap: 24px;
  align-items: start;
`;

/* The drop target is the card itself, so a border that answers the drag is the
   one place a rule carries meaning. */
const InputCard = styled.section<{ $dragging: boolean }>`
  position: relative;
  min-width: 0;
  overflow: hidden;
  border: 1px solid ${({ $dragging }) => ($dragging ? "var(--service-focus)" : "var(--service-border)")};
  border-radius: 2px;
  background: var(--service-surface);
`;

const DropHint = styled.div`
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  border: 2px dashed var(--service-focus);
  border-radius: 2px;
  background: rgba(7, 9, 13, 0.94);
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 500;
  pointer-events: none;
`;

const CardHeader = styled.header`
  padding: 16px 20px;
  border-bottom: 1px solid var(--service-border);

  @media (max-width: 520px) {
    padding: 14px 16px;
  }
`;

const CardTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
`;

const CardLead = styled.p`
  margin: 4px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
`;

const InlineCode = styled.code`
  color: var(--service-text);
  font-size: 0.8125rem;
  white-space: nowrap;
`;

const CardBody = styled.div`
  display: grid;
  gap: 8px;
  padding: 16px 20px 20px;

  @media (max-width: 520px) {
    padding: 14px 16px 16px;
  }
`;

const FieldLabel = styled.label`
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
`;

/* The export is literal JSON, so this field is the one place monospace is
   correct: real code, set in a form control rather than a <pre>. */
const JsonTextarea = styled(DitherTextarea)`
  min-height: 240px;
  height: 18rem;
  resize: vertical;
  font-family: var(--v2-font-mono);
  font-size: 0.8125rem;
  line-height: 1.55;

  &::placeholder {
    color: var(--v2-text-subtle);
  }

  @media (max-width: 640px) {
    height: 14rem;
    font-size: 0.875rem;
  }
`;

const Tip = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
`;

const ErrorBox = styled.p`
  margin: 4px 0 0;
  padding: 10px 12px;
  border: 1px solid rgba(255, 107, 107, 0.45);
  border-radius: 2px;
  background: rgba(255, 107, 107, 0.08);
  color: #ff9b9b;
  font-size: 0.875rem;
  font-weight: 500;
  line-height: 1.5;
  overflow-wrap: anywhere;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 16px;
  margin-top: 8px;
`;

const ParseButton = styled(DitherButton)`
  min-height: 40px;
  padding: 0 16px;
  border-radius: 2px;
  font-size: 0.8125rem;

  @media (pointer: coarse) {
    min-height: 44px;
  }
`;

const Steps = styled.section`
  min-width: 0;
  padding: 16px 20px 20px;
  border: 1px solid var(--service-border);
  border-radius: 2px;
  background: var(--service-surface);

  @media (max-width: 520px) {
    padding: 14px 16px 16px;
  }
`;

const StepsTitle = styled.h2`
  margin: 0 0 12px;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
`;

/* A real ordered list: the order matters, so the numbers are the list's own. */
const StepList = styled.ol`
  display: grid;
  gap: 16px 32px;
  margin: 0;
  padding: 0 0 0 20px;

  li {
    min-width: 0;
    color: var(--service-text-muted);
    font-size: 0.875rem;
    line-height: 1.5;
  }

  li::marker {
    color: var(--service-text-muted);
    font-variant-numeric: tabular-nums;
  }

  @media (min-width: 820px) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
`;

const Command = styled.code`
  display: block;
  width: fit-content;
  max-width: 100%;
  overflow-x: auto;
  margin-top: 6px;
  color: var(--service-text);
  font-size: 0.8125rem;
  white-space: normal;
`;
