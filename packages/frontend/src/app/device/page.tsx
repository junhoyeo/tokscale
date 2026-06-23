import type { Metadata } from 'next';
import DeviceClient from './DeviceClient';

export const metadata: Metadata = {
  title: 'Device Authorization - Token Usage',
  description: 'Authorize your device to sync token usage data',
};

export default function DevicePage() {
  return <DeviceClient />;
}
