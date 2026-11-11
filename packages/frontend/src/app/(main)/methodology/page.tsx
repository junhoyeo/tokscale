import type { Metadata } from "next";
import Link from "next/link";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { MIN_PERCENT_BASE } from "@/components/explore/format";
import { methodologyUrl } from "@/lib/seo/explore";
import styles from "./methodology.module.css";

const TITLE = "Methodology - How Tokscale Counts Models, Agents, and People | Tokscale";
const DESCRIPTION =
  "How Tokscale computes active users, adoption, change, tokens, and estimated cost for AI models and coding agents, who is counted, and what Tokscale never collects.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: methodologyUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "article", url: methodologyUrl(), siteName: "Tokscale" },
};

/** Keep in sync with METHODOLOGY_LAST_MODIFIED in lib/seo/explore.ts. */
const LAST_UPDATED = "October 19, 2026";

export default function MethodologyPage() {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <article className={styles.prose}>
          <h1 className={styles.title}>How we count</h1>
          <p className={styles.updated}>Last updated: {LAST_UPDATED}</p>
          <p className={styles.lead}>
            Every number on the Humans, Models, and Agents pages comes from usage that people chose to submit with the
            open-source Tokscale CLI. This page explains what each number means and where it stops being reliable.
          </p>

          <nav aria-label="On this page">
            <ul className={styles.toc}>
              <li><a href="#source">Where the data comes from</a></li>
              <li><a href="#metrics">Metrics</a></li>
              <li><a href="#who">Who is counted</a></li>
              <li><a href="#models">Model names and Unknown</a></li>
              <li><a href="#updates">Updates</a></li>
              <li><a href="#never">What we never collect</a></li>
              <li><a href="#limits">Limits</a></li>
            </ul>
          </nav>

          <h2 id="source">Where the data comes from</h2>
          <p>
            The Tokscale CLI reads the local session files that coding agents such as Claude Code, Codex CLI, OpenCode,
            Cursor, and Gemini CLI write on your machine. It counts tokens per agent, per model, and per day. Nothing
            leaves your machine until you run <code>tokscale submit</code> while signed in with GitHub.
          </p>
          <p>
            So the numbers describe <strong>people who use Tokscale and submit</strong>, not every developer. Read them as
            a large, self-selected sample.
          </p>

          <h2 id="metrics">Metrics</h2>
          <dl className={styles.metrics}>
            <div className={styles.metric} id="active-users">
              <dt>Active users</dt>
              <dd>
                <p>
                  The number of distinct accounts that submitted any usage of a model (or agent) on at least one day in the
                  period. One account is one GitHub sign-in, across all of its devices.
                </p>
              </dd>
            </div>
            <div className={styles.metric} id="adoption">
              <dt>Adoption</dt>
              <dd>
                <p>
                  Active users of a model divided by all accounts with any usage in the same period. If 1,000 accounts were
                  active and 360 used a model, its adoption is 36%.
                </p>
                <p>
                  One account can use several models, so adoption across models adds up to more than 100%. That is
                  expected, not an error.
                </p>
              </dd>
            </div>
            <div className={styles.metric} id="previous-window">
              <dt>Previous window and change</dt>
              <dd>
                <p>
                  For the 7-day and 30-day views, the change compares active users with the window of the same length
                  right before it. We show how many users were added or lost. We add a percentage only when the previous
                  window had at least {MIN_PERCENT_BASE} active users, because a large percentage on a small base says
                  little.
                </p>
                <p>
                  <strong>New</strong> means the model had no usage in the previous window. All time has no previous
                  window, so it shows no change.
                </p>
              </dd>
            </div>
            <div className={styles.metric} id="tokens">
              <dt>Tokens</dt>
              <dd>
                <p>
                  Input, output, cache read, cache write, and reasoning tokens added together, as the agent recorded
                  them. Cache reads are often most of an agent&apos;s tokens, because agents resend context on every
                  turn. Model pages show the split as the token mix.
                </p>
              </dd>
            </div>
            <div className={styles.metric} id="estimated-cost">
              <dt>Est. cost</dt>
              <dd>
                <p>
                  An estimate from public list prices, computed by the CLI when the usage is submitted. Prices come mainly
                  from LiteLLM&apos;s public pricing data, with OpenRouter and other public sources as fallbacks. Cache tokens use their own, lower
                  prices where the provider publishes them.
                </p>
                <p>
                  It is not what anyone paid. Subscriptions, free tiers, discounts, and enterprise contracts are not
                  visible to Tokscale, and usage of a model with no public price adds no cost. That is why cost is always
                  labelled &ldquo;Est. cost&rdquo;.
                </p>
                <p>
                  When a model has usage but its submitted usage carries no cost (a free model, or one with no public
                  price), the cost shows as &ldquo;&mdash;&rdquo; instead of $0.
                </p>
              </dd>
            </div>
            <div className={styles.metric} id="periods">
              <dt>Periods and rank</dt>
              <dd>
                <p>
                  Periods are whole UTC days. Each table shows its exact dates in the coverage line above it. Rank follows
                  the selected sort: active users by default, or tokens.
                </p>
              </dd>
            </div>
          </dl>

          <h2 id="who">Who is counted</h2>
          <ul>
            <li>Every account that submitted usage in the period counts, unless it is hidden.</li>
            <li>
              <strong>Hidden accounts are excluded everywhere.</strong> Accounts that moderation hid from the leaderboard
              count in no total, no ranking, and no adoption denominator.
            </li>
            <li>Lists of people, such as the top profiles on a model page, show public profiles only.</li>
          </ul>

          <h2 id="models">Model names and Unknown</h2>
          <p>
            Agents report model names in different forms. Tokscale folds them into one key: lowercase, without a trailing
            date such as <code>-20260801</code>, without a reasoning-tier suffix, and without an{" "}
            <code>anthropic/</code> prefix. Model pages list the other names a model was reported as.
          </p>
          <p>
            When an agent does not record the model, or the name cannot be matched to a known model, the usage counts as{" "}
            <strong>Unknown</strong>. It stays in every total and is never dropped. The coverage line on each table shows
            the share of tokens that came from an identified model.
          </p>

          <h2 id="updates">Updates</h2>
          <p>
            Numbers change only when people submit. Each table shows the time of the newest submission it includes
            (&ldquo;Data through&rdquo;). Rankings are cached for up to 10 minutes, so a new submission can take that long
            to appear. We never label these pages as live.
          </p>

          <h2 id="never">What Tokscale never collects</h2>
          <ul>
            <li>Prompts, responses, or any conversation content</li>
            <li>Source code, file contents, or file names</li>
            <li>API keys or AI provider credentials</li>
          </ul>
          <p>
            Only usage counts, estimated costs, dates, agent and model names, session timing, and a few technical details
            leave your machine. The <Link href="/privacy">privacy policy</Link> lists every field.
          </p>

          <h2 id="limits">Limits</h2>
          <ul>
            <li>The sample is self-selected: people who installed Tokscale and chose to submit.</li>
            <li>Only agents that the CLI can read are counted. Usage in other tools is invisible.</li>
            <li>A person with two GitHub accounts counts twice.</li>
            <li>Costs are estimates at list price, not spend.</li>
          </ul>
          <p>
            The code that computes all of this is open source at{" "}
            <a href="https://github.com/junhoyeo/tokscale" target="_blank" rel="noopener noreferrer">
              github.com/junhoyeo/tokscale
            </a>
            .
          </p>
        </article>
      </main>
      <PublicFooter />
    </div>
  );
}
