"use client";

import { useState, useEffect, useCallback, useId, useMemo } from "react";
import { useRouter } from "nextjs-toploader/app";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";
import { DitherDialog } from "dither-kit/DitherDialog";
import { DitherInput } from "dither-kit/DitherInput";
import { DitherAvatar } from "dither-kit/DitherAvatar";
import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import { Select } from "@/components/ui/Select";
import { deviceDisplayLabel } from "@/lib/devices/shared";
import { formatNumber, formatCurrency } from "@/lib/utils";
import { formatRelativeTime } from "@/lib/format";
import { useSettings } from "@/lib/useSettings";
import { COMMON_TIMEZONE_GROUPS, getBrowserTimeZone } from "@/lib/timezone";

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

interface CreatedApiToken extends ApiToken {
  token: string;
}

// Subset of GET /api/users/[username]/devices we render here. That public
// endpoint already aggregates usage per device, so settings reuses it with
// the session user's username instead of adding a private listing route.
interface SettingsDevice {
  id: string;
  deviceKey: string;
  /** Resolved label (custom name or fallback) — what we render. */
  displayName: string;
  /** Raw user-set name (null = never renamed) — what we edit. */
  customName: string | null;
  lastSubmittedAt: string | null;
  totalTokens: number;
  totalCost: number;
  activeDays: number;
}

// Mirror the server-side RenameBodySchema in
// /api/settings/devices/[deviceId]/route.ts (varchar(120), no control chars).
const DEVICE_NAME_MAX_LENGTH = 120;
const DEVICE_NAME_CONTROL_CHARS = /\p{C}/u;

function validateDeviceName(name: string): string | null {
  if (name.length > DEVICE_NAME_MAX_LENGTH) {
    return `Device name must be ${DEVICE_NAME_MAX_LENGTH} characters or fewer`;
  }
  if (DEVICE_NAME_CONTROL_CHARS.test(name)) {
    return "Device name must not contain control characters";
  }
  return null;
}

// ============================================================================
// Page frame
// ============================================================================

const PageWrapper = styled.div`
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--service-canvas);
  color: var(--service-text);
`;

/* Clears the fixed navigation, like .service-main on the other pages. */
const MainContent = styled.main`
  flex: 1;
  width: 100%;
  max-width: 1152px;
  margin: 0 auto;
  padding: 112px 32px 64px;

  @media (max-width: 640px) {
    padding: 96px 20px 48px;
  }
`;

const LoadingMain = styled.main`
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--service-text-muted);
  font-size: 0.9375rem;
`;

const PageHead = styled.header`
  margin-bottom: 40px;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.75rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
`;

const PageLead = styled.p`
  margin: 10px 0 0;
  max-width: 66ch;
  color: var(--service-text-muted);
  font-size: 1rem;
  line-height: 1.6;
`;

// ============================================================================
// Section: a label column beside its panel, both on one pair of rules
// ============================================================================

const SectionRow = styled.section`
  display: grid;
  grid-template-columns: minmax(0, 420px) minmax(0, 1fr);
  gap: 48px;
  padding: 32px 0;
  border-top: 1px solid var(--service-border);

  &:last-of-type {
    border-bottom: 1px solid var(--service-border);
  }

  @media (max-width: 900px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 16px;
  }
`;

const LabelColumn = styled.div`
  min-width: 0;
`;

const SectionTitle = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
`;

const SectionDescription = styled.p`
  margin: 8px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.6;
`;

const Panel = styled.div`
  display: grid;
  gap: 16px;
  min-width: 0;
  padding: 20px;
  border-radius: 2px;
  background: var(--service-surface);
`;

const FieldLabel = styled.label`
  display: block;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
`;

const FieldHint = styled.p`
  margin: 6px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px 16px;
`;

const InlineForm = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: end;
  gap: 12px;

  @media (max-width: 560px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const NameField = styled(DitherInput)`
  height: 40px;
  border-radius: 2px;
  font-size: 0.875rem;
`;

const TimezoneSelect = styled(Select)`
  --select-height: 40px;
  --select-max-width: 360px;
  --select-padding-x: 12px;
  --select-radius: 2px;
  --select-background: var(--service-surface-muted);
  --select-border-color: var(--service-border-strong);
  --select-font-size: 0.875rem;
