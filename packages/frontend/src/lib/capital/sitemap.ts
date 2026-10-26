import type { MetadataRoute } from "next";
import { ledger } from "./ledger";
import {
  absoluteUrl,
  capitalUrl,
  companiesUrl,
  companyUrl,
  computeUrl,
  investorUrl,
  investorsUrl,
  methodologyUrl,
  roundsUrl,
  sectorUrl,
  spendingUrl,
} from "./urls";

/**
 * Sitemap entries for the capital section. app/sitemap.ts spreads these in.
 *
 * lastModified is a real content date, never the request time: event and profile pages use
 * their latest review or correction date; the spending page the date its XBRL facts were
 * retrieved. Section pages take the newest of their parts.
 *
 * Pages that are not indexable (no primary source, no sourced fact, filtered list views) are
 * left out, matching their robots metadata.
 */
const day = (date: string) => new Date(`${date.slice(0, 10)}T00:00:00.000Z`);
const newest = (dates: Date[]) => new Date(Math.max(...dates.map((d) => d.getTime())));

export function capitalSitemapEntries(): MetadataRoute.Sitemap {
  const entries: MetadataRoute.Sitemap = [];
  const seen = new Set<string>();
  const push = (entry: MetadataRoute.Sitemap[number]) => {
    if (seen.has(entry.url)) return;
    seen.add(entry.url);
    entries.push(entry);
  };

  const events = ledger.getEvents().filter((e) => e.indexable);
  const eventModified = (e: (typeof events)[number]) =>
    day([e.event.reviewedAt, ...e.event.corrections.map((c) => c.date)].sort().at(-1)!);
  const ledgerModified = day(ledger.reviewedAt());
  const capexModified = day(ledger.capexRetrievedAt);

  push({ url: capitalUrl(), lastModified: newest([ledgerModified, capexModified]), changeFrequency: "weekly", priority: 0.8 });
  push({ url: companiesUrl(), lastModified: ledgerModified, changeFrequency: "weekly", priority: 0.7 });
  push({ url: investorsUrl(), lastModified: ledgerModified, changeFrequency: "weekly", priority: 0.6 });
  push({ url: roundsUrl(), lastModified: ledgerModified, changeFrequency: "weekly", priority: 0.7 });
  push({ url: computeUrl(), lastModified: ledgerModified, changeFrequency: "weekly", priority: 0.7 });
  push({ url: spendingUrl(), lastModified: capexModified, changeFrequency: "monthly", priority: 0.7 });
  push({ url: methodologyUrl(), lastModified: ledgerModified, changeFrequency: "monthly", priority: 0.4 });

  for (const sector of ledger.getSectors()) {
    const dates = [ledgerModified, ...sector.events.filter((e) => e.indexable).map(eventModified)];
    push({ url: sectorUrl(sector.slug), lastModified: newest(dates), changeFrequency: "weekly", priority: 0.6 });
  }

  for (const event of events) {
    push({ url: absoluteUrl(event.href), lastModified: eventModified(event), changeFrequency: "monthly", priority: 0.6 });
  }

  for (const org of ledger.getCompanies()) {
    const profile = ledger.getCompany(org.slug);
    if (!profile || !profile.indexable) continue;
    push({ url: companyUrl(org.slug), lastModified: day(profile.reviewedAt), changeFrequency: "weekly", priority: 0.6 });
  }

  for (const row of ledger.getInvestors()) {
    const profile = ledger.getInvestor(row.org.slug);
    if (!profile || !profile.indexable) continue;
    push({ url: investorUrl(row.org.slug), lastModified: day(profile.reviewedAt), changeFrequency: "monthly", priority: 0.5 });
  }

  return entries;
}
