import type { Metadata } from 'next';
import LocalClient from './LocalClient';

export const metadata: Metadata = {
  title: 'Local Data Viewer | Tokscale',
  description: 'Inspect a Tokscale export from your own machine, in your browser. Nothing is uploaded.',
  // Renders entirely from data the visitor loads client-side, so a crawler
  // only ever sees an empty shell. Kept out of the index rather than left as a
  // thin-content page; still crawlable so its outbound links are followed.
  robots: { index: false, follow: true },
};

export default function LocalViewerPage() {
  return <LocalClient />;
}
