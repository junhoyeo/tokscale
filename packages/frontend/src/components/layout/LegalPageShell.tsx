import { Children, isValidElement, type ReactNode } from "react";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";

/**
 * Shell for legal/informational pages.
 *
 * A server component keeps the prose in the initial HTML for crawlers and
 * ships no client JS for the prose itself. `lede` is the plain sentence under
 * the title that says what the page is. A page that gives its h2s an id gets
 * an "On this page" list of them: beside the text on wide screens (sticky),
 * above it on narrow ones.
 */
export function LegalPageShell({
  title,
  lede,
  lastUpdated,
  children,
}: {
  title: string;
  lede?: string;
  lastUpdated: string;
  children: ReactNode;
}) {
  const contents = Children.toArray(children).flatMap((child) =>
    isValidElement<{ id?: string; children?: ReactNode }>(child) &&
    child.type === "h2" &&
    child.props.id &&
    typeof child.props.children === "string"
      ? [{ id: child.props.id, label: child.props.children }]
      : [],
  );

  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <article className={contents.length ? "legal-prose legal-prose--contents" : "legal-prose"}>
          <h1>{title}</h1>
          {lede ? <p className="legal-lede">{lede}</p> : null}
          <p className="legal-updated">Last updated: {lastUpdated}</p>
          {contents.length ? (
            <div className="legal-layout">
              <nav className="legal-contents" aria-labelledby="legal-contents-title">
                <p className="legal-contents-title" id="legal-contents-title">
                  On this page
                </p>
                <ul>
                  {contents.map((entry) => (
                    <li key={entry.id}>
                      <a href={`#${entry.id}`}>{entry.label}</a>
                    </li>
                  ))}
                </ul>
              </nav>
              <div className="legal-text">{children}</div>
            </div>
          ) : (
            children
          )}
        </article>
      </main>
      <PublicFooter />
    </div>
  );
}
