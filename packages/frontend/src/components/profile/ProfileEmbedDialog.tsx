"use client";

import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import styled from "styled-components";
import { toast } from "react-toastify";

type EmbedTheme = "dark" | "light";
type EmbedSortBy = "tokens" | "cost";

interface ProfileEmbedDialogProps {
  open: boolean;
  username: string;
  displayName?: string | null;
  onClose: () => void;
}

const TOKSCALE_URL = "https://tokscale.ai";

export function ProfileEmbedDialog({
  open,
  username,
  displayName,
  onClose,
}: ProfileEmbedDialogProps) {
  const [theme, setTheme] = useState<EmbedTheme>("dark");
  const [sortBy, setSortBy] = useState<EmbedSortBy>("tokens");
  const [compact, setCompact] = useState(false);

  useEffect(() => {
    if (!open) return;

    const previousOverflow = document.body.style.overflow;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };

    document.body.style.overflow = "hidden";
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, onClose]);

  const {
    embedUrl,
    previewUrl,
    markdownSnippet,
    htmlSnippet,
    profileUrl,
  } = useMemo(() => {
    const params = new URLSearchParams();

    if (theme !== "dark") params.set("theme", theme);
    if (sortBy !== "tokens") params.set("sort", sortBy);
    if (compact) params.set("compact", "1");

    const query = params.toString();
    const baseEmbedUrl = `${TOKSCALE_URL}/api/embed/${username}/svg`;
    const resolvedEmbedUrl = query ? `${baseEmbedUrl}?${query}` : baseEmbedUrl;
    const resolvedProfileUrl = `${TOKSCALE_URL}/u/${username}`;

    return {
      embedUrl: resolvedEmbedUrl,
      previewUrl: `${resolvedEmbedUrl}${query ? "&" : "?"}preview=1`,
      markdownSnippet: `[![Tokscale Stats](${resolvedEmbedUrl})](${resolvedProfileUrl})`,
      htmlSnippet: `<a href="${resolvedProfileUrl}"><img alt="Tokscale Stats for @${username}" src="${resolvedEmbedUrl}" /></a>`,
      profileUrl: resolvedProfileUrl,
    };
  }, [compact, sortBy, theme, username]);

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
    <Overlay onClick={(event) => {
      if (event.target === event.currentTarget) onClose();
    }}>
      <Dialog
        role="dialog"
        aria-modal="true"
        aria-labelledby="profile-embed-dialog-title"
      >
        <DialogHeader>
          <HeaderCopy>
            <Eyebrow>GitHub README embed</Eyebrow>
            <DialogTitle id="profile-embed-dialog-title">
              Share @{username} with a polished Tokscale card
            </DialogTitle>
            <DialogDescription>
              Preview the live embed, tweak the presentation, and copy a ready-to-paste snippet for your README.
            </DialogDescription>
          </HeaderCopy>

          <CloseButton type="button" onClick={onClose} aria-label="Close embed dialog">
            <CloseIcon />
          </CloseButton>
        </DialogHeader>

        <DialogBody>
          <PreviewPanel>
            <PreviewSurface>
              <PreviewLabel>Live preview</PreviewLabel>
              <PreviewFrame>
                <PreviewImage
                  src={previewUrl}
                  alt={`Tokscale README embed preview for ${displayName || username}`}
                />
              </PreviewFrame>
            </PreviewSurface>

            <HighlightsList>
              <HighlightItem>GitHub-ready markdown with a linked image card</HighlightItem>
              <HighlightItem>Matches the new Tokscale 2.0 visual language</HighlightItem>
              <HighlightItem>Automatically refreshes as profile stats update</HighlightItem>
            </HighlightsList>
          </PreviewPanel>

          <ControlsPanel>
            <OptionGroup>
              <OptionLabel>Theme</OptionLabel>
              <SegmentedControl>
                <SegmentButton
                  type="button"
                  $active={theme === "dark"}
                  onClick={() => setTheme("dark")}
                >
                  Dark
                </SegmentButton>
                <SegmentButton
                  type="button"
                  $active={theme === "light"}
                  onClick={() => setTheme("light")}
                >
                  Light
                </SegmentButton>
              </SegmentedControl>
            </OptionGroup>

            <OptionGroup>
              <OptionLabel>Ranking</OptionLabel>
              <SegmentedControl>
                <SegmentButton
                  type="button"
                  $active={sortBy === "tokens"}
                  onClick={() => setSortBy("tokens")}
                >
                  Tokens
                </SegmentButton>
                <SegmentButton
                  type="button"
                  $active={sortBy === "cost"}
                  onClick={() => setSortBy("cost")}
                >
                  Cost
                </SegmentButton>
              </SegmentedControl>
            </OptionGroup>

            <OptionGroup>
              <OptionLabel>Layout</OptionLabel>
              <SegmentedControl>
                <SegmentButton
                  type="button"
                  $active={!compact}
                  onClick={() => setCompact(false)}
                >
                  Full
                </SegmentButton>
                <SegmentButton
                  type="button"
                  $active={compact}
                  onClick={() => setCompact(true)}
                >
                  Compact
                </SegmentButton>
              </SegmentedControl>
            </OptionGroup>

            <SnippetSection>
              <SnippetHeader>
                <SnippetTitle>Markdown snippet</SnippetTitle>
                <InlineActions>
                  <InlineActionButton type="button" onClick={() => copyToClipboard(embedUrl, "Image URL")}>
                    Copy image URL
                  </InlineActionButton>
                  <InlineActionButton type="button" onClick={() => copyToClipboard(htmlSnippet, "HTML snippet")}>
                    Copy HTML
                  </InlineActionButton>
                </InlineActions>
              </SnippetHeader>

              <CodeBlock>{markdownSnippet}</CodeBlock>

              <PrimaryActions>
                <PrimaryButton type="button" onClick={() => copyToClipboard(markdownSnippet, "Markdown snippet")}>
                  Copy markdown
                </PrimaryButton>
                <SecondaryLink href={profileUrl} target="_blank" rel="noopener noreferrer">
                  View profile
                </SecondaryLink>
              </PrimaryActions>
            </SnippetSection>
          </ControlsPanel>
        </DialogBody>
      </Dialog>
    </Overlay>,
    document.body
  );
}

