import type { Metadata } from "next";
import Link from "next/link";
import { LegalPageShell } from "@/components/layout/LegalPageShell";
import { legalUrl } from "@/lib/seo/urls";

export const dynamic = "force-dynamic";

const DESCRIPTION =
  "How to reach the maintainer of Tokscale — bug reports, client support requests, incorrect leaderboard data, and privacy questions.";

export const metadata: Metadata = {
  title: "Contact | Tokscale",
  description: DESCRIPTION,
  alternates: { canonical: legalUrl("contact") },
  openGraph: {
    title: "Contact | Tokscale",
    description: DESCRIPTION,
    type: "article",
    url: legalUrl("contact"),
    siteName: "Tokscale",
  },
};

export default function ContactPage() {
  return (
    <LegalPageShell title="Contact" lastUpdated="July 30, 2026">
      <p>
        Tokscale is built and maintained by Junho Yeo. There are two ways to get
        in touch, and which one is better depends on what you need.
      </p>

      <h2>GitHub — bugs, features, and client support</h2>
      <p>
        For anything about the software itself, open an issue at{" "}
        <a
          href="https://github.com/junhoyeo/tokscale/issues"
          target="_blank"
          rel="noopener noreferrer"
        >
          github.com/junhoyeo/tokscale/issues
        </a>
        . This is the fastest route, and it is the right one for:
      </p>
      <ul>
        <li>Bugs in the CLI or the website</li>
        <li>Support for an AI coding client Tokscale does not yet read</li>
        <li>Token counts or cost estimates that look wrong</li>
        <li>Feature requests</li>
      </ul>
      <p>
        Issues are public, so please do not include API keys, tokens, or
        anything else you would not want published.
      </p>

      <h2>Email — privacy, data, and everything else</h2>
      <p>
        Write to <a href="mailto:i@junho.io">i@junho.io</a> for anything that
        does not belong in a public issue:
      </p>
      <ul>
        <li>
          Privacy questions, or requests about data held about you — see the{" "}
          <Link href="/privacy">privacy policy</Link> for what is collected
        </li>
        <li>Security reports, including anything you would rather not disclose publicly</li>
        <li>Reporting abuse, impersonation, or falsified leaderboard data</li>
        <li>Press, sponsorship, and partnership enquiries</li>
      </ul>

      <h2>Deleting your data yourself</h2>
      <p>
        You do not need to contact anyone to remove your data. Both{" "}
        <strong>delete submitted data</strong> and{" "}
        <strong>delete account</strong> are available directly from{" "}
        <Link href="/settings">your settings page</Link>, and both take effect
        immediately.
      </p>

      <h2>Response times</h2>
      <p>
        Tokscale is a personal open-source project, not a staffed product, so
        replies are best effort rather than guaranteed. Security reports and
        privacy requests are prioritised.
      </p>
    </LegalPageShell>
  );
}
