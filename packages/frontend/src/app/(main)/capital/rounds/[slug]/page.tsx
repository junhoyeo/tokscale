import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { absoluteUrl } from "@/lib/capital";
import { ledger } from "@/lib/capital/ledger";
import { CapitalShell } from "../../_components/CapitalShell";
import { EventDetail, eventJsonLd, eventSeoTitle } from "../../_components/EventDetail";
import { JsonLd } from "../../_components/JsonLd";

interface PageProps {
  params: Promise<{ slug: string }>;
}

export const dynamicParams = false;

export function generateStaticParams() {
  return ledger.getRounds().map((e) => ({ slug: e.slug }));
}

function load(slug: string) {
  const e = ledger.getEvent(slug);
  return e && e.route === "rounds" ? e : null;
}

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const { slug } = await params;
  const e = load(slug);
  if (!e) return { title: "Not found | Tokscale", robots: { index: false } };
  const title = `${eventSeoTitle(e)} | Tokscale`;
  const description = e.event.summary;
  const canonical = absoluteUrl(e.href);
  const image = { url: absoluteUrl(e.ogImage), width: 1200, height: 630, alt: e.title };
  return {
    title,
    description,
    alternates: { canonical },
    // Events without a primary source stay out of search results.
    robots: e.indexable ? undefined : { index: false, follow: true },
    openGraph: { title, description, type: "article", url: canonical, siteName: "Tokscale", images: [image], publishedTime: e.date, modifiedTime: e.event.reviewedAt },
    twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title, description, images: [image.url] },
  };
}

export default async function RoundPage({ params }: PageProps) {
  const { slug } = await params;
  const e = load(slug);
  if (!e) notFound();
  return (
    <CapitalShell section={e.isCompute ? "compute" : "rounds"}>
      <JsonLd data={eventJsonLd(e, absoluteUrl(e.href), absoluteUrl)} />
      <EventDetail e={e} />
    </CapitalShell>
  );
}
