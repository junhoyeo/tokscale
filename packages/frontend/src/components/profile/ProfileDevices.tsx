"use client";

import styled from "styled-components";
import { formatNumber, formatCurrency } from "@/lib/utils";

export interface ProfileDevice {
  id: string;
  name: string;
  os: string | null;
  totalTokens: number;
  totalCost: number;
  activeDays: number;
  lastActiveDate: string | null;
}

interface ProfileDevicesProps {
  devices: ProfileDevice[];
}

// Distinct hues for the per-device share bar; cycles when there are more
// devices than colors.
const DEVICE_COLORS = [
  "#006edb",
  "#894ceb",
  "#30a147",
  "#eb670f",
  "#d6336c",
  "#0ca678",
  "#f59f00",
  "#4263eb",
];

const OS_LABELS: Record<string, string> = {
  linux: "Linux",
  macos: "macOS",
  windows: "Windows",
};

/** Per-device usage breakdown shown on the profile breakdown tab. */
export function ProfileDevices({ devices }: ProfileDevicesProps) {
  if (devices.length === 0) {
    return null;
  }

  const totalTokens = devices.reduce((sum, d) => sum + d.totalTokens, 0);

  const ranked = [...devices]
    .sort((a, b) => b.totalTokens - a.totalTokens)
    .map((device, index) => ({
      ...device,
      color: DEVICE_COLORS[index % DEVICE_COLORS.length],
      percentage: totalTokens > 0 ? (device.totalTokens / totalTokens) * 100 : 0,
    }));

  return (
    <Container
      style={{
        backgroundColor: "var(--color-bg-default)",
        borderColor: "var(--color-border-default)",
      }}
    >
      <Header>
        <Title style={{ color: "var(--color-fg-default)" }}>
          {devices.length === 1 ? "1 device" : `${devices.length} devices`}
        </Title>
        <Subtitle style={{ color: "var(--color-fg-subtle)" }}>
          Usage aggregated across every machine
        </Subtitle>
      </Header>

      {totalTokens > 0 && (
        <ProgressBar style={{ backgroundColor: "var(--color-bg-subtle)" }}>
          {ranked.map((device) => (
            <div
              key={device.id}
              style={{
                width: `${device.percentage}%`,
                backgroundColor: device.color,
              }}
              title={`${device.name}: ${formatNumber(device.totalTokens)}`}
            />
          ))}
        </ProgressBar>
      )}

      <DeviceList>
        {ranked.map((device) => (
          <DeviceRow key={device.id}>
            <DeviceLeft>
              <Dot style={{ backgroundColor: device.color }} />
              <DeviceMeta>
                <DeviceName style={{ color: "var(--color-fg-default)" }}>
                  {device.name}
                </DeviceName>
                <DeviceSub style={{ color: "var(--color-fg-muted)" }}>
                  {device.os ? OS_LABELS[device.os] ?? device.os : "Unknown OS"}
                  {device.activeDays > 0 && ` · ${device.activeDays} active days`}
                </DeviceSub>
              </DeviceMeta>
            </DeviceLeft>
            <DeviceRight>
              <DeviceTokens style={{ color: "var(--color-fg-default)" }}>
                {formatNumber(device.totalTokens)}
              </DeviceTokens>
              <DeviceSub style={{ color: "var(--color-fg-subtle)" }}>
                {formatCurrency(device.totalCost)} · {device.percentage.toFixed(1)}%
              </DeviceSub>
            </DeviceRight>
          </DeviceRow>
        ))}
      </DeviceList>
    </Container>
  );
}

const Container = styled.div`
  border-radius: 1rem;
  border-width: 1px;
  border-style: solid;
  padding: 1rem;

  @media (min-width: 640px) {
    padding: 1.5rem;
  }
`;

const Header = styled.div`
  margin-bottom: 1rem;
`;

const Title = styled.h3`
  font-size: 1rem;
  font-weight: 600;

  @media (min-width: 640px) {
    font-size: 1.125rem;
  }
`;

const Subtitle = styled.p`
  font-size: 0.75rem;
  margin-top: 0.125rem;
`;

const ProgressBar = styled.div`
  height: 0.75rem;
  border-radius: 9999px;
  overflow: hidden;
  display: flex;
  margin-bottom: 1.25rem;
`;

const DeviceList = styled.div`
  display: flex;
  flex-direction: column;
  gap: 0.875rem;
`;

const DeviceRow = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
`;

const DeviceLeft = styled.div`
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-width: 0;
`;

const Dot = styled.div`
  width: 0.75rem;
  height: 0.75rem;
  border-radius: 9999px;
  flex-shrink: 0;
`;

const DeviceMeta = styled.div`
  min-width: 0;
`;

const DeviceName = styled.p`
  font-size: 0.875rem;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const DeviceSub = styled.p`
  font-size: 0.75rem;
`;

const DeviceRight = styled.div`
  text-align: right;
  flex-shrink: 0;
`;

const DeviceTokens = styled.p`
  font-size: 0.875rem;
  font-weight: 600;

  @media (min-width: 640px) {
    font-size: 1rem;
  }
`;
