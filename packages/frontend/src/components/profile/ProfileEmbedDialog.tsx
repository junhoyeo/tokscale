"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import styled from "styled-components";
import { toast } from "react-toastify";
import { DitherButton } from "dither-kit/DitherButton";
import { useFocusTrap } from "dither-kit/use-focus-trap";
import { Select } from "@/components/ui/Select";
import {
  EMBED_TEMPLATES,
  resolvePalette,
  type EmbedTemplate,
} from "@/lib/embed/embedShared";
import { getPaletteNames, type ColorPaletteName } from "@/lib/themes";
import { DitherSelectionGroup } from "./ditherControls";
import {
  buildEmbedPreviewPath,
  buildProfileEmbedLinks,
  getEmbedDialogCapabilities,
  type EmbedNumberFormat,
  type EmbedPeriod,
  type EmbedRankFormat,
  type EmbedSortBy,
  type EmbedTheme,
  type EmbedView,
} from "./embedDialogOptions";

function titleCase(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

const EMBED_TEMPLATE_LABELS: Record<EmbedTemplate, string> = {
  classic: "Overview",
  minimal: "Token focus",
  terminal: "Readout",
  graph: "Contributions",
  orbit: "Rank focus",
  vitals: "Activity summary",
  blueprint: "Detailed stats",
  receipt: "Compact list",
};

interface ProfileEmbedDialogProps {
  open: boolean;
  username: string;
  displayName?: string | null;
  onClose: () => void;
}

export function ProfileEmbedDialog({
  open,
  username,
  displayName,
  onClose,
}: ProfileEmbedDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const [theme, setTheme] = useState<EmbedTheme>("dark");
  const [period, setPeriod] = useState<EmbedPeriod>("all");
  const [sortBy, setSortBy] = useState<EmbedSortBy>("tokens");
  const [layoutCompact, setLayoutCompact] = useState(false);
  const [threeDCompact, setThreeDCompact] = useState(false);
  const [view, setView] = useState<EmbedView>("2d");
  const [template, setTemplate] = useState<EmbedTemplate>("classic");
  const [color, setColor] = useState<ColorPaletteName | null>(null);
  const [tokensFormat, setTokensFormat] =
    useState<EmbedNumberFormat>("compact");
  const [costFormat, setCostFormat] = useState<EmbedNumberFormat>("compact");
  const [rankFormat, setRankFormat] = useState<EmbedRankFormat>("plain");
  const [graph, setGraph] = useState(false);
  const compact = view === "3d" ? threeDCompact : layoutCompact;
  const setCompactForView = (nextCompact: boolean) => {
    if (view === "3d") {
      setThreeDCompact(nextCompact);
    } else {
      setLayoutCompact(nextCompact);
    }
  };

  const capabilities = getEmbedDialogCapabilities({
    compact,
    template,
    view,
  });

  // The kit's focus trap owns Tab cycling and focus restoration; Escape and
  // the scroll lock stay here because they are dialog policy, not the trap.
  useFocusTrap(dialogRef, open);

  useEffect(() => {
    if (!open) return;

    const previousOverflow = document.body.style.overflow;
    const focusDialog = window.requestAnimationFrame(() => {
      dialogRef.current?.focus();
    });
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };

    document.body.style.overflow = "hidden";
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      window.cancelAnimationFrame(focusDialog);
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, onClose]);

  const { embedUrl, markdownSnippet, htmlSnippet, profileUrl } = useMemo(
    () =>
      buildProfileEmbedLinks(username, {
        color,
        compact,
        costFormat,
        graph,
        period,
        rankFormat,
        sortBy,
        template,
        theme,
        tokensFormat,
        view,
      }),
    [
      color,
      compact,
      costFormat,
      graph,
      period,
      rankFormat,
      sortBy,
      template,
      theme,
      tokensFormat,
      username,
      view,
    ],
  );
  const previewUrl = useMemo(() => buildEmbedPreviewPath(embedUrl), [embedUrl]);

  const copyToClipboard = async (value: string, label: string) => {
    try {
      await navigator.clipboard.writeText(value);
      toast.success(`${label} copied`);
    } catch {
      toast.error(`Failed to copy ${label.toLowerCase()}`);
    }
  };

  if (!open || typeof document === "undefined") {
    return null;
  }

  return createPortal(
    <Overlay
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <Dialog
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="profile-embed-dialog-title"
        aria-describedby="profile-embed-dialog-description"
        tabIndex={-1}
      >
        <DialogHeader>
          <HeaderCopy>
            <DialogTitle id="profile-embed-dialog-title">
              Embed @{username}
            </DialogTitle>
            <DialogDescription id="profile-embed-dialog-description">
              The live card updates with the profile. Pick a template, then
              copy the markdown into a README.
            </DialogDescription>
          </HeaderCopy>

          <CloseButton
            type="button"
            onClick={onClose}
            aria-label="Close embed dialog"
          >
            <CloseIcon />
          </CloseButton>
        </DialogHeader>

        <DialogBody>
          <PreviewPanel>
            <PreviewLabel>Live preview</PreviewLabel>
            <PreviewFrame $threeD={view === "3d"}>
              <PreviewImage
                src={previewUrl}
                alt={`Tokscale README embed preview for ${displayName || username}`}
              />
            </PreviewFrame>
            <PreviewMeta>
              {EMBED_TEMPLATE_LABELS[template]} · {view.toUpperCase()} ·{" "}
              {titleCase(theme)}
            </PreviewMeta>
          </PreviewPanel>

          <ControlsPanel aria-label="Embed customization">
            {capabilities.showTemplate && (
              <OptionRow>
                <OptionLabel id="embed-template-label">Template</OptionLabel>
                <TemplateSelect
                  name="profile-embed-template"
                  aria-labelledby="embed-template-label"
                  value={template}
                  options={EMBED_TEMPLATES.map((tpl) => ({
                    value: tpl,
                    label: EMBED_TEMPLATE_LABELS[tpl],
                  }))}
                  onChange={(next) => setTemplate(next as EmbedTemplate)}
                />
              </OptionRow>
            )}

            <OptionRow>
              <OptionLabel id="embed-view-label">View</OptionLabel>
              <DitherSelectionGroup
                label="Embed view"
                value={view}
                options={[
                  { value: "2d", label: "2D" },
                  { value: "3d", label: "3D" },
                ]}
                onChange={(next) => setView(next as EmbedView)}
              />
            </OptionRow>

            <OptionRow>
              <OptionLabel id="embed-theme-label">Theme</OptionLabel>
              <DitherSelectionGroup
                label="Embed theme"
                value={theme}
                options={[
                  { value: "dark", label: "Dark" },
                  { value: "light", label: "Light" },
                ]}
                onChange={(next) => setTheme(next as EmbedTheme)}
              />
            </OptionRow>

            <OptionRow>
              <OptionLabel id="embed-period-label">Time range</OptionLabel>
              <DitherSelectionGroup
                label="Embed time range"
                value={period}
                options={[
                  { value: "all", label: "Lifetime" },
                  { value: "month", label: "30d" },
                  { value: "week", label: "7d" },
                ]}
                onChange={(next) => setPeriod(next as EmbedPeriod)}
              />
            </OptionRow>

            {capabilities.showAccent && (
              <OptionRow $stacked>
                <OptionLabel id="embed-accent-label">Accent color</OptionLabel>
                <SwatchRow role="group" aria-labelledby="embed-accent-label">
                  <Swatch
                    type="button"
                    $active={color === null}
                    $color={resolvePalette(theme, null).brand}
                    aria-pressed={color === null}
                    aria-label="Default accent color"
                    title="Default"
                    onClick={() => setColor(null)}
                  />
                  {getPaletteNames().map((name) => (
                    <Swatch
                      key={name}
                      type="button"
                      $active={color === name}
                      $color={resolvePalette(theme, name).brand}
                      aria-pressed={color === name}
                      aria-label={`${name} accent color`}
                      title={titleCase(name)}
                      onClick={() => setColor(name)}
                    />
                  ))}
                </SwatchRow>
              </OptionRow>
            )}

            <OptionRow>
              <OptionLabel id="embed-ranking-label">Ranking</OptionLabel>
              <DitherSelectionGroup
                label="Embed ranking metric"
                value={sortBy}
                options={[
                  { value: "tokens", label: "Tokens" },
                  { value: "cost", label: "Cost" },
                ]}
                onChange={(next) => setSortBy(next as EmbedSortBy)}
              />
            </OptionRow>

            {capabilities.showRankFormat && (
              <OptionRow>
                <OptionLabel id="embed-rank-format-label">
                  Rank format
                </OptionLabel>
                <DitherSelectionGroup
                  label="Embed rank format"
                  value={rankFormat}
                  options={["plain", "percent", "total"].map((mode) => ({
                    value: mode,
                    label: titleCase(mode),
                  }))}
                  onChange={(next) => setRankFormat(next as EmbedRankFormat)}
                />
              </OptionRow>
            )}

            {capabilities.showLayout && (
              <OptionRow>
                <OptionLabel id="embed-layout-label">
                  {view === "3d" ? "Number format" : "Layout"}
                </OptionLabel>
                <DitherSelectionGroup
                  label={view === "3d" ? "Embed number format" : "Embed layout"}
                  value={compact ? "compact" : "full"}
                  options={[
                    { value: "full", label: "Full" },
                    { value: "compact", label: "Compact" },
                  ]}
                  onChange={(next) => setCompactForView(next === "compact")}
                />
              </OptionRow>
            )}

            {capabilities.showGraph && (
              <OptionRow>
                <OptionLabel id="embed-graph-label">
                  Contribution graph
                </OptionLabel>
                <DitherSelectionGroup
                  label="Embed contribution graph"
                  value={graph ? "on" : "off"}
                  options={[
                    { value: "off", label: "Off" },
                    { value: "on", label: "On" },
                  ]}
                  onChange={(next) => setGraph(next === "on")}
                />
              </OptionRow>
            )}

            {capabilities.showNumberFormats && (
              <>
                <OptionRow>
                  <OptionLabel id="embed-token-format-label">
                    Token format
                  </OptionLabel>
                  <DitherSelectionGroup
                    label="Embed token format"
                    value={tokensFormat}
                    options={[
                      { value: "compact", label: "Compact" },
                      { value: "full", label: "Full" },
                    ]}
                    onChange={(next) =>
                      setTokensFormat(next as EmbedNumberFormat)
                    }
                  />
                </OptionRow>

                <OptionRow>
                  <OptionLabel id="embed-cost-format-label">
                    Cost format
                  </OptionLabel>
                  <DitherSelectionGroup
                    label="Embed cost format"
                    value={costFormat}
                    options={[
                      { value: "compact", label: "Compact" },
                      { value: "full", label: "Full" },
                    ]}
                    onChange={(next) =>
                      setCostFormat(next as EmbedNumberFormat)
                    }
                  />
                </OptionRow>
              </>
            )}

            <SnippetSection>
              <SnippetHeader>
                <SnippetTitle>Markdown</SnippetTitle>
                <InlineActions>
                  <InlineActionButton
                    type="button"
                    onClick={() => copyToClipboard(embedUrl, "Image URL")}
                  >
                    Copy image URL
                  </InlineActionButton>
                  <InlineActionButton
                    type="button"
                    onClick={() => copyToClipboard(htmlSnippet, "HTML snippet")}
                  >
                    Copy HTML
                  </InlineActionButton>
                </InlineActions>
              </SnippetHeader>

              <CodeBlock>{markdownSnippet}</CodeBlock>

              <PrimaryActions>
                <DitherButton
                  type="button"
                  color="blue"
                  variant="solid"
                  className="embed-primary-action"
                  onClick={() =>
                    copyToClipboard(markdownSnippet, "Markdown snippet")
                  }
                >
                  Copy markdown
                </DitherButton>
                <SecondaryLink
                  href={profileUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  View profile
                </SecondaryLink>
              </PrimaryActions>
            </SnippetSection>
          </ControlsPanel>
        </DialogBody>
      </Dialog>
    </Overlay>,
    document.body,
  );
}

const Overlay = styled.div`
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgb(0 0 0 / 0.72);
  overscroll-behavior: contain;

  @media (max-width: 640px) {
    align-items: flex-end;
    padding: 8px 0 0;
  }
`;

const Dialog = styled.div`
  display: grid;
  width: min(100%, 1040px);
  height: min(86dvh, 780px);
  max-height: calc(100dvh - 32px);
  grid-template-rows: auto minmax(0, 1fr);
  overflow: hidden;
  border: 1px solid var(--service-border-strong);
  border-radius: var(--radius);
  outline: none;
  background: var(--service-surface);
  box-shadow: 0 24px 64px rgb(0 0 0 / 0.5);
  container-type: inline-size;

  @media (max-width: 840px) {
    height: min(92dvh, 860px);
  }

  @media (max-width: 640px) {
    width: 100%;
    height: min(96dvh, 860px);
    max-height: calc(100dvh - 8px);
    border-bottom: 0;
  }
`;

const DialogHeader = styled.div`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--service-border);

  @media (max-width: 640px) {
    padding: 11px 14px;
  }
`;

const HeaderCopy = styled.div`
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
`;

const DialogTitle = styled.h2`
  overflow: hidden;
  margin: 0;
  color: var(--service-text);
  font-size: 1.0625rem;
  font-weight: 600;
  letter-spacing: -0.01em;
  line-height: 1.3;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const DialogDescription = styled.p`
  max-width: 60ch;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.4;

  @media (max-width: 480px) {
    display: none;
  }
`;

const CloseButton = styled.button`
  display: inline-flex;
  width: 32px;
  height: 32px;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: var(--radius);
  background: transparent;
  color: var(--service-text-muted);
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (hover: hover) {
    &:hover {
      border-color: var(--service-border-strong);
      color: var(--service-text);
    }
  }

  @media (pointer: coarse) {
    width: 44px;
    height: 44px;
  }
`;

const DialogBody = styled.div`
  display: grid;
  min-height: 0;
  grid-template-columns: minmax(0, 1fr) minmax(320px, 380px);
  overflow: hidden;
  scrollbar-width: thin;

  @media (max-width: 840px) {
    grid-template-columns: 1fr;
    align-content: start;
    overflow-y: auto;
    padding-bottom: max(24px, env(safe-area-inset-bottom));
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
  }
`;

const PreviewPanel = styled.div`
  display: flex;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  gap: 8px;
  overflow: hidden;
  padding: 16px;
  border-right: 1px solid var(--service-border);
  background: var(--service-canvas);

  @media (max-width: 840px) {
    overflow: visible;
    border-right: 0;
    border-bottom: 1px solid var(--service-border);
  }

  @media (max-width: 640px) {
    padding: 12px;
  }
`;

const PreviewLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.75rem;
  line-height: 1.3;
`;

const PreviewFrame = styled.div<{ $threeD: boolean }>`
  display: flex;
  min-height: 0;
  flex: 1 1 auto;
  align-items: center;
  justify-content: center;
  overflow: hidden;

  @media (max-width: 840px) {
    height: ${({ $threeD }) => ($threeD ? "320px" : "200px")};
    flex: 0 0 auto;
  }

  @media (max-width: 640px) {
    height: ${({ $threeD }) => ($threeD ? "272px" : "160px")};
  }
`;

const PreviewImage = styled.img`
  width: auto;
  max-width: 100%;
  max-height: 100%;
  height: auto;
  object-fit: contain;
`;

const PreviewMeta = styled.span`
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
  line-height: 1.3;
`;

const ControlsPanel = styled.div`
  display: flex;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  overflow-y: auto;
  padding: 6px 16px 16px;
  background: var(--service-surface);
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
  scrollbar-width: thin;

  @media (max-width: 840px) {
    overflow: visible;
    scrollbar-gutter: auto;
  }

  @media (max-width: 640px) {
    padding: 4px 14px max(16px, env(safe-area-inset-bottom));
  }

  button.embed-primary-action {
    padding-right: 1rem;
    padding-left: 1rem;
    font-size: 0.875rem;
  }
`;

const OptionRow = styled.div<{ $stacked?: boolean }>`
  display: grid;
  grid-template-columns: ${({ $stacked }) =>
    $stacked ? "1fr" : "6.5rem minmax(0, 1fr)"};
  align-items: ${({ $stacked }) => ($stacked ? "start" : "center")};
  gap: ${({ $stacked }) => ($stacked ? "8px" : "12px")};
  padding: 10px 0;
  border-bottom: 1px solid var(--service-border);
`;

const OptionLabel = styled.span`
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 500;
  line-height: 1.3;
`;

const TemplateSelect = styled(Select)`
  --select-width: 100%;
  --select-height: 32px;
  --select-font-size: 0.8125rem;
  --select-padding-x: 10px;
  --select-background: var(--service-surface-muted);
  --select-border-color: var(--service-border);
  --select-max-width: 100%;
`;

const SwatchRow = styled.div`
  display: flex;
  width: 100%;
  flex-wrap: wrap;
  gap: 8px;
`;

const Swatch = styled.button<{ $active: boolean; $color: string }>`
  position: relative;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 0;
  border-radius: var(--radius);
  background: ${({ $color }) => $color};
  box-shadow: ${({ $active }) =>
    $active
      ? "0 0 0 2px var(--service-text)"
      : "inset 0 0 0 1px rgb(255 255 255 / 0.14)"};
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    width: 40px;
    height: 40px;
  }

  @media (max-width: 640px) {
    width: 40px;
    height: 40px;
  }
`;

const SnippetSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-top: 14px;
`;

const SnippetHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
`;

const SnippetTitle = styled.h3`
  margin: 0;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 600;
  line-height: 1.3;
`;

const InlineActions = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
`;

const InlineActionButton = styled.button`
  display: inline-flex;
  min-height: 28px;
  align-items: center;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--service-text-muted);
  font: inherit;
  font-size: 0.75rem;
  text-decoration: none;
  cursor: pointer;

  &:hover {
    color: var(--service-text);
    text-decoration: underline;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }
`;

const CodeBlock = styled.pre`
  max-height: 92px;
  overflow: auto;
  margin: 0;
  padding: 10px;
  border: 1px solid var(--service-border);
  border-radius: var(--radius);
  background: var(--service-canvas);
  color: var(--service-text);
  font-size: 0.75rem;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;

  @media (max-width: 840px) {
    max-height: none;
    overflow: visible;
  }
`;

const PrimaryActions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 14px;
`;

const SecondaryLink = styled.a`
  display: inline-flex;
  min-height: 34px;
  align-items: center;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 500;
  text-decoration: none;

  &:hover {
    text-decoration: underline;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 3px;
  }

  @media (pointer: coarse) {
    min-height: 44px;
  }
`;

function CloseIcon() {
  return (
    <svg
      aria-hidden="true"
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
    >
      <path
        d="M18 6L6 18"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path
        d="M6 6L18 18"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}
