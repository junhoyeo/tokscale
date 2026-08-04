import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { Navigation } from "@/components/layout/Navigation";
import { ServiceFooter } from "@/components/layout/ServiceFooter";
import { getSession } from "@/lib/auth/session";
import { getGroupLeaderboardData } from "@/lib/groups/getGroupLeaderboard";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug, getGroupMemberCount } from "@/lib/groups/queries";
import { groupUrl } from "@/lib/seo/urls";
import GroupDetailClient from "./GroupDetailClient";

export const dynamic = "force-dynamic";

interface GroupPageProps {
  params: Promise<{ slug: string }>;
}

/**
 * getGroupBySlug is React-cached, so this shares the page's lookup rather than
 * issuing a second query.
 *
 * The name is only used when the group is public. A private group 404s for
 * non-members below, but generateMetadata still runs — putting its name in the
 * title would leak it to anyone who guessed the slug.
 */
export async function generateMetadata({ params }: GroupPageProps): Promise<Metadata> {
  const { slug } = await params;
  const group = await getGroupBySlug(slug);
  const canonical = groupUrl(slug);

  if (!group?.isPublic) {
    return {
      title: "Group | Tokscale",
      alternates: { canonical },
      robots: { index: false, follow: false },
    };
  }

  const title = `${group.name} - Group Token Usage | Tokscale`;
  const description =
    group.description?.trim() ||
    `Combined AI token usage and cost for the ${group.name} group on Tokscale.`;

  return {
    title,
    description,
    alternates: { canonical },
    openGraph: {
      title,
      description,
      type: "website",
      url: canonical,
      siteName: "Tokscale",
    },
    twitter: {
      card: "summary_large_image",
      title,
      description,
    },
  };
}

function PageShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">{children}</main>
      <ServiceFooter />
    </div>
  );
}

export default async function GroupPage({ params }: GroupPageProps) {
  const { slug } = await params;
  const group = await getGroupBySlug(slug);

  if (!group) {
    notFound();
  }

  const session = await getSession();
  const membership = session ? await getGroupMembership(group.id, session.id) : null;

  if (!group.isPublic && !membership) {
    notFound();
  }

  const [memberCount, initialData] = await Promise.all([
    getGroupMemberCount(group.id),
    getGroupLeaderboardData(group.id, "all", 1, 50, "tokens"),
  ]);

  return (
    <PageShell>
      <GroupDetailClient
        group={{
          id: group.id,
          name: group.name,
          slug: group.slug,
          description: group.description,
          avatarUrl: group.avatarUrl,
          isPublic: group.isPublic,
          memberCount,
          membership,
        }}
        currentUser={session}
        initialData={initialData}
      />
    </PageShell>
  );
}
