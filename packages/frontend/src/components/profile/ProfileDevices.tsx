"use client";

import type { ReactNode } from "react";
import styled from "styled-components";
import { formatNumber, formatCurrency } from "@/lib/utils";
import { formatRelativeTime } from "@/lib/format";
import { ListCard, ListHeader, ListMetricCell, ListRow } from "./listStyles";

/**
 * Shape returned by GET /api/users/[username]/devices (route already coerces
 * the SQL SUM strings to numbers and applies the display-name fallback).
 */
export interface ProfileDevice {
  id: string;
  deviceKey: string;
  /** Resolved label (custom name or fallback) — what public UIs render. */
  displayName: string;
  /** Raw user-set name, null when the device has never been renamed. */
  customName: string | null;
  createdAt: string | null;
  lastSubmittedAt: string | null;
  totalTokens: number;
  totalCost: number;
  inputTokens: number;
  outputTokens: number;
  activeDays: number;
  firstDay: string | null;
  lastDay: string | null;
}

export interface ProfileDevicesProps {
  devices: ProfileDevice[];
}

const SectionHeading = styled.h2`
  font-size: 1.125rem;
  font-weight: 600;
  margin-bottom: 0.75rem;
`;

// Card/header/row/metric-cell primitives are shared with ProfileModels via
// ./listStyles so the two profile tables stay visually in sync.
const DevicesContainer = ListCard;
const DevicesHeader = ListHeader;
const DeviceRow = ListRow;

const DeviceNameCell = styled.div`
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
`;

const DeviceNameText = styled.span`
  font-size: 0.8125rem;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (min-width: 480px) {
    font-size: 0.875rem;
  }
`;

const DeviceSubText = styled.span`
  font-size: 0.75rem;
`;

// All device metric columns share the same fixed width, unlike the per-column
// widths in ProfileModels, so bake them in here.
function DeviceMetricCell(props: {
  $hideOnMobile?: boolean;
  children?: ReactNode;
}) {
  return <ListMetricCell $width="4.5rem" $smWidth="5.5rem" {...props} />;
}

const MetricText = styled.span`
  font-size: 0.8125rem;

  @media (min-width: 480px) {
    font-size: 0.875rem;
  }
`;

const CostText = styled.span`
  font-size: 0.8125rem;
  font-weight: 500;

  @media (min-width: 480px) {
    font-size: 0.875rem;
  }
`;

/**
 * Per-device usage breakdown for the public profile page. Hidden entirely
 * when the user has no recorded devices (pre-device legacy profiles).
 */
export function ProfileDevices({ devices }: ProfileDevicesProps) {
  if (devices.length === 0) return null;

  return (
    <section aria-label="Devices">
      <SectionHeading style={{ color: "var(--color-fg-default)" }}>
        Devices
      </SectionHeading>

      <DevicesContainer
        style={{
          backgroundColor: "var(--color-bg-default)",
          borderColor: "var(--color-border-default)",
        }}
      >
        <DevicesHeader
          style={{
            backgroundColor: "var(--color-bg-elevated)",
            borderColor: "var(--color-border-default)",
            color: "var(--color-fg-muted)",
          }}
        >
          <div>Device</div>
          <DeviceMetricCell>Tokens</DeviceMetricCell>
          <DeviceMetricCell>Cost</DeviceMetricCell>
          <DeviceMetricCell $hideOnMobile>Active Days</DeviceMetricCell>
        </DevicesHeader>

        <div>
          {devices.map((device, index) => (
            <DeviceRow
              key={device.id}
              style={{
                backgroundColor:
                  index % 2 === 1 ? "var(--color-bg-elevated)" : "transparent",
                borderTop:
                  index > 0 ? "1px solid var(--color-border-default)" : undefined,
              }}
            >
              <DeviceNameCell>
                <DeviceNameText style={{ color: "var(--color-fg-default)" }}>
                  {device.displayName}
                </DeviceNameText>
                <DeviceSubText
                  style={{ color: "var(--color-fg-muted)" }}
                  suppressHydrationWarning
                >
                  Last submit {formatRelativeTime(device.lastSubmittedAt)}
                </DeviceSubText>
              </DeviceNameCell>
              <DeviceMetricCell>
                <MetricText
                  style={{ color: "var(--color-fg-default)" }}
                  title={device.totalTokens.toLocaleString("en-US")}
                >
                  {formatNumber(device.totalTokens)}
                </MetricText>
              </DeviceMetricCell>
              <DeviceMetricCell>
                <CostText style={{ color: "var(--color-primary)" }}>
                  {formatCurrency(device.totalCost)}
                </CostText>
              </DeviceMetricCell>
              <DeviceMetricCell $hideOnMobile>
                <MetricText style={{ color: "var(--color-fg-muted)" }}>
                  {device.activeDays}
                </MetricText>
              </DeviceMetricCell>
            </DeviceRow>
          ))}
        </div>
      </DevicesContainer>
    </section>
  );
}
