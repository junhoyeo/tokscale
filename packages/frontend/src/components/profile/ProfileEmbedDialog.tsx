"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import styled from "styled-components";
import { toast } from "react-toastify";
import {
  EMBED_TEMPLATES,
  resolvePalette,
  type EmbedTemplate,
} from "@/lib/embed/embedShared";
import { getPaletteNames, type ColorPaletteName } from "@/lib/themes";
import {
  buildEmbedPreviewPath,
  buildProfileEmbedLinks,
  getEmbedDialogCapabilities,
  type EmbedNumberFormat,
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

  useEffect(() => {
    if (!open) return;

    const previousOverflow = document.body.style.overflow;
    const previousFocus = document.activeElement as HTMLElement | null;
    const focusDialog = window.requestAnimationFrame(() => {
      dialogRef.current?.focus();
    });
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }

      if (event.key !== "Tab" || !dialogRef.current) {
        return;
      }

      const focusableElements = Array.from(
        dialogRef.current.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => element.getClientRects().length > 0);

      if (focusableElements.length === 0) {
        event.preventDefault();
        dialogRef.current.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];
      const activeElement = document.activeElement;

      if (
        event.shiftKey &&
        (activeElement === firstElement ||
          activeElement === dialogRef.current ||
          !dialogRef.current.contains(activeElement))
      ) {
        event.preventDefault();
        lastElement.focus();
      } else if (
        !event.shiftKey &&
        (activeElement === lastElement ||
          !dialogRef.current.contains(activeElement))
      ) {
        event.preventDefault();
        firstElement.focus();
      }
    };

    document.body.style.overflow = "hidden";
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      window.cancelAnimationFrame(focusDialog);
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", handleKeyDown);
      previousFocus?.focus();
    };
  }, [open, onClose]);

  const { embedUrl, markdownSnippet, htmlSnippet, profileUrl } = useMemo(
    () =>
      buildProfileEmbedLinks(username, {
        color,
        compact,
        costFormat,
        graph,
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
              Customize the live card, then copy it into GitHub or any HTML
              page.
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
            <PreviewSurface>
              <PreviewLabel>Live preview</PreviewLabel>
              <PreviewFrame $threeD={view === "3d"}>
                <PreviewImage
                  src={previewUrl}
                  alt={`Tokscale README embed preview for ${displayName || username}`}
                />
              </PreviewFrame>
            </PreviewSurface>
          </PreviewPanel>

          <ControlsPanel aria-label="Embed customization">
            <ControlsHeader>
              <ControlsTitle>Card settings</ControlsTitle>
            </ControlsHeader>

            {capabilities.showTemplate && (
              <OptionGroup>
                <OptionLabel id="embed-template-label">Template</OptionLabel>
                <SelectWrap>
                  <SelectControl
                    name="profile-embed-template"
                    aria-labelledby="embed-template-label"
                    value={template}
                    onChange={(event) =>
                      setTemplate(event.currentTarget.value as EmbedTemplate)
                    }
                  >
                    {EMBED_TEMPLATES.map((tpl) => (
                      <option key={tpl} value={tpl}>
                        {EMBED_TEMPLATE_LABELS[tpl]}
                      </option>
                    ))}
                  </SelectControl>
                  <SelectIcon aria-hidden="true" />
                </SelectWrap>
              </OptionGroup>
            )}

            <OptionGroup>
              <OptionLabel id="embed-view-label">View</OptionLabel>
              <SegmentedControl role="group" aria-labelledby="embed-view-label">
                <SegmentButton
                  type="button"
                  $active={view === "2d"}
                  aria-pressed={view === "2d"}
                  onClick={() => setView("2d")}
                >
                  2D
                </SegmentButton>
                <SegmentButton
                  type="button"
                  $active={view === "3d"}
                  aria-pressed={view === "3d"}
                  onClick={() => setView("3d")}
                >
                  3D
                </SegmentButton>
              </SegmentedControl>
            </OptionGroup>

            <OptionGroup>
              <OptionLabel id="embed-theme-label">Theme</OptionLabel>
              <SegmentedControl
                role="group"
                aria-labelledby="embed-theme-label"
              >
                <SegmentButton
                  type="button"
                  $active={theme === "dark"}
                  aria-pressed={theme === "dark"}
                  onClick={() => setTheme("dark")}
                >
                  Dark
                </SegmentButton>
                <SegmentButton
                  type="button"
                  $active={theme === "light"}
                  aria-pressed={theme === "light"}
                  onClick={() => setTheme("light")}
                >
                  Light
                </SegmentButton>
              </SegmentedControl>
            </OptionGroup>

            {capabilities.showAccent && (
              <OptionGroup $stacked>
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
              </OptionGroup>
            )}

            <OptionGroup>
              <OptionLabel id="embed-ranking-label">Ranking</OptionLabel>
              <SegmentedControl
                role="group"
                aria-labelledby="embed-ranking-label"
              >
                <SegmentButton
                  type="button"
                  $active={sortBy === "tokens"}
                  aria-pressed={sortBy === "tokens"}
                  onClick={() => setSortBy("tokens")}
                >
                  Tokens
                </SegmentButton>
                <SegmentButton
                  type="button"
                  $active={sortBy === "cost"}
                  aria-pressed={sortBy === "cost"}
                  onClick={() => setSortBy("cost")}
                >
                  Cost
                </SegmentButton>
              </SegmentedControl>
            </OptionGroup>

            {capabilities.showRankFormat && (
              <OptionGroup>
                <OptionLabel id="embed-rank-format-label">
                  Rank format
                </OptionLabel>
                <SegmentedControl
                  role="group"
                  aria-labelledby="embed-rank-format-label"
                >
                  {(["plain", "percent", "total"] as const).map((mode) => (
                    <SegmentButton
                      key={mode}
                      type="button"
                      $active={rankFormat === mode}
                      aria-pressed={rankFormat === mode}
                      onClick={() => setRankFormat(mode)}
                    >
                      {titleCase(mode)}
                    </SegmentButton>
                  ))}
                </SegmentedControl>
              </OptionGroup>
            )}

            {capabilities.showLayout && (
              <OptionGroup>
                <OptionLabel id="embed-layout-label">
                  {view === "3d" ? "Number format" : "Layout"}
                </OptionLabel>
                <SegmentedControl
                  role="group"
                  aria-labelledby="embed-layout-label"
                >
                  <SegmentButton
                    type="button"
                    $active={!compact}
                    aria-pressed={!compact}
                    onClick={() => setCompactForView(false)}
                  >
                    Full
                  </SegmentButton>
                  <SegmentButton
                    type="button"
                    $active={compact}
                    aria-pressed={compact}
                    onClick={() => setCompactForView(true)}
                  >
                    Compact
                  </SegmentButton>
                </SegmentedControl>
              </OptionGroup>
            )}

            {capabilities.showGraph && (
              <OptionGroup>
                <OptionLabel id="embed-graph-label">
                  Contribution graph
                </OptionLabel>
                <SegmentedControl
                  role="group"
                  aria-labelledby="embed-graph-label"
                >
                  <SegmentButton
                    type="button"
                    $active={!graph}
                    aria-pressed={!graph}
                    onClick={() => setGraph(false)}
                  >
                    Off
                  </SegmentButton>
                  <SegmentButton
                    type="button"
                    $active={graph}
                    aria-pressed={graph}
                    onClick={() => setGraph(true)}
                  >
                    On
                  </SegmentButton>
                </SegmentedControl>
              </OptionGroup>
            )}

            {capabilities.showNumberFormats && (
              <>
                <OptionGroup>
                  <OptionLabel id="embed-token-format-label">
                    Token format
                  </OptionLabel>
                  <SegmentedControl
                    role="group"
                    aria-labelledby="embed-token-format-label"
                  >
                    <SegmentButton
                      type="button"
                      $active={tokensFormat === "compact"}
                      aria-pressed={tokensFormat === "compact"}
                      onClick={() => setTokensFormat("compact")}
                    >
                      Compact
                    </SegmentButton>
                    <SegmentButton
                      type="button"
                      $active={tokensFormat === "full"}
                      aria-pressed={tokensFormat === "full"}
                      onClick={() => setTokensFormat("full")}
                    >
                      Full
                    </SegmentButton>
                  </SegmentedControl>
                </OptionGroup>

                <OptionGroup>
                  <OptionLabel id="embed-cost-format-label">
                    Cost format
                  </OptionLabel>
                  <SegmentedControl
                    role="group"
                    aria-labelledby="embed-cost-format-label"
                  >
                    <SegmentButton
                      type="button"
                      $active={costFormat === "compact"}
                      aria-pressed={costFormat === "compact"}
                      onClick={() => setCostFormat("compact")}
                    >
                      Compact
                    </SegmentButton>
                    <SegmentButton
                      type="button"
                      $active={costFormat === "full"}
                      aria-pressed={costFormat === "full"}
                      onClick={() => setCostFormat("full")}
                    >
                      Full
                    </SegmentButton>
                  </SegmentedControl>
                </OptionGroup>
              </>
            )}

            <SnippetSection>
              <SnippetHeader>
                <SnippetTitle>Markdown snippet</SnippetTitle>
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
                <PrimaryButton
                  type="button"
                  onClick={() =>
                    copyToClipboard(markdownSnippet, "Markdown snippet")
                  }
                >
                  Copy markdown
                </PrimaryButton>
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
  background: rgba(5, 7, 12, 0.78);
  backdrop-filter: blur(10px);
  overscroll-behavior: contain;

  @media (max-width: 640px) {
    align-items: flex-end;
    padding: 8px 0 0;
  }
`;

const Dialog = styled.div`
  display: grid;
  width: min(100%, 960px);
  height: min(84dvh, 760px);
  max-height: calc(100dvh - 32px);
  grid-template-rows: auto minmax(0, 1fr);
  overflow: hidden;
  border: 1px solid var(--service-border-strong);
  border-radius: 14px;
  outline: none;
  background: var(--service-surface);

  @media (max-width: 840px) {
    height: min(92dvh, 860px);
  }

  @media (max-width: 640px) {
    width: 100%;
    height: min(96dvh, 860px);
    max-height: calc(100dvh - 8px);
    border-bottom: 0;
    border-radius: 14px 14px 0 0;
  }
`;

const DialogHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 13px 16px;
  border-bottom: 1px solid var(--service-border);
  background: var(--service-surface);

  @media (max-width: 640px) {
    padding: 12px 14px;
  }
`;

const HeaderCopy = styled.div`
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
`;

const DialogTitle = styled.h2`
  overflow: hidden;
  margin: 0;
  color: var(--service-text);
  font-size: 1.125rem;
  font-weight: 600;
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const DialogDescription = styled.p`
  overflow: hidden;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 480px) {
    display: none;
  }
`;

const CloseButton = styled.button`
  display: inline-flex;
  width: 34px;
  height: 34px;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  background: var(--service-surface-muted);
  color: var(--service-text-muted);
  transition:
    border-color 150ms ease,
    background-color 150ms ease,
    color 150ms ease;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (hover: hover) {
    &:hover {
      background: var(--service-accent-soft);
      color: var(--service-text);
    }
  }

  @media (pointer: coarse) {
    width: 44px;
    height: 44px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const DialogBody = styled.div`
  display: grid;
  min-height: 0;
  grid-template-columns: minmax(0, 1fr) minmax(320px, 360px);
  overflow: hidden;
  scrollbar-color: color-mix(
      in srgb,
      var(--service-text-muted) 52%,
      transparent
    )
    var(--service-canvas);
  scrollbar-width: thin;

  &::-webkit-scrollbar {
    width: 9px;
  }

  &::-webkit-scrollbar-track {
    background: var(--service-canvas);
  }

  &::-webkit-scrollbar-thumb {
    border: 2px solid var(--service-canvas);
    border-radius: 999px;
    background: color-mix(in srgb, var(--service-text-muted) 52%, transparent);
  }

  @media (max-width: 840px) {
    grid-template-columns: 1fr;
    align-content: start;
    overflow-y: auto;
    padding-bottom: max(24px, env(safe-area-inset-bottom));
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
    scroll-padding-bottom: max(24px, env(safe-area-inset-bottom));
  }

  @media (max-width: 640px) {
    padding-bottom: max(32px, calc(env(safe-area-inset-bottom) + 16px));
    scroll-padding-bottom: max(32px, calc(env(safe-area-inset-bottom) + 16px));
  }
`;

const PreviewPanel = styled.div`
  display: flex;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
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

const PreviewSurface = styled.div`
  display: flex;
  min-height: 0;
  flex: 1 1 auto;
  flex-direction: column;
  gap: 8px;
`;

const PreviewLabel = styled.span`
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 600;
  line-height: 1.3;
`;

const PreviewFrame = styled.div<{ $threeD: boolean }>`
  display: flex;
  min-height: 0;
  flex: 1 1 auto;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  padding: 20px 0;

  @media (max-width: 840px) {
    height: ${({ $threeD }) => ($threeD ? "320px" : "200px")};
    flex: 0 0 auto;
  }

  @media (max-width: 640px) {
    height: ${({ $threeD }) => ($threeD ? "272px" : "160px")};
    padding: 8px 0;
  }
`;

const PreviewImage = styled.img`
  width: auto;
  max-width: 100%;
  max-height: 100%;
  height: auto;
  object-fit: contain;
`;

const ControlsPanel = styled.div`
  display: flex;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  overflow-y: auto;
  padding: 14px 16px 16px;
  background: var(--service-surface);
  overscroll-behavior: contain;
  scrollbar-color: color-mix(
      in srgb,
      var(--service-text-muted) 52%,
      transparent
    )
    var(--service-canvas);
  scrollbar-gutter: stable;
  scrollbar-width: thin;

  &::-webkit-scrollbar {
    width: 9px;
  }

  &::-webkit-scrollbar-track {
    border-left: 1px solid var(--service-border);
    background: var(--service-canvas);
  }

  &::-webkit-scrollbar-thumb {
    border: 2px solid var(--service-canvas);
    border-radius: 999px;
    background: color-mix(in srgb, var(--service-text-muted) 52%, transparent);
  }

  @media (max-width: 840px) {
    overflow: visible;
    scrollbar-gutter: auto;
  }

  @media (max-width: 640px) {
    padding: 12px 14px max(16px, env(safe-area-inset-bottom));
  }
`;

const ControlsHeader = styled.div`
  display: flex;
  align-items: center;
  padding-bottom: 9px;
  border-bottom: 1px solid var(--service-border);
`;

const ControlsTitle = styled.h3`
  margin: 0;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 600;
  line-height: 1.3;
`;

const OptionGroup = styled.div<{ $stacked?: boolean }>`
  display: grid;
  grid-template-columns: ${({ $stacked }) =>
    $stacked ? "1fr" : "7rem minmax(0, 1fr)"};
  align-items: ${({ $stacked }) => ($stacked ? "start" : "center")};
  gap: ${({ $stacked }) => ($stacked ? "7px" : "10px")};
  padding: 8px 0;
  border-bottom: 1px solid var(--service-border);

  @media (max-width: 400px) {
    grid-template-columns: ${({ $stacked }) =>
      $stacked ? "1fr" : "6.25rem minmax(0, 1fr)"};
    gap: ${({ $stacked }) => ($stacked ? "7px" : "8px")};
  }
`;

const OptionLabel = styled.span`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
  line-height: 1.3;
`;

const SelectWrap = styled.div`
  position: relative;
  min-width: 0;
`;

const SelectControl = styled.select`
  width: 100%;
  min-height: 34px;
  appearance: none;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  padding: 6px 30px 6px 10px;
  background: var(--service-surface-muted);
  color: var(--service-text);
  font: inherit;
  font-size: 0.8125rem;
  line-height: 1.3;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    min-height: 44px;
    font-size: 1rem;
  }
`;

const SelectIcon = styled.span`
  position: absolute;
  top: 50%;
  right: 12px;
  width: 7px;
  height: 7px;
  border-right: 1.5px solid var(--service-text-muted);
  border-bottom: 1.5px solid var(--service-text-muted);
  pointer-events: none;
  transform: translateY(-70%) rotate(45deg);
`;

const SegmentedControl = styled.div`
  display: inline-flex;
  width: fit-content;
  max-width: 100%;
  flex-wrap: wrap;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--service-border);
  border-radius: 8px;
  background: var(--service-surface-muted);
`;

const SegmentButton = styled.button<{ $active: boolean }>`
  display: inline-flex;
  min-height: 28px;
  align-items: center;
  justify-content: center;
  border: 1px solid
    ${({ $active }) =>
      $active ? "var(--service-border-strong)" : "transparent"};
  border-radius: 6px;
  padding: 4px 8px;
  background: ${({ $active }) =>
    $active ? "var(--service-accent-soft)" : "transparent"};
  color: ${({ $active }) =>
    $active ? "var(--service-text)" : "var(--service-text-muted)"};
  font: inherit;
  font-size: 0.75rem;
  font-weight: 550;
  line-height: 1;
  transition:
    border-color 150ms ease,
    background-color 150ms ease,
    color 150ms ease,
    transform 120ms ease;

  &:active {
    transform: translateY(1px);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }

  @media (hover: hover) {
    &:hover {
      color: var(--service-text);
    }
  }

  @media (pointer: coarse) {
    min-height: 44px;
    padding-right: 12px;
    padding-left: 12px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const SwatchRow = styled.div`
  display: grid;
  width: 100%;
  grid-template-columns: repeat(10, 28px);
  justify-content: space-between;
  gap: 0;

  @media (pointer: coarse) {
    width: fit-content;
    grid-template-columns: repeat(5, 44px);
    justify-content: start;
    gap: 8px;
  }

  @media (max-width: 640px) {
    width: fit-content;
    grid-template-columns: repeat(5, 44px);
    justify-content: start;
    gap: 8px;
  }
`;

const Swatch = styled.button<{ $active: boolean; $color: string }>`
  width: 28px;
  height: 28px;
  border: 2px solid var(--service-surface);
  border-radius: 7px;
  background: ${({ $color }) => $color};
  box-shadow:
    0 0 0 1px
      ${({ $active }) =>
        $active ? "var(--service-text)" : "var(--service-border-strong)"},
    inset 0 0 0 1px rgba(0, 0, 0, 0.18);
  cursor: pointer;
  transition: transform 120ms ease;

  &:active {
    transform: translateY(1px);
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    width: 44px;
    height: 44px;
  }

  @media (max-width: 640px) {
    width: 44px;
    height: 44px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const SnippetSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-top: 10px;
  padding: 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 10px;
  background: var(--service-canvas);
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
  gap: 5px;
`;

const InlineActionButton = styled.button`
  display: inline-flex;
  min-height: 28px;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--service-border-strong);
  border-radius: 7px;
  padding: 4px 8px;
  background: var(--service-surface-muted);
  color: var(--service-text-muted);
  font: inherit;
  font-size: 0.75rem;
  font-weight: 550;
  line-height: 1;
  transition:
    border-color 150ms ease,
    background-color 150ms ease,
    color 150ms ease;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (hover: hover) {
    &:hover {
      background: var(--service-accent-soft);
      color: var(--service-text);
    }
  }

  @media (pointer: coarse) {
    min-height: 44px;
    padding-right: 12px;
    padding-left: 12px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const CodeBlock = styled.pre`
  max-height: 92px;
  overflow: auto;
  margin: 0;
  padding: 10px;
  border: 1px solid var(--service-border);
  border-radius: 8px;
  background: #080b11;
  color: var(--service-text);
  font-family: var(--font-mono), ui-monospace, monospace;
  font-size: 0.6875rem;
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
  gap: 7px;
`;

const PrimaryButton = styled.button`
  display: inline-flex;
  min-height: 34px;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--service-accent);
  border-radius: 8px;
  padding: 7px 11px;
  background: var(--service-accent);
  color: var(--service-accent-foreground);
  font: inherit;
  font-size: 0.75rem;
  font-weight: 650;
  line-height: 1;
  transition:
    border-color 150ms ease,
    background-color 150ms ease;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (hover: hover) {
    &:hover {
      border-color: var(--service-accent-hover);
      background: var(--service-accent-hover);
    }
  }

  @media (pointer: coarse) {
    min-height: 44px;
    padding-right: 14px;
    padding-left: 14px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

const SecondaryLink = styled.a`
  display: inline-flex;
  min-height: 34px;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--service-border-strong);
  border-radius: 8px;
  padding: 7px 11px;
  background: var(--service-surface-muted);
  color: var(--service-text);
  font-size: 0.75rem;
  font-weight: 550;
  line-height: 1;
  text-decoration: none;
  transition:
    border-color 150ms ease,
    background-color 150ms ease;

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (hover: hover) {
    &:hover {
      background: var(--service-accent-soft);
    }
  }

  @media (pointer: coarse) {
    min-height: 44px;
    padding-right: 14px;
    padding-left: 14px;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
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
