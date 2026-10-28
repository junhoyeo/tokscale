"use client";

import styled from "styled-components";
import { formatRelativeTime } from "@/lib/format";
import { formatCurrency, formatNumber } from "@/lib/utils";
import { DitherMiniBar } from "./ditherControls";
import {
  ListBody,
  ListCaption,
  ListCard,
  ListCell,
  ListHead,
  ListHeaderCell,
  ListPrimaryCell,
  ListRow,
  ListTable,
  NumericValue,
} from "./listStyles";

/** Public device usage shape returned by the profile devices route. */
export interface ProfileDevice {
  id: string;
  deviceKey: string;
  displayName: string;
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
  className?: string;
}

const DevicesSection = styled.section`
  display: grid;
  gap: 0.5rem;
`;

const SectionHeading = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  letter-spacing: -0.01em;
  line-height: 1.25;
`;

const SectionNote = styled.p`
  margin: 0 0 0.25rem;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.4;
`;

const DeviceName = styled.span`
  display: block;
  overflow: hidden;
  color: var(--service-text);
  text-overflow: ellipsis;
  white-space: nowrap;

  @media (max-width: 639px) {
    overflow-wrap: anywhere;
    white-space: normal;
  }
`;

const DeviceShare = styled.span`
  display: block;
  margin-top: 0.375rem;
`;

const LastSubmitted = styled.span`
  display: block;
  margin-top: 0.2rem;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 400;
  line-height: 1.25;
`;

/** Compact per-device usage for public profiles. */
export function ProfileDevices({ devices, className }: ProfileDevicesProps) {
  if (devices.length === 0) return null;
  const maxTokens = devices.reduce(
    (max, device) => Math.max(max, device.totalTokens),
    0,
  );

  return (
    <DevicesSection
      className={className}
      aria-labelledby="profile-devices-heading"
    >
      <SectionHeading id="profile-devices-heading">
        Devices
      </SectionHeading>
      <SectionNote>All-time usage from each machine that submitted.</SectionNote>

      <ListCard>
        <ListTable>
          <ListCaption>Usage by device</ListCaption>
          <ListHead>
            <tr>
              <ListHeaderCell $width="52%">Device</ListHeaderCell>
              <ListHeaderCell $width="19%" $align="right">
                Tokens
              </ListHeaderCell>
              <ListHeaderCell $width="16%" $align="right">
                Cost
              </ListHeaderCell>
              <ListHeaderCell $width="13%" $align="right">
                Active days
              </ListHeaderCell>
            </tr>
          </ListHead>
          <ListBody>
            {devices.map((device) => (
              <ListRow key={device.id}>
                <ListPrimaryCell scope="row">
                  <DeviceName>{device.displayName}</DeviceName>
                  {devices.length > 1 ? (
                    <DeviceShare>
                      <DitherMiniBar
                        ratio={
                          maxTokens > 0 ? device.totalTokens / maxTokens : 0
                        }
                        color={device === devices[0] ? "#f4f7fb" : "#8a8c92"}
                      />
                    </DeviceShare>
                  ) : null}
                  <LastSubmitted>
                    Last submitted{" "}
                    <time
                      dateTime={device.lastSubmittedAt ?? undefined}
                      suppressHydrationWarning
                    >
                      {formatRelativeTime(device.lastSubmittedAt)}
                    </time>
                  </LastSubmitted>
                </ListPrimaryCell>
                <ListCell data-label="Tokens" $align="right">
                  <NumericValue
                    title={device.totalTokens.toLocaleString("en-US")}
                  >
                    {formatNumber(device.totalTokens)}
                  </NumericValue>
                </ListCell>
                <ListCell data-label="Cost" $align="right">
                  <NumericValue $accent>
                    {formatCurrency(device.totalCost)}
                  </NumericValue>
                </ListCell>
                <ListCell data-label="Active days" $align="right">
                  <NumericValue>
                    {device.activeDays.toLocaleString("en-US")}
                  </NumericValue>
                </ListCell>
              </ListRow>
            ))}
          </ListBody>
        </ListTable>
      </ListCard>
    </DevicesSection>
  );
}
