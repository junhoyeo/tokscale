"use client";

import { useId, useMemo, useState, type ReactNode } from "react";
import styled from "styled-components";
import { CopyCommand } from "@/components/home/CopyCommand";

export const EMBED_SHOWCASE_TEMPLATES = [
  { id: "classic", label: "Overview" },
  { id: "minimal", label: "Token focus" },
  { id: "terminal", label: "Readout" },
  { id: "graph", label: "Contributions" },
  { id: "orbit", label: "Rank focus" },
  { id: "vitals", label: "Activity summary" },
  { id: "blueprint", label: "Detailed stats" },
  { id: "receipt", label: "Compact list" },
] as const;

export type EmbedShowcaseTemplate = (typeof EMBED_SHOWCASE_TEMPLATES)[number]["id"];

const ORIGIN = "https://tokscale.ai";
const PREVIEW_USER = "junhoyeo";
const GITHUB_USERNAME = /^[a-z\d](?:[a-z\d-]{0,38})$/i;

export function embedImageUrl(username: string, template: EmbedShowcaseTemplate): string {
  return `${ORIGIN}/api/embed/${encodeURIComponent(username)}/svg?template=${template}`;
}

export function embedMarkdown(username: string, template: EmbedShowcaseTemplate): string {
  return `[![Tokscale Stats](${embedImageUrl(username, template)})](${ORIGIN}/u/${encodeURIComponent(username)})`;
}

/** `intro` is the section heading and lead; it sits above the controls, beside the live card. */
export function EmbedShowcase({ intro }: { intro?: ReactNode } = {}) {
  const [template, setTemplate] = useState<EmbedShowcaseTemplate>("classic");
  const [username, setUsername] = useState("");
  const inputId = useId();
  const trimmed = username.trim().replace(/^@/, "");
  const valid = trimmed.length > 0 && GITHUB_USERNAME.test(trimmed);
  const snippetUser = valid ? trimmed : "your-username";
  const snippet = useMemo(() => embedMarkdown(snippetUser, template), [snippetUser, template]);
  const label = EMBED_SHOWCASE_TEMPLATES.find((t) => t.id === template)?.label ?? template;

  return (
    <Layout>
      <Controls>
        {intro}
        <fieldset>
          <legend>Template</legend>
          <Options>
            {EMBED_SHOWCASE_TEMPLATES.map((item) => (
              <label key={item.id} data-active={item.id === template || undefined}>
                <input
                  type="radio"
                  name="embed-template"
                  value={item.id}
                  checked={item.id === template}
                  onChange={() => setTemplate(item.id)}
                />
                <span>{item.label}</span>
                <code>{item.id}</code>
              </label>
            ))}
          </Options>
        </fieldset>

        <Field>
          <label htmlFor={inputId}>Your GitHub username</label>
          <input
            id={inputId}
            type="text"
            inputMode="text"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            placeholder="your-username"
            value={username}
            onChange={(event) => setUsername(event.target.value)}
          />
        </Field>

        <Field as="div">
          <span>Markdown for your README</span>
          <CopyCommand command={snippet} label="Copy README markdown" prompt={false} wrap />
        </Field>
      </Controls>

      <Preview>
        <PreviewMeta>
          Preview: {label} for @{PREVIEW_USER}, rendered live by tokscale.ai
        </PreviewMeta>
        <PreviewFrame>
          {/* eslint-disable-next-line @next/next/no-img-element -- remote SVG from the embed API */}
          <img
            key={template}
            src={embedImageUrl(PREVIEW_USER, template)}
            alt={`Tokscale ${label} README card for @${PREVIEW_USER}`}
            loading="lazy"
          />
        </PreviewFrame>
      </Preview>
    </Layout>
  );
}

const Layout = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  gap: 56px;
  align-items: start;

  @media (max-width: 960px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 24px;
  }
`;

const Controls = styled.div`
  display: flex;
  flex-direction: column;
  gap: 24px;
  min-width: 0;

  fieldset {
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    margin-bottom: 10px;
    color: var(--v2-text-muted);
    font-size: 13px;
    font-weight: 500;
  }
`;

const Options = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 6px;

  label {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 52px;
    padding: 8px 12px;
    border-radius: var(--radius, 2px);
    background: var(--v2-surface);
    cursor: pointer;
  }

  label:hover {
    background: var(--v2-surface-raised);
  }

  label[data-active] {
    background: var(--v2-surface-raised);
    background: var(--v2-accent-soft);
  }

  label:has(input:focus-visible) {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }

  input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  span {
    color: var(--v2-text);
    font-size: 14px;
    font-weight: 500;
  }

  code {
    color: var(--v2-text-subtle);
    font-size: 12px;
  }
`;

const Field = styled.div`
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;

  > label,
  > span {
    color: var(--v2-text-muted);
    font-size: 13px;
    font-weight: 500;
  }

  input {
    width: 100%;
    max-width: 420px;
    min-height: 44px;
    padding: 0 12px;
    border: 1px solid var(--v2-rule-strong);
    border-radius: 8px;
    background: var(--v2-surface);
    color: var(--v2-text);
    font-size: 16px;
  }

  input::placeholder {
    color: var(--v2-text-subtle);
  }

  input:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }
`;

/* The live card stays in view while the controls scroll past on wide screens. */
const Preview = styled.div`
  position: sticky;
  top: 96px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;

  @media (max-width: 960px) {
    position: static;
  }
`;

const PreviewMeta = styled.p`
  margin: 0;
  color: var(--v2-text-subtle);
  font-size: 13px;
`;

const PreviewFrame = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 360px;
  padding: 40px;
  border-radius: var(--radius, 2px);
  background: var(--v2-surface);

  img {
    display: block;
    max-width: 100%;
    height: auto;
  }

  @media (max-width: 520px) {
    min-height: 200px;
    padding: 16px;
  }
`;