const Overlay = styled.div`
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background:
    radial-gradient(circle at top, rgba(22, 154, 255, 0.18), transparent 32%),
    rgba(6, 10, 18, 0.82);
  backdrop-filter: blur(18px);

  @media (max-width: 640px) {
    align-items: flex-end;
    padding: 12px;
  }
`;

const Dialog = styled.div`
  width: min(100%, 1040px);
  max-height: min(88vh, 920px);
  overflow: auto;
  border: 1px solid rgba(133, 202, 255, 0.16);
  border-radius: 28px;
  background:
    radial-gradient(circle at top right, rgba(22, 154, 255, 0.16), transparent 30%),
    linear-gradient(180deg, rgba(26, 33, 42, 0.98) 0%, rgba(17, 17, 19, 0.98) 100%);
  box-shadow:
    0 30px 80px rgba(0, 0, 0, 0.55),
    inset 0 1px 0 rgba(255, 255, 255, 0.04);

  @media (max-width: 640px) {
    width: 100%;
    max-height: 92vh;
    border-radius: 24px 24px 0 0;
  }
`;

const DialogHeader = styled.div`
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 28px 28px 0;

  @media (max-width: 640px) {
    padding: 22px 18px 0;
  }
`;

const HeaderCopy = styled.div`
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
`;

const Eyebrow = styled.span`
  display: inline-flex;
  width: fit-content;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border: 1px solid rgba(133, 202, 255, 0.18);
  border-radius: 999px;
  background: rgba(133, 202, 255, 0.08);
  color: var(--color-accent-blue);
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
`;

const DialogTitle = styled.h2`
  color: var(--color-fg-default);
  font-size: clamp(28px, 3vw, 38px);
  font-weight: 700;
  line-height: 1.05;
  letter-spacing: -0.04em;
`;

const DialogDescription = styled.p`
  max-width: 720px;
  color: var(--color-fg-muted);
  font-size: 15px;
  line-height: 1.6;
`;

const CloseButton = styled.button`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  flex-shrink: 0;
  border: 1px solid var(--color-border-default);
  border-radius: 999px;
  background: rgba(32, 41, 50, 0.82);
  color: var(--color-fg-default);
  transition:
    transform 150ms ease,
    border-color 150ms ease,
    background 150ms ease;

  &:hover {
    transform: translateY(-1px);
    border-color: rgba(133, 202, 255, 0.28);
    background: rgba(32, 41, 50, 1);
  }
`;

const DialogBody = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1.1fr) minmax(320px, 420px);
  gap: 24px;
  padding: 28px;

  @media (max-width: 920px) {
    grid-template-columns: 1fr;
  }

  @media (max-width: 640px) {
    gap: 18px;
    padding: 18px;
  }
`;

const PreviewPanel = styled.div`
  display: flex;
  flex-direction: column;
  gap: 16px;
`;

const PreviewSurface = styled.div`
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-height: 100%;
  padding: 20px;
  border: 1px solid rgba(133, 202, 255, 0.12);
  border-radius: 24px;
  background:
    linear-gradient(180deg, rgba(16, 18, 28, 0.98) 0%, rgba(10, 12, 18, 0.98) 100%);
`;

const PreviewLabel = styled.span`
  color: var(--color-fg-muted);
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
`;

const PreviewFrame = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 360px;
  padding: 24px;
  border: 1px dashed rgba(133, 202, 255, 0.16);
  border-radius: 18px;
  background:
    linear-gradient(135deg, rgba(22, 154, 255, 0.06) 0%, transparent 35%),
    rgba(255, 255, 255, 0.02);

  @media (max-width: 640px) {
    min-height: 220px;
    padding: 16px;
  }
`;

