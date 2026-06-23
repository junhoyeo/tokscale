import type { Metadata } from 'next';
import LocalClient from './LocalClient';

export const metadata: Metadata = {
  title: 'Local Data - Token Usage',
  description: 'View your local AI token usage data',
};

export default function LocalViewerPage() {
  return <LocalClient />;
}