`;

/* Bordered control for a section's Save: visible enough to press, not filled,
   so the screen keeps its one primary action. */
const SaveButton = styled.button`
  min-height: 36px;
  padding: 0 14px;
  border: 1px solid var(--service-border-strong);
  border-radius: 2px;
  background: transparent;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 500;

  &:hover:not(:disabled) {
    border-color: var(--service-text-muted);
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const TextButton = styled.button`
  min-height: 32px;
  padding: 0 2px;
  border: 0;
  background: transparent;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 500;
  text-decoration: none;
  white-space: nowrap;

  &:hover {
    text-decoration: underline;
    text-underline-offset: 4px;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
`;

const ErrorText = styled.p`
  margin: 0;
  color: #ff9b9b;
  font-size: 0.8125rem;
  line-height: 1.5;
`;

const Muted = styled.p`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.6;
`;

const VisuallyHidden = styled.span`
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
  border: 0;
`;

// ============================================================================
// Profile
// ============================================================================

const ProfileRow = styled.div`
  display: flex;
  align-items: center;
  gap: 16px;
`;

const Avatar = styled.img`
  width: 48px;
  height: 48px;
  flex: 0 0 auto;
  border-radius: 2px;
  object-fit: cover;
`;

/* A real GitHub avatar stays a picture; when there is none (or it fails to
   load) the deterministic dither avatar stands in, never a broken image. */
function ProfileAvatar({ src, name }: { src: string | null; name: string }) {
  const [failed, setFailed] = useState(false);

  if (src && !failed) {
    return (
      <Avatar
        src={src}
        alt={name}
        width={48}
        height={48}
        loading="lazy"
        onError={() => setFailed(true)}
      />
    );
  }

  return <DitherAvatar name={name} color="blue" size={48} grid={8} cellPx={4} animate={false} bloom="off" />;
}

const ProfileName = styled.p`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 500;
`;

const ProfileMeta = styled.p`
  margin: 2px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
  overflow-wrap: anywhere;
`;

// ============================================================================
// Token reveal
// ============================================================================

const Reveal = styled.div`
  display: grid;
  gap: 8px;
  padding: 12px 14px;
  border: 1px solid var(--service-border-strong);
  border-radius: 2px;
  background: var(--service-surface-muted);
`;

const RevealTitle = styled.p`
  margin: 0;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 600;
`;

const RevealRow = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 8px;
`;

const TokenCode = styled.code`
  display: block;
  overflow-x: auto;
  padding: 8px 10px;
  border-radius: 2px;
  background: var(--service-canvas);
  color: var(--service-text);
  font-size: 0.8125rem;
  white-space: nowrap;
`;

// ============================================================================
// Lists (tokens, devices)
// ============================================================================

const List = styled.ul`
  display: grid;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const ListRow = styled.li<{ $editing?: boolean }>`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px 16px;
  min-width: 0;
  padding: 12px 0 12px 12px;
  border-top: 1px solid var(--service-border);
  border-left: 1px solid ${({ $editing }) => ($editing ? "var(--service-accent)" : "transparent")};
  margin-left: -12px;
  font-variant-numeric: tabular-nums;

  &:first-child {
    border-top: 0;
  }
`;

const RowMain = styled.div`
  min-width: 0;
`;

const RowName = styled.p`
  margin: 0;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
`;

const RowMeta = styled.p`
  margin: 2px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
`;

const DeviceEditRow = styled.div`
  display: grid;
  width: 100%;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 8px;

  @media (max-width: 560px) {
    grid-template-columns: minmax(0, 1fr) auto;
  }
`;

// ============================================================================
// Danger zone
// ============================================================================

const DangerRow = styled.div`
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 8px 16px;
  padding: 16px 0;

  & + & {
    border-top: 1px solid var(--service-border);
  }

  @media (max-width: 560px) {
    grid-template-columns: minmax(0, 1fr);
    justify-items: start;
  }
`;

const DangerTitle = styled.p`
  margin: 0;
  color: var(--service-text);
  font-size: 0.875rem;
  font-weight: 500;
`;

const DangerDescription = styled.p`
  margin: 2px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
`;

const DangerButton = styled.button`
  min-height: 36px;
  padding: 0 14px;
  border: 1px solid rgba(255, 107, 107, 0.55);
  border-radius: 2px;
  background: transparent;
  color: #ff9b9b;
  font-size: 0.8125rem;
  font-weight: 500;
  white-space: nowrap;

  &:hover {
    border-color: #ff9b9b;
    color: #ffb3b3;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

/* A paragraph of the confirmation's story: steps sit under each other with one
   gap, so an inline <strong> never breaks onto its own line. */
const DialogBody = styled.p`
  margin: 0 0 12px;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.6;
`;

const DialogList = styled.ul`
  margin: 0 0 12px;
  padding: 0 0 0 20px;
  list-style: disc;

  li {
    margin-bottom: 4px;
  }

  li::marker {
    color: var(--service-text-muted);
  }
`;

const DialogStrong = styled.strong`
  color: var(--service-text);
  font-weight: 600;
`;

const StepDots = styled.div`
  display: flex;
  gap: 6px;
  margin-bottom: 14px;
`;

const StepDot = styled.span<{ $active: boolean }>`
  width: 6px;
  height: 6px;
  background: ${({ $active }) => ($active ? "var(--service-text)" : "var(--service-border-strong)")};
`;

/* The kit draws its dialog with Tailwind utilities that live in a cascade
   layer; this app's reset is unlayered, so the header lost its row. These
   unlayered rules restore the layout and put our type, rule and radius on it. */
const ConfirmationDialog = styled(DitherDialog)`
  width: min(480px, calc(100vw - 32px));
  border: 1px solid var(--service-border-strong);
  border-radius: 2px;
  background: var(--service-surface);
  color: var(--service-text);
  font-family: var(--v2-font-ui);

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 12px 14px 16px;
    border-bottom: 1px solid var(--service-border);
  }

  header h2 {
    margin: 0;
    color: var(--service-text);
    font-family: var(--v2-font-ui);
    font-size: 1rem;
    font-weight: 600;
  }

  header button {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 2px;
    background: transparent;
    color: var(--service-text-muted);
    font-size: 1.125rem;
    line-height: 1;

    &:hover {
      color: var(--service-text);
    }
  }

  > div {
    padding: 16px;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid var(--service-border);
  }
