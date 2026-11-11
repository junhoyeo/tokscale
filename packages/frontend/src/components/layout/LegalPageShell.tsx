import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";

/**
 * Shell for legal/informational pages.
 *
 * A server component keeps the prose in the initial HTML for crawlers and
 * ships no client JS for the prose itself.
 */
export function LegalPageShell({
  title,
  lastUpdated,
  children,
}: {
  title: string;
  lastUpdated: string;
  children: React.ReactNode;
}) {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <article className="legal-prose">
          <h1>{title}</h1>
          <p className="legal-updated">Last updated: {lastUpdated}</p>
          {children}
        </article>
      </main>
      <PublicFooter />
    </div>
  );
}
