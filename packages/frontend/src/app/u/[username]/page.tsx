import type { Metadata } from 'next';
import { notFound, permanentRedirect } from 'next/navigation';
import type { ProfileDevice } from '@/components/profile';
import { loadPublicProfileDevicesForPage } from '@/lib/publicProfileDevices';
import { loadPublicProfileForPage } from '@/lib/publicProfileData';
import { profileUrl } from '@/lib/seo/urls';
import { getSession } from '@/lib/auth/session';
import { getModerationNotice } from '@/lib/moderation/notice';
import ProfilePageClient, { type ProfileData } from './ProfilePageClient';

export const dynamic = "force-dynamic";

const PROFILE_PERIODS = ["all", "week", "month"] as const;
type ProfilePeriod = (typeof PROFILE_PERIODS)[number];

function parseProfilePeriod(value: string | string[] | undefined): ProfilePeriod {
  const period = Array.isArray(value) ? value[0] : value;
  return PROFILE_PERIODS.includes(period as ProfilePeriod)
    ? (period as ProfilePeriod)
    : "all";
}

async function getProfileData(
  username: string,
  period: ProfilePeriod,
): Promise<ProfileData | null> {
  // Calling the shared server handler keeps Vercel Deployment Protection out
  // of the render path. A server-side HTTP self-fetch is anonymous and is
  // redirected to Vercel's HTML login page on protected preview deployments.
  const result = await loadPublicProfileForPage(username, period);

  if (result.kind === "redirect") {
    if (result.location) {
      const canonicalUsername = decodeURIComponent(
        new URL(result.location).pathname.split("/").at(-1) ?? "",
      );
      if (canonicalUsername && canonicalUsername !== username) {
        return getProfileData(canonicalUsername, period);
      }
    }
  }

  if (result.kind !== "data") {
    return null;
  }

  return result.data as ProfileData;
}

// Devices are an enrichment on top of the core profile: if this fetch fails
// we still render the profile, just without the Devices section.
async function getProfileDevices(username: string) {
  try {
    return (await loadPublicProfileDevicesForPage(username)) as ProfileDevice[];
  } catch {
    return [];
  }
}

export async function generateMetadata({ params }: { params: Promise<{ username: string }> }): Promise<Metadata> {
  const { username } = await params;
  return {
    title: `@${username} - Token Usage | Tokscale`,
    description: `View ${username}'s AI token usage statistics and cost breakdown on Tokscale`,
    // ?period=all|week|month all render this same profile over a different
    // window, so they consolidate onto the bare URL. Safe to build from the
    // request param: a non-canonical casing permanent-redirects below rather
    // than rendering, so this only ever runs for the canonical spelling.
    alternates: {
      canonical: profileUrl(username),
    },
    // No `images` on either block: opengraph-image.tsx in this directory
    // supplies a card rendered from this user's real numbers. Setting an
    // explicit images array here would override it with the generic
    // site-wide PNG, which is what every profile used to share.
    openGraph: {
      title: `@${username}'s Token Usage | Tokscale`,
      description: `AI token usage statistics for ${username} on Tokscale`,
      type: 'profile',
      url: profileUrl(username),
      siteName: 'Tokscale',
    },
    twitter: {
      card: 'summary_large_image',
      title: `@${username}'s Token Usage | Tokscale`,
    },
  };
}

export default async function ProfilePage({
  params,
  searchParams,
}: {
  params: Promise<{ username: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { username } = await params;
  const resolvedSearchParams = await searchParams;
  const period = parseProfilePeriod(resolvedSearchParams.period);
  const [data, devices] = await Promise.all([
    getProfileData(username, period),
    getProfileDevices(username),
  ]);

  if (!data) {
    notFound();
  }

  if (data.user?.username && data.user.username !== username) {
    permanentRedirect(`/u/${data.user.username}${period === "all" ? "" : `?period=${period}`}`);
  }

  // Fetched outside the cached public payload and only for the account owner:
  // publicProfileData is unstable_cache'd and served to everyone, so moderation
  // state must never travel with it. A visitor sees nothing at all.
  const session = await getSession().catch(() => null);
  const moderationNotice =
    session && data.user?.id && session.id === data.user.id
      ? await getModerationNotice(session.id)
      : null;

  return (
    <ProfilePageClient
      initialData={data}
      initialDevices={devices}
      username={username}
      moderationNotice={moderationNotice}
    />
  );
}
