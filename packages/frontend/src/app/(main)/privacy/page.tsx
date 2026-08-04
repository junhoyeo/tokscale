import type { Metadata } from "next";
import Link from "next/link";
import { LegalPageShell } from "@/components/layout/LegalPageShell";
import { legalUrl } from "@/lib/seo/urls";

const DESCRIPTION =
  "What Tokscale collects, what it never collects, and how to delete your data. Tokscale records token counts and costs only — never prompts, responses, or source code.";

export const metadata: Metadata = {
  title: "Privacy Policy | Tokscale",
  description: DESCRIPTION,
  alternates: { canonical: legalUrl("privacy") },
  openGraph: {
    title: "Privacy Policy | Tokscale",
    description: DESCRIPTION,
    type: "article",
    url: legalUrl("privacy"),
    siteName: "Tokscale",
  },
};

export default function PrivacyPage() {
  return (
    <LegalPageShell title="Privacy Policy" lastUpdated="July 30, 2026">
      <p>
        Tokscale is an open-source tool and public leaderboard for tracking AI
        coding assistant token usage. It is operated by Junho Yeo. The full
        source, including everything described here, is available at{" "}
        <a
          href="https://github.com/junhoyeo/tokscale"
          target="_blank"
          rel="noopener noreferrer"
        >
          github.com/junhoyeo/tokscale
        </a>
        .
      </p>

      <h2>What Tokscale never collects</h2>
      <p>
        The Tokscale CLI reads your local AI assistant session files to count
        tokens. It does not transmit their contents. Specifically, Tokscale
        never collects:
      </p>
      <ul>
        <li>
          <strong>Prompts, responses, or any conversation content</strong>
        </li>
        <li>
          <strong>Source code, file contents, or file names</strong>
        </li>
        <li>
          <strong>API keys or AI provider credentials</strong>
        </li>
      </ul>
      <p>
        Only the aggregate numbers described below leave your machine. You can
        inspect exactly what would be sent before submitting anything.
      </p>

      <h2>Information from your GitHub account</h2>
      <p>
        Signing in uses GitHub OAuth with the <code>read:user</code> and{" "}
        <code>user:email</code> scopes. Tokscale stores your GitHub numeric ID,
        username, display name, avatar URL, and email address. Tokscale never
        requests access to your repositories.
      </p>

      <h2>Usage data you submit</h2>
      <p>When you run a submission, the CLI sends:</p>
      <ul>
        <li>
          Token counts, broken down into input, output, cache read, cache write,
          and reasoning tokens
        </li>
        <li>Estimated cost, derived from public provider pricing</li>
        <li>The date range covered, and a per-day breakdown of the above</li>
        <li>
          Identifiers of the AI clients used (for example Claude Code, Cursor,
          Codex) and of the models used
        </li>
        <li>Message counts — how many messages, never their content</li>
        <li>
          Session timing metrics: total active time, longest continuous session,
          maximum concurrent sessions, and session count
        </li>
        <li>The names of MCP servers configured in your clients</li>
        <li>The version of the Tokscale CLI used</li>
      </ul>
      <p>
        Each machine you submit from is recorded as a device, identified by a
        device key and the device label reported by the CLI.
      </p>

      <h2>What is public</h2>
      <p>
        Tokscale is a public leaderboard, so once you submit, your profile page
        and leaderboard entry are visible to anyone, including people who are
        not signed in. That covers your username, avatar, display name, total
        tokens and cost, contribution graph, per-client and per-model breakdown,
        your devices, and any groups you join.
      </p>
      <p>
        These profile pages are also listed in the Tokscale sitemap, which means
        search engines may crawl and index them. If you do not want that, delete
        your submitted data or your account using the options below.
      </p>

      <h2>Cookies</h2>
      <ul>
        <li>
          <strong>Session cookie</strong> — keeps you signed in after GitHub
          authentication
        </li>
        <li>
          <strong>OAuth state cookie</strong> — short-lived, protects the
          sign-in flow against cross-site request forgery
        </li>
        <li>
          <strong>
            <code>leaderboard-sort-by</code>
          </strong>{" "}
          — remembers whether you sort the leaderboard by tokens or cost
        </li>
      </ul>

      <h2>Analytics and service providers</h2>
      <p>Tokscale relies on these third parties:</p>
      <ul>
        <li>
          <strong>GitHub</strong> — authentication
        </li>
        <li>
          <strong>Vercel</strong> — hosting, and Vercel Analytics for aggregate
          page view statistics
        </li>
        <li>
          <strong>Neon</strong> — the PostgreSQL database where the data above
          is stored
        </li>
      </ul>

      <h2>Advertising</h2>
      <p>
        Tokscale does not currently display advertising and sets no advertising
        cookies. If advertising is introduced, this policy will be updated
        before any ad code is served, and this section will name the ad networks
        involved. Third-party ad vendors commonly use cookies to serve ads based
        on prior visits to a site.
      </p>

      <h2>Deleting your data</h2>
      <p>
        Both options are available from{" "}
        <Link href="/settings">your settings page</Link>:
      </p>
      <ul>
        <li>
          <strong>Delete submitted data</strong> — permanently removes your
          submissions and device records while keeping your account
        </li>
        <li>
          <strong>Delete account</strong> — permanently removes your account and
          every record associated with it, including submissions, devices, API
          tokens, and group memberships
        </li>
      </ul>
      <p>
        Both are immediate and cannot be undone. You can additionally revoke
        Tokscale&apos;s access from your GitHub account settings at any time.
      </p>

      <h2>Changes to this policy</h2>
      <p>
        Material changes will be reflected in the &ldquo;last updated&rdquo; date
        at the top of this page. Because Tokscale is open source, the full
        history of this document is visible in the repository.
      </p>

      <h2>Contact</h2>
      <p>
        Questions about this policy can go to{" "}
        <a href="mailto:i@junho.io">i@junho.io</a> or the{" "}
        <Link href="/contact">contact page</Link>.
      </p>
    </LegalPageShell>
  );
}