const PreviewImage = styled.img`
  width: 100%;
  max-width: 100%;
  height: auto;
  filter: drop-shadow(0 22px 48px rgba(0, 0, 0, 0.36));
`;

const HighlightsList = styled.ul`
  display: grid;
  gap: 10px;
  padding: 0;
  margin: 0;
  list-style: none;
`;

const HighlightItem = styled.li`
  position: relative;
  padding-left: 18px;
  color: var(--color-fg-muted);
  font-size: 14px;
  line-height: 1.5;

  &::before {
    content: "";
    position: absolute;
    left: 0;
    top: 8px;
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: linear-gradient(135deg, #169aff 0%, #85caff 100%);
    box-shadow: 0 0 12px rgba(22, 154, 255, 0.5);
  }
`;

const ControlsPanel = styled.div`
  display: flex;
  flex-direction: column;
  gap: 16px;
`;

const OptionGroup = styled.div`
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 16px;
  border: 1px solid var(--color-border-default);
  border-radius: 20px;
  background: rgba(16, 18, 28, 0.68);
`;

const OptionLabel = styled.span`
  color: var(--color-fg-default);
  font-size: 14px;
  font-weight: 600;
`;

const SegmentedControl = styled.div`
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
`;

const SegmentButton = styled.button<{ $active: boolean }>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 40px;
  padding: 10px 14px;
  border: 1px solid ${({ $active }) => $active ? "rgba(133, 202, 255, 0.24)" : "var(--color-border-default)"};
  border-radius: 999px;
  background: ${({ $active }) =>
    $active
      ? "linear-gradient(135deg, rgba(22, 154, 255, 0.18) 0%, rgba(133, 202, 255, 0.1) 100%)"
      : "rgba(32, 41, 50, 0.8)"};
  color: ${({ $active }) => $active ? "var(--color-fg-default)" : "var(--color-fg-muted)"};
  font-size: 14px;
  font-weight: 600;
  transition:
    transform 150ms ease,
    border-color 150ms ease,
    color 150ms ease,
    background 150ms ease;

  &:hover {
    transform: translateY(-1px);
    color: var(--color-fg-default);
    border-color: rgba(133, 202, 255, 0.24);
  }
`;

const SnippetSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px;
  border: 1px solid rgba(133, 202, 255, 0.16);
  border-radius: 24px;
  background:
    linear-gradient(180deg, rgba(26, 33, 42, 0.92) 0%, rgba(17, 18, 24, 0.92) 100%);
`;

const SnippetHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
`;

const SnippetTitle = styled.h3`
  color: var(--color-fg-default);
  font-size: 15px;
  font-weight: 700;
`;

const InlineActions = styled.div`
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
`;

const InlineActionButton = styled.button`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 34px;
  padding: 8px 12px;
  border: 1px solid var(--color-border-default);
  border-radius: 999px;
  background: rgba(32, 41, 50, 0.68);
  color: var(--color-fg-muted);
  font-size: 13px;
  font-weight: 600;
  transition:
    border-color 150ms ease,
    color 150ms ease,
    background 150ms ease;

  &:hover {
    color: var(--color-fg-default);
    border-color: rgba(133, 202, 255, 0.2);
    background: rgba(32, 41, 50, 0.92);
  }
`;

const CodeBlock = styled.pre`
  overflow: auto;
  margin: 0;
  padding: 16px;
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 18px;
  background: rgba(8, 12, 18, 0.92);
  color: #d9edff;
  font-family: var(--font-mono), ui-monospace, monospace;
  font-size: 13px;
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
`;

const PrimaryActions = styled.div`
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
`;

const PrimaryButton = styled.button`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 44px;
  padding: 12px 16px;
  border: 1px solid rgba(133, 202, 255, 0.26);
  border-radius: 999px;
  background: linear-gradient(135deg, #169aff 0%, #0073ff 100%);
  color: #fff;
  font-size: 14px;
  font-weight: 700;
  transition:
    transform 150ms ease,
    filter 150ms ease;

  &:hover {
    transform: translateY(-1px);
    filter: brightness(1.06);
  }
`;

const SecondaryLink = styled.a`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 44px;
  padding: 12px 16px;
  border: 1px solid var(--color-border-default);
  border-radius: 999px;
  background: rgba(32, 41, 50, 0.68);
  color: var(--color-fg-default);
  font-size: 14px;
  font-weight: 600;
  text-decoration: none;
  transition:
    transform 150ms ease,
    border-color 150ms ease;

  &:hover {
    transform: translateY(-1px);
    border-color: rgba(133, 202, 255, 0.22);
  }
`;

function CloseIcon() {
  return (
    <svg aria-hidden="true" width="20" height="20" viewBox="0 0 24 24" fill="none">
      <path d="M18 6L6 18" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      <path d="M6 6L18 18" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
