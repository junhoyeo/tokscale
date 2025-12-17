import type { Metadata } from 'next';
import SettingsClient from './SettingsClient';

export const metadata: Metadata = {
  title: 'Settings - Token Usage',
  description: 'Manage your account settings and API tokens',
};

export default function SettingsPage() {
  return <SettingsClient />;
}
