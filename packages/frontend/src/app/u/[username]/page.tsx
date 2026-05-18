import type { Metadata } from 'next';
import { notFound, permanentRedirect } from 'next/navigation';
import ProfilePageClient from './ProfilePageClient';

export const revalidate = 60;

async function getProfileData(username: string) {
  // In production: use explicit URL or Vercel auto-URL.
  // In dev: use 127.0.0.1 to avoid ECONNREFUSED from localhost dual-stack DNS.
  const baseUrl = process.env.NEXT_PUBLIC_URL
    || (process.env.VERCEL_URL ? `https://${process.env.VERCEL_URL}` : null)
    || 'http://127.0.0.1:3000';
  
  const res = await fetch(`${baseUrl}/api/users/${username}`, {
    next: { revalidate: 60 },
  });
  
  if (!res.ok) {
    return null;
  }
  
  return res.json();
}

async function getSourceSummaries(username: string) {
  try {
    const baseUrl = process.env.NEXT_PUBLIC_URL
      || (process.env.VERCEL_URL ? `https://${process.env.VERCEL_URL}` : null)
      || 'http://127.0.0.1:3000';

    const res = await fetch(`${baseUrl}/api/users/${username}/sources`, {
      next: { revalidate: 60 },
    });

    if (!res.ok) {
      return { sources: [] };
    }

    return res.json();
  } catch {
    return { sources: [] };
  }
}

async function getSourceDetail(username: string, sourceKey: string) {
  try {
    const baseUrl = process.env.NEXT_PUBLIC_URL
      || (process.env.VERCEL_URL ? `https://${process.env.VERCEL_URL}` : null)
      || 'http://127.0.0.1:3000';

    const res = await fetch(`${baseUrl}/api/users/${username}/sources/${encodeURIComponent(sourceKey)}`, {
      next: { revalidate: 60 },
    });

    if (!res.ok) {
      return { source: null };
    }

    return res.json();
  } catch {
    return { source: null };
  }
}

async function getSourceSummary(username: string, sourceKey: string) {
  try {
    const baseUrl = process.env.NEXT_PUBLIC_URL
      || (process.env.VERCEL_URL ? `https://${process.env.VERCEL_URL}` : null)
      || 'http://127.0.0.1:3000';

    const res = await fetch(
      `${baseUrl}/api/users/${username}/sources/${encodeURIComponent(sourceKey)}/summary`,
      {
        next: { revalidate: 60 },
      }
    );

    if (!res.ok) {
      return { source: null };
    }

    return res.json();
  } catch {
    return { source: null };
  }
}

export async function generateMetadata({ params }: { params: Promise<{ username: string }> }): Promise<Metadata> {
  const { username } = await params;
  return {
    title: `@${username} - Token Usage | Tokscale`,
    description: `View ${username}'s AI token usage statistics and cost breakdown on Tokscale`,
    openGraph: {
      title: `@${username}'s Token Usage | Tokscale`,
      description: `AI token usage statistics for ${username} on Tokscale`,
      type: 'profile',
      url: `https://tokscale.ai/u/${username}`,
      siteName: 'Tokscale',
      images: [
        {
          url: 'https://tokscale.ai/og-image.png',
          width: 1200,
          height: 630,
          alt: `${username}'s Token Usage on Tokscale`,
        },
      ],
    },
    twitter: {
      card: 'summary_large_image',
      title: `@${username}'s Token Usage | Tokscale`,
      images: ['https://tokscale.ai/og-image.png'],
    },
  };
}

export default async function ProfilePage({ params }: { params: Promise<{ username: string }> }) {
  const { username } = await params;
  const [data, sourceData] = await Promise.all([
    getProfileData(username),
    getSourceSummaries(username),
  ]);
  
  if (!data) {
    notFound();
  }

  if (data.user?.username && data.user.username !== username) {
    permanentRedirect(`/u/${data.user.username}`);
  }

  const initialSourceKey = sourceData?.sources?.[0]?.sourceKey;
  const [sourceDetailData, sourceSummaryData] = initialSourceKey
    ? await Promise.all([
        getSourceDetail(username, initialSourceKey),
        getSourceSummary(username, initialSourceKey),
      ])
    : [{ source: null }, { source: null }];
  
  return (
    <ProfilePageClient
      initialData={data}
      initialSources={sourceData?.sources ?? []}
      initialSelectedSource={sourceDetailData?.source ?? null}
      initialSelectedSourceSummary={sourceSummaryData?.source ?? null}
      username={username}
    />
  );
}