`;

const DialogActions = styled.div`
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
`;

const DialogCancel = styled.button`
  min-height: 36px;
  padding: 0 14px;
  border: 1px solid var(--service-border-strong);
  border-radius: 2px;
  background: transparent;
  color: var(--service-text);
  font-size: 0.8125rem;
  font-weight: 500;

  &:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }
`;

const DialogConfirm = styled(DitherButton)`
  min-height: 36px;
  padding: 0 14px;
  border-radius: 2px;
  font-size: 0.8125rem;
`;

// ============================================================================
// Confirmation dialog
// ============================================================================

type DangerAction = "delete-data" | "delete-account";

interface ConfirmationConfig {
  title: string;
  steps: Array<{
    body: React.ReactNode;
    confirmLabel: string;
  }>;
  typedConfirmation: string;
  onConfirm: () => Promise<void>;
}

const CONFIRMATION_CONFIGS: Record<DangerAction, ConfirmationConfig> = {
  "delete-data": {
    title: "Delete submitted data",
    steps: [
      {
        body: (
          <>
            <DialogBody>This permanently removes the usage data you have submitted:</DialogBody>
            <DialogList>
              <li>Leaderboard entries</li>
              <li>Public profile stats</li>
              <li>Daily usage history</li>
            </DialogList>
            <DialogBody>
              Your account and API tokens stay active. You can submit new data at any time.
            </DialogBody>
          </>
        ),
        confirmLabel: "I want to delete my data",
      },
      {
        body: (
          <DialogBody>
            This cannot be undone. All your historical token usage and cost data is erased from the
            leaderboard and your public profile.
          </DialogBody>
        ),
        confirmLabel: "I understand, continue",
      },
    ],
    typedConfirmation: "delete my data",
    onConfirm: async () => {
      const res = await fetch("/api/settings/submitted-data", { method: "DELETE" });
      if (!res.ok) throw new Error("Failed to delete submitted data");
    },
  },
  "delete-account": {
    title: "Delete account",
    steps: [
      {
        body: (
          <>
            <DialogBody>This permanently deletes your account and everything attached to it:</DialogBody>
            <DialogList>
              <li>User profile</li>
              <li>All submitted usage data</li>
              <li>Leaderboard entries</li>
              <li>API tokens and active sessions</li>
            </DialogList>
            <DialogBody>You are signed out immediately. This cannot be reversed.</DialogBody>
          </>
        ),
        confirmLabel: "I want to delete my account",
      },
      {
        body: (
          <DialogBody>
            This is <DialogStrong>permanent and irreversible</DialogStrong>. Your username becomes
            available for others to register, and all your data (submissions, tokens, sessions) is
            wiped.
          </DialogBody>
        ),
        confirmLabel: "I understand, continue",
      },
    ],
    typedConfirmation: "delete my account",
    onConfirm: async () => {
      const res = await fetch("/api/settings/account", { method: "DELETE" });
      if (!res.ok) throw new Error("Failed to delete account");
    },
  },
};

function DangerConfirmationDialog({
  action,
  onClose,
  onSuccess,
}: {
  action: DangerAction;
  onClose: () => void;
  onSuccess: () => void;
}) {
  const config = CONFIRMATION_CONFIGS[action];
  const totalSteps = config.steps.length + 1; // +1 for the typed confirmation step
  const [step, setStep] = useState(0);
  const [typedValue, setTypedValue] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const typedId = useId();

  const isTypedStep = step === config.steps.length;
  const typedMatch = typedValue.toLowerCase().trim() === config.typedConfirmation;

  const handleConfirm = useCallback(async () => {
    if (isTypedStep) {
      if (!typedMatch || isSubmitting) return;
      setIsSubmitting(true);
      try {
        await config.onConfirm();
        onSuccess();
      } catch {
        alert(
          `Failed to ${action === "delete-data" ? "delete submitted data" : "delete account"}. Please try again.`,
        );
        setIsSubmitting(false);
      }
    } else {
      setStep((s) => s + 1);
    }
  }, [isTypedStep, typedMatch, isSubmitting, config, onSuccess, action]);

  return (
    <ConfirmationDialog
      open
      title={config.title}
      closeOnBackdrop={!isSubmitting}
      onClose={onClose}
      footer={
        <DialogActions>
          <DialogCancel type="button" onClick={onClose} disabled={isSubmitting}>
            Cancel
          </DialogCancel>
          <DialogConfirm
            color="red"
            variant="solid"
            onClick={handleConfirm}
            disabled={(isTypedStep && !typedMatch) || isSubmitting}
          >
            {isSubmitting
              ? "Deleting..."
              : isTypedStep
                ? "Delete permanently"
                : config.steps[step].confirmLabel}
          </DialogConfirm>
        </DialogActions>
      }
    >
      <StepDots aria-hidden="true">
        {Array.from({ length: totalSteps }, (_, i) => (
          <StepDot key={i} $active={i <= step} />
        ))}
      </StepDots>

      {isTypedStep ? (
        <div>
          <DialogBody>
            Type <DialogStrong>{config.typedConfirmation}</DialogStrong> to confirm:
          </DialogBody>
          <FieldLabel htmlFor={typedId} style={{ marginBottom: 6 }}>Confirmation</FieldLabel>
          <DitherInput
            id={typedId}
            value={typedValue}
            onChange={setTypedValue}
            placeholder={config.typedConfirmation}
            style={{ height: 38, borderRadius: 2 }}
          />
        </div>
      ) : (
        config.steps[step].body
      )}
    </ConfirmationDialog>
  );
}

// ============================================================================
// Data helpers
// ============================================================================

function apiTokenListItem(token: CreatedApiToken): ApiToken {
  return {
    id: token.id,
    name: token.name,
    createdAt: token.createdAt,
    lastUsedAt: token.lastUsedAt,
  };
}

function prependApiToken(tokens: ApiToken[], token: ApiToken): ApiToken[] {
  return [token, ...tokens.filter((item) => item.id !== token.id)];
}

function mergeApiTokenList(serverTokens: ApiToken[], currentTokens: ApiToken[]): ApiToken[] {
  const serverTokenIds = new Set(serverTokens.map((token) => token.id));
  const localTokens = currentTokens.filter((token) => !serverTokenIds.has(token.id));
  return [...localTokens, ...serverTokens];
}

async function fetchApiTokens(): Promise<ApiToken[]> {
  const tokensResponse = await fetch("/api/settings/tokens");
  const tokensData = await tokensResponse.json();
  return Array.isArray(tokensData.tokens) ? tokensData.tokens : [];
}

async function fetchDevices(username: string): Promise<SettingsDevice[]> {
  const devicesResponse = await fetch(`/api/users/${encodeURIComponent(username)}/devices`);
  if (!devicesResponse.ok) return [];
  const devicesData = await devicesResponse.json();
  return Array.isArray(devicesData.devices) ? devicesData.devices : [];
}

const timeFormatterCache = new Map<string, Intl.DateTimeFormat>();
function formatTimeIn(zone: string, date: Date): string {
  const key = zone;
  let formatter = timeFormatterCache.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-US", {
      dateStyle: "medium",
      timeStyle: "short",
      timeZone: zone === "auto" ? undefined : zone,
    });
    timeFormatterCache.set(key, formatter);
  }
  return formatter.format(date);
}

// ============================================================================
// Main component
// ============================================================================

export default function SettingsClient() {
  const router = useRouter();
  const { timezone, setTimezone, mounted } = useSettings();
  // Use the server-safe UTC label for the first render. The browser's zone can
  // differ from the server's, so reading it before hydration would make the
  // Auto option's text disagree with the SSR markup.
  const browserTimeZone = mounted ? getBrowserTimeZone() : "UTC";
  const [user, setUser] = useState<User | null>(null);
  const [tokens, setTokens] = useState<ApiToken[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [dangerAction, setDangerAction] = useState<DangerAction | null>(null);
  const [tokenName, setTokenName] = useState("CI token");
  const [createdToken, setCreatedToken] = useState<CreatedApiToken | null>(null);
  const [isCreatingToken, setIsCreatingToken] = useState(false);
  const [createTokenError, setCreateTokenError] = useState<string | null>(null);
  const [devices, setDevices] = useState<SettingsDevice[]>([]);
  const [editingDeviceId, setEditingDeviceId] = useState<string | null>(null);
  const [editingDeviceName, setEditingDeviceName] = useState("");
  const [isSavingDeviceName, setIsSavingDeviceName] = useState(false);
  const [deviceError, setDeviceError] = useState<string | null>(null);
  const [pendingTimezone, setPendingTimezone] = useState(timezone);
  const [timezoneSaved, setTimezoneSaved] = useState("");
  const tokenNameId = useId();
  const timezoneId = useId();
  const timezoneLabelId = useId();

  useEffect(() => {
    setPendingTimezone(timezone);
  }, [timezone]);

  useEffect(() => {
    let cancelled = false;

    async function loadSettings() {
      try {
        const sessionResponse = await fetch("/api/auth/session");
        const sessionData = await sessionResponse.json();
        if (cancelled) return;

        if (!sessionData.user) {
          router.push("/api/auth/github?returnTo=/settings");
          return;
        }

        const [loadedTokens, loadedDevices] = await Promise.all([
          fetchApiTokens().catch(() => []),
          fetchDevices(sessionData.user.username).catch(() => [] as SettingsDevice[]),
        ]);

        if (!cancelled) {
          setUser(sessionData.user);
          setTokens((current) => mergeApiTokenList(loadedTokens, current));
          setDevices(loadedDevices);
          setIsLoading(false);
        }
      } catch {
        if (!cancelled) {
          router.push("/leaderboard");
        }
      }
    }

    loadSettings();
    return () => {
      cancelled = true;
    };
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

  const handleDangerSuccess = useCallback(() => {
    if (dangerAction === "delete-account") {
      // Account is gone — redirect to home.
      window.location.href = "/";
    } else {
      // Data deleted — close the dialog and stay.
      setDangerAction(null);
      alert("Submitted data has been deleted.");
    }
  }, [dangerAction]);

  const handleCreateToken = async () => {
    setIsCreatingToken(true);
    setCreateTokenError(null);

    try {
      const response = await fetch("/api/settings/tokens", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: tokenName }),
      });

      const data = await response.json();
      if (!response.ok || !data.token) {
        throw new Error(data.error || "Failed to create token");
      }

      setCreatedToken(data.token);
      setTokens((current) => prependApiToken(current, apiTokenListItem(data.token)));
    } catch (error) {
      setCreateTokenError(error instanceof Error ? error.message : "Failed to create token");
    } finally {
      setIsCreatingToken(false);
    }
  };

  const startEditingDevice = (device: SettingsDevice) => {
    setEditingDeviceId(device.id);
    setDeviceError(null);
    // Pre-fill from the raw custom name, not the resolved display label, so
    // an unnamed device starts empty and a custom name that happens to equal
    // the fallback label ("Unnamed device" etc.) is preserved.
    setEditingDeviceName(device.customName ?? "");
  };

  const cancelEditingDevice = () => {
    setEditingDeviceId(null);
    setEditingDeviceName("");
    setDeviceError(null);
  };

  const handleSaveDeviceName = async (device: SettingsDevice) => {
    const trimmed = editingDeviceName.trim();
    const validationError = validateDeviceName(trimmed);
    if (validationError) {
      setDeviceError(validationError);
      return;
    }

    setIsSavingDeviceName(true);
    setDeviceError(null);

    try {
      const response = await fetch(`/api/settings/devices/${device.id}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        // Empty input clears the custom name; server stores null and the
        // display label falls back via deviceDisplayLabel.
        body: JSON.stringify({ name: trimmed === "" ? null : trimmed }),
      });

      const data = await response.json();
      if (!response.ok || !data.device) {
        throw new Error(data.error || "Failed to rename device");
      }

      setDevices((current) =>
        current.map((item) =>
          item.id === device.id
            ? {
                ...item,
                displayName: deviceDisplayLabel(data.device.deviceKey, data.device.displayName),
                customName: data.device.displayName ?? null,
              }
            : item,
        ),
      );
      setEditingDeviceId(null);
      setEditingDeviceName("");
    } catch (error) {
      setDeviceError(error instanceof Error ? error.message : "Failed to rename device");
    } finally {
      setIsSavingDeviceName(false);
    }
  };

  const handleCopyCreatedToken = async () => {
    if (!createdToken) return;
    await navigator.clipboard.writeText(createdToken.token);
    // The raw token is shown once and only once. After the user has copied
    // it we drop it from React state so it no longer lives in the component
    // tree (and thus no longer in any DevTools / extension snapshot of it).
    // Users who haven't copied yet still have the value in the reveal panel
    // until they navigate away.
    setCreatedToken(null);
  };

  const handleSaveTimezone = () => {
    setTimezone(pendingTimezone);
    setTimezoneSaved(`Saved. Profile timestamps now use ${pendingTimezone === "auto" ? "your browser's timezone" : pendingTimezone.replace(/_/g, " ")}.`);
  };

  const timezoneOptions = useMemo(
    () => [
      { value: "auto", label: `Auto (browser): ${browserTimeZone}` },
      ...COMMON_TIMEZONE_GROUPS.map((group) => ({
        label: group.region,
        options: group.zones.map((zone) => ({
          value: zone,
          label: zone.replace(/_/g, " "),
        })),
      })),
      ...(timezone !== "auto" && !COMMON_TIMEZONE_GROUPS.some((group) => group.zones.includes(timezone))
        ? [{ value: timezone, label: timezone.replace(/_/g, " ") }]
        : []),
    ],
    [browserTimeZone, timezone],
  );

  // What the setting produces: a real profile timestamp in the chosen zone.
  const timezonePreview = useMemo(() => {
    if (!mounted) return "";
    const sample = new Date(Date.UTC(2026, 6, 27, 16, 9));
    try {
      return formatTimeIn(pendingTimezone, sample);
    } catch {
      return formatTimeIn("auto", sample);
    }
  }, [mounted, pendingTimezone]);

  if (isLoading) {
    return (
      <PageWrapper>
        <Navigation />
        <LoadingMain>Loading…</LoadingMain>
        <ServiceFooter />
      </PageWrapper>
    );
  }

  if (!user) {
    return null;
  }

  const timezoneDirty = pendingTimezone !== timezone;

  return (
    <PageWrapper>
      <Navigation />

      <MainContent>
        <PageHead>
          <Title>Settings</Title>
          <PageLead>
            What Tokscale holds for @{user.username}, and the four things you control: how profile
            timestamps read, the API tokens that submit your usage, the names of your machines, and
            whether your data stays published.
          </PageLead>
        </PageHead>

        <SectionRow aria-labelledby="settings-profile">
          <LabelColumn>
            <SectionTitle id="settings-profile">Profile</SectionTitle>
            <SectionDescription>
              Name, photo and email come from GitHub. Tokscale never writes to your account there.
            </SectionDescription>
          </LabelColumn>
          <Panel>
            <ProfileRow>
              <ProfileAvatar src={user.avatarUrl} name={user.displayName || user.username} />
              <div>
                <ProfileName>{user.displayName || user.username}</ProfileName>
                <ProfileMeta>@{user.username}</ProfileMeta>
                {user.email && <ProfileMeta>{user.email}</ProfileMeta>}
              </div>
            </ProfileRow>
          </Panel>
        </SectionRow>

        <SectionRow aria-labelledby="settings-timezone">
          <LabelColumn>
            <SectionTitle id="settings-timezone">Display timezone</SectionTitle>
            <SectionDescription>
              Sets the zone for timestamps on profile pages, such as &ldquo;Updated&rdquo; and
              &ldquo;Joined&rdquo;, and is stored only in this browser. It does not move usage
              between days: which day a session counts toward is decided by the machine that
              scanned it, from that machine&rsquo;s own clock.
            </SectionDescription>
          </LabelColumn>
          <Panel>
            <div>
              <FieldLabel id={timezoneLabelId} htmlFor={timezoneId}>
                Timezone
              </FieldLabel>
              <div style={{ marginTop: 8 }}>
                <TimezoneSelect
                  id={timezoneId}
                  aria-labelledby={timezoneLabelId}
                  value={pendingTimezone}
                  options={timezoneOptions}
                  onChange={setPendingTimezone}
                />
              </div>
              <FieldHint>
                {timezonePreview
                  ? `A profile update on 27 Jul 2026, 16:09 UTC reads “${timezonePreview}”.`
                  : "Timestamps are formatted in the zone you pick."}
              </FieldHint>
            </div>
            <Actions>
              <SaveButton type="button" onClick={handleSaveTimezone} disabled={!timezoneDirty}>
                Save timezone
              </SaveButton>
            </Actions>
            <VisuallyHidden aria-live="polite">{timezoneSaved}</VisuallyHidden>
          </Panel>
        </SectionRow>

        <SectionRow aria-labelledby="settings-tokens">
          <LabelColumn>
            <SectionTitle id="settings-tokens">API tokens</SectionTitle>
            <SectionDescription>
              Tokens let CI or the CLI submit usage as you. Treat them like passwords: any token can
              submit data, and revoking one takes effect immediately.
            </SectionDescription>
          </LabelColumn>
          <Panel>
            <InlineForm>
              <div>
                <FieldLabel htmlFor={tokenNameId}>Token name</FieldLabel>
                <div style={{ marginTop: 8 }}>
                  <NameField
                    id={tokenNameId}
                    value={tokenName}
                    onChange={setTokenName}
                    maxLength={100}
                    style={{ maxWidth: 320 }}
                  />
                </div>
              </div>
              <DitherButton
                color="blue"
                variant="solid"
                onClick={handleCreateToken}
                disabled={isCreatingToken}
                style={{ minHeight: 40, padding: "0 16px", borderRadius: 2, fontSize: "0.8125rem" }}
              >
                {isCreatingToken ? "Creating…" : "Create token"}
              </DitherButton>
            </InlineForm>

            {createTokenError && <ErrorText role="alert">{createTokenError}</ErrorText>}

            {createdToken && (
              <Reveal>
                <RevealTitle>Copy this token now. It is not shown again.</RevealTitle>
                <RevealRow>
                  <TokenCode>{createdToken.token}</TokenCode>
                  <SaveButton type="button" onClick={handleCopyCreatedToken}>
                    Copy
                  </SaveButton>
                </RevealRow>
              </Reveal>
            )}

            {tokens.length === 0 ? (
              <Muted>
                No API tokens yet. Create one above, or run <code>tokscale login</code> in your
                terminal.
              </Muted>
            ) : (
              <List aria-label="API tokens">
                {tokens.map((token) => (
                  <ListRow key={token.id}>
                    <RowMain>
                      <RowName>{token.name}</RowName>
                      <RowMeta>
                        Created {new Date(token.createdAt).toLocaleDateString()}
                        {token.lastUsedAt && ` · Last used ${new Date(token.lastUsedAt).toLocaleDateString()}`}
                      </RowMeta>
                    </RowMain>
                    <TextButton type="button" onClick={() => handleRevokeToken(token.id)}>
                      Revoke
                    </TextButton>
                  </ListRow>
                ))}
              </List>
            )}
          </Panel>
        </SectionRow>

        <SectionRow aria-labelledby="settings-devices">
          <LabelColumn>
            <SectionTitle id="settings-devices">Devices</SectionTitle>
            <SectionDescription>
              Machines that have submitted usage. Rename one to tell them apart; the name is shown
              on your public profile.
            </SectionDescription>
          </LabelColumn>
          <Panel>
            {deviceError && <ErrorText role="alert">{deviceError}</ErrorText>}

            {devices.length === 0 ? (
              <Muted>
                No devices yet. Run <code>bunx tokscale submit</code> to register this machine.
              </Muted>
            ) : (
              <List aria-label="Devices">
                {devices.map((device) => (
                  <ListRow key={device.id} $editing={editingDeviceId === device.id}>
                    {editingDeviceId === device.id ? (
                      <DeviceEditRow>
                        <NameField
                          aria-label="Device name"
                          value={editingDeviceName}
                          maxLength={DEVICE_NAME_MAX_LENGTH}
                          placeholder="Device name (empty to reset)"
                          autoFocus
                          disabled={isSavingDeviceName}
                          onChange={setEditingDeviceName}
                          onKeyDown={(event) => {
                            if (event.key === "Enter") {
                              event.preventDefault();
                              handleSaveDeviceName(device);
                            } else if (event.key === "Escape") {
                              cancelEditingDevice();
                            }
                          }}
                        />
                        <SaveButton
                          type="button"
                          disabled={isSavingDeviceName}
                          onClick={() => handleSaveDeviceName(device)}
                        >
                          {isSavingDeviceName ? "Saving…" : "Save"}
                        </SaveButton>
                        <TextButton type="button" disabled={isSavingDeviceName} onClick={cancelEditingDevice}>
                          Cancel
                        </TextButton>
                      </DeviceEditRow>
                    ) : (
                      <>
                        <RowMain>
                          <RowName>{device.displayName}</RowName>
                          <RowMeta>
                            {formatNumber(device.totalTokens)} tokens · {formatCurrency(device.totalCost)} ·{" "}
                            {device.activeDays} active {device.activeDays === 1 ? "day" : "days"} · Last
                            submit {formatRelativeTime(device.lastSubmittedAt)}
                          </RowMeta>
                        </RowMain>
                        <TextButton type="button" onClick={() => startEditingDevice(device)}>
                          Rename
                        </TextButton>
                      </>
                    )}
                  </ListRow>
                ))}
              </List>
            )}
          </Panel>
        </SectionRow>

        <SectionRow aria-labelledby="settings-danger">
          <LabelColumn>
            <SectionTitle id="settings-danger">Danger zone</SectionTitle>
            <SectionDescription>
              Publishing is the point of Tokscale, so both options here remove data from it. Neither
              can be undone.
            </SectionDescription>
          </LabelColumn>
          <Panel>
            <DangerRow>
              <div>
                <DangerTitle>Delete submitted data</DangerTitle>
                <DangerDescription>
                  Removes leaderboard entries, profile stats and usage history. Your account and API
                  tokens stay active.
                </DangerDescription>
              </div>
              <DangerButton type="button" onClick={() => setDangerAction("delete-data")}>
                Delete data
              </DangerButton>
            </DangerRow>

            <DangerRow>
              <div>
                <DangerTitle>Delete account</DangerTitle>
                <DangerDescription>
                  Permanently deletes the account, its submissions, its tokens and its sessions.
                </DangerDescription>
              </div>
              <DangerButton type="button" onClick={() => setDangerAction("delete-account")}>
                Delete account
              </DangerButton>
            </DangerRow>
          </Panel>
        </SectionRow>
      </MainContent>

      <ServiceFooter />

      {dangerAction && (
        <DangerConfirmationDialog
          action={dangerAction}
          onClose={() => setDangerAction(null)}
          onSuccess={handleDangerSuccess}
        />
      )}
    </PageWrapper>
  );
}
