"use client";

import styled from "styled-components";
import { formatNumber, formatCurrency } from "@/lib/utils";
import { formatRelativeTime } from "@/lib/format";

/**
 * Shape returned by GET /api/users/[username]/devices (route already coerces
 * the SQL SUM strings to numbers and applies the display-name fallback).
 */
export interface ProfileDevice {
  id: string;
  deviceKey: string;
  displayName: string;
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

const DevicesContainer = styled.div`
  border-radius: 1rem;
  border-width: 1px;
  border-style: solid;
  overflow: hidden;
`;

const DevicesHeader = styled.div`
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: 0.75rem;
  padding: 0.75rem;
  font-size: 0.75rem;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom-width: 1px;
  border-bottom-style: solid;

  @media (min-width: 480px) {
    grid-template-columns: 1fr auto auto auto;
    gap: 1rem;
    padding-left: 1rem;
    padding-right: 1rem;
  }

  @media (min-width: 640px) {
    padding-left: 1.5rem;
    padding-right: 1.5rem;
  }
`;

const DeviceRow = styled.div`
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: 0.75rem;
  padding: 0.75rem;
  align-items: center;

  @media (min-width: 480px) {
    grid-template-columns: 1fr auto auto auto;
    gap: 1rem;
    padding-left: 1rem;
    padding-right: 1rem;
  }

  @media (min-width: 640px) {
    padding-left: 1.5rem;
    padding-right: 1.5rem;
  }
`;

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

const DeviceMetricCell = styled.div<{ $hideOnMobile?: boolean }>`
  text-align: right;
  width: 4.5rem;

  ${(props) =>
    props.$hideOnMobile &&
    `
    @media (max-width: 479px) {
      display: none;
    }
  `}

  @media (min-width: 640px) {
    width: 5.5rem;
  }
`;

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
