import type { Metadata } from "next";
import Link from "next/link";
import { LegalPageShell } from "@/components/layout/LegalPageShell";
import { legalUrl } from "@/lib/seo/urls";

export const dynamic = "force-dynamic";

const DESCRIPTION =
  "The terms for using Tokscale: acceptable use of the leaderboard, the public nature of submitted data, and the limits of the cost estimates it reports.";

export const metadata: Metadata = {
  title: "Terms of Service | Tokscale",
  description: DESCRIPTION,
  alternates: { canonical: legalUrl("terms") },
  openGraph: {
    title: "Terms of Service | Tokscale",
    description: DESCRIPTION,
    type: "article",
    url: legalUrl("terms"),
    siteName: "Tokscale",
  },
};

export default function TermsPage() {
  return (
    <LegalPageShell title="Terms of Service" lastUpdated="July 30, 2026">
      <p>
        These terms cover your use of the Tokscale website at tokscale.ai and
        the Tokscale CLI. By signing in or submitting usage data, you agree to
        them. If you do not, please do not use the service.
      </p>

      <h2>What Tokscale is</h2>
      <p>
        Tokscale is a free, open-source tool that measures how many tokens you
        spend on AI coding assistants, and a public leaderboard that ranks
        participants by that usage. The software is released under the MIT
        license and is available at{" "}
        <a
          href="https://github.com/junhoyeo/tokscale"
          target="_blank"
          rel="noopener noreferrer"
        >
          github.com/junhoyeo/tokscale
        </a>
        . The MIT license governs the software itself; these terms govern the
        hosted service.
      </p>

      <h2>Accounts</h2>
      <p>
        Accounts are created through GitHub. You are responsible for activity
        that happens under your account and for keeping any API tokens you
        generate secret. If you believe a token has been exposed, revoke it from{" "}
        <Link href="/settings">your settings page</Link>.
      </p>

      <h2>Your data is public</h2>
      <p>
        Tokscale is a public leaderboard. Data you submit — including your
        username, avatar, token totals, estimated costs, contribution history,
        clients and models used, and devices — is visible to anyone and may be
        indexed by search engines. Do not submit data you are not willing to
        make public. What is collected is described in full in the{" "}
        <Link href="/privacy">privacy policy</Link>.
      </p>

      <h2>Cost figures are estimates</h2>
      <p>
        The costs Tokscale reports are calculated from published provider
        pricing applied to observed token counts. They are estimates for
        comparison and curiosity. They are <strong>not</strong> billing
        records, and they will not match your actual invoices — subscription
        plans, free tiers, promotional credits, negotiated rates, and pricing
        changes are not accounted for. Do not rely on them for accounting,
        budgeting, expense reporting, or any financial decision.
      </p>

      <h2>Acceptable use</h2>
      <p>You agree not to:</p>
      <ul>
        <li>
          Submit fabricated, inflated, or otherwise falsified usage data, or
          manipulate your ranking by any means other than genuinely using AI
          coding tools
        </li>
        <li>
          Create multiple accounts to distort the leaderboard, or submit usage
          that is not yours
        </li>
        <li>
          Access the service in a way that places unreasonable load on it,
          including automated scraping outside of the documented API
        </li>
        <li>
          Attempt to gain unauthorized access to accounts, data, or
          infrastructure
        </li>
        <li>
          Use the service to publish unlawful content, or content that harasses
          or impersonates others — including through usernames, group names, and
          device labels
        </li>
      </ul>
      <p>
        Accounts, submissions, or groups that violate these rules may be removed
        without notice.
      </p>

      <h2>Availability</h2>
      <p>
        Tokscale is provided free of charge and with no uptime commitment.
        Features may change or be withdrawn, and the service may be interrupted
        or discontinued at any time. Data may be lost. Keep your own copy of
        anything you would not want to lose — the CLI can export your data
        locally.
      </p>

      <h2>No warranty</h2>
      <p>
        The service is provided &ldquo;as is&rdquo; and &ldquo;as
        available,&rdquo; without warranties of any kind, whether express or
        implied, including any implied warranties of merchantability, fitness
        for a particular purpose, and non-infringement.
      </p>

      <h2>Limitation of liability</h2>
      <p>
        To the maximum extent permitted by law, the operator of Tokscale is not
        liable for any indirect, incidental, special, consequential, or punitive
        damages, or for any loss of data, profits, or goodwill, arising out of
        your use of or inability to use the service.
      </p>

      <h2>Ending your use</h2>
      <p>
        You can delete your submitted data or your entire account at any time
        from <Link href="/settings">your settings page</Link>. Deletion is
        immediate and cannot be undone.
      </p>

      <h2>Changes to these terms</h2>
      <p>
        These terms may be updated. Material changes will be reflected in the
        &ldquo;last updated&rdquo; date above, and the full revision history is
        public in the repository. Continuing to use the service after a change
        means you accept the updated terms.
      </p>

      <h2>Contact</h2>
      <p>
        Questions about these terms can go to{" "}
        <a href="mailto:i@junho.io">i@junho.io</a> or the{" "}
        <Link href="/contact">contact page</Link>.
      </p>
    </LegalPageShell>
  );
}
