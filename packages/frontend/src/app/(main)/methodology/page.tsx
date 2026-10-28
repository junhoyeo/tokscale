import type { Metadata } from "next";
import Link from "next/link";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { MIN_PERCENT_BASE } from "@/components/explore/format";
import { DitherBar } from "@/components/explore/DitherViz";
import { PrimaryActionLink } from "@/components/leaderboard/RankingUI";
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

/**
 * The metrics table's shape, drawn from the page's own subject: the fields a
 * real row carries and how a missing value is written. Values are illustrative
 * and labelled as such; every real row on the site links to its own source.
 */
function RowFragment() {
  return (
    <div className={styles.fragment} data-ui>
      <p className={styles.fragmentTitle}>One row of the models table</p>
      <table className={styles.fragmentTable}>
        <thead>
          <tr>
            <th scope="col">Model</th>
            <th scope="col">Active users</th>
            <th scope="col">Change</th>
            <th scope="col">Adoption</th>
            <th scope="col">Est. cost</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>Claude Opus 5.5</td>
            <td>57</td>
            <td>+11 (+24%)</td>
            <td>
              <span className={styles.adoption}>
                <span className={styles.adoptionBar}>
                  <DitherBar value={0.56} />
                </span>
                56%
              </span>
            </td>
            <td>$0.45</td>
          </tr>
          <tr>
            <td>Gemini 3.7 Flash</td>
            <td>5</td>
            <td>New +5</td>
            <td>
              <span className={styles.adoption}>
                <span className={styles.adoptionBar}>
                  <DitherBar value={0.05} />
                </span>
                5%
              </span>
            </td>
            <td>—</td>
          </tr>
          <tr>
            <td>Claude Opus 4.7</td>
            <td>4</td>
            <td>−3 (−43%)</td>
            <td>
              <span className={styles.adoption}>
                <span className={styles.adoptionBar}>
                  <DitherBar value={0.04} />
                </span>
                4%
              </span>
            </td>
            <td>$0.80</td>
          </tr>
        </tbody>
      </table>
      <p className={styles.fragmentNote}>
        Illustrative row values, shown to explain the fields. The change is always against the window of the same length
        right before; a dash means the model has no public price, not that it is free.
      </p>
    </div>
  );
}


/**
 * Three more crops of the interface the numbers live in: an agent breakdown
 * with its dithered share bar, one score cell with its conditions, and the
 * coverage line that dates every table.
 */
function AgentFragment() {
  return (
    <div className={styles.fragment} data-ui>
      <p className={styles.fragmentTitle}>An agent breakdown on a model page</p>
      <ul className={styles.shareList}>
        <li>
          <span className={styles.shareName}>Claude Code</span>
          <span className={styles.shareBar}><DitherBar value={0.9} /></span>
          <span className={styles.shareValue}>90%</span>
          <span className={styles.shareMeta}>146 active users</span>
        </li>
        <li>
          <span className={styles.shareName}>Codex CLI</span>
          <span className={styles.shareBar}><DitherBar value={0.08} /></span>
          <span className={styles.shareValue}>8%</span>
          <span className={styles.shareMeta}>12 active users</span>
        </li>
        <li>
          <span className={styles.shareName}>Gemini CLI</span>
          <span className={styles.shareBar}><DitherBar value={0.02} /></span>
          <span className={styles.shareValue}>2%</span>
          <span className={styles.shareMeta}>3 active users</span>
        </li>
      </ul>
      <p className={styles.fragmentNote}>
        Each share is that agent&apos;s part of the model&apos;s tokens in the period. Illustrative values.
      </p>
    </div>
  );
}

function ScoreFragment() {
  return (
    <div className={styles.fragment} data-ui>
      <p className={styles.fragmentTitle}>One score cell on a benchmark page</p>
      <dl className={styles.scoreFacts}>
        <div>
          <dt>Score</dt>
          <dd>64.8%</dd>
        </div>
        <div>
          <dt>Conditions</dt>
          <dd>Max effort · Claude Code</dd>
        </div>
        <div>
          <dt>Unit</dt>
          <dd>% of tasks solved</dd>
        </div>
        <div>
          <dt>Run date</dt>
          <dd>Oct 6, 2026</dd>
        </div>
        <div>
          <dt>Source</dt>
          <dd>tbench.ai</dd>
        </div>
      </dl>
      <p className={styles.fragmentNote}>
        A score keeps the source, unit and run conditions it came from. Benchmarks are never combined into one score.
        Illustrative values.
      </p>
    </div>
  );
}

function CoverageFragment() {
  return (
    <div className={`${styles.fragment} ${styles.field}`} data-ui>
      <p className={styles.fragmentTitle}>The coverage line above every table</p>
      <dl className={styles.coverage}>
        <div>
          <dt>Period</dt>
          <dd>Sep 29 – Oct 28, 2026 (UTC)</dd>
        </div>
        <div>
          <dt>Accounts</dt>
          <dd>101</dd>
        </div>
        <div>
          <dt>Identified</dt>
          <dd>100% of tokens</dd>
        </div>
        <div>
          <dt>Data through</dt>
          <dd>Oct 28, 2026, 07:38 UTC</dd>
        </div>
      </dl>
      <p className={styles.fragmentNote}>
        The dates bound the period, the account count is the adoption denominator, and the last stamp is the newest
        submission included. Illustrative values.
      </p>
    </div>
  );
}

export default function MethodologyPage() {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <article className={styles.page}>
          <header className={styles.header}>
            <h1 className={styles.title}>How we count models, agents and people</h1>
            <p className={styles.updated}>Last updated {LAST_UPDATED}</p>
            <p className={styles.lead}>
              Every number on the Humans, Models and Agents pages comes from usage that people chose to submit with the
              open-source Tokscale CLI. This page explains what each number means and where it stops being reliable.
            </p>
            <p style={{ margin: "0 0 28px" }}>
              <PrimaryActionLink href="/models">See the models the numbers describe</PrimaryActionLink>
            </p>
          </header>

          <div className={styles.layout}>
            <nav className={styles.rail} aria-label="On this page">
            <p className={styles.railTitle}>On this page</p>
            <ul className={styles.toc}>
              <li><a href="#source">Where the data comes from</a></li>
              <li><a href="#metrics">Metrics</a></li>
              <li><a href="#who">Who is counted</a></li>
              <li><a href="#models">Model names and Unknown</a></li>
              <li><a href="#anatomy">How to read a row</a></li>
              <li><a href="#updates">Updates</a></li>
              <li><a href="#never">What we never collect</a></li>
              <li><a href="#limits">Limits</a></li>
            </ul>
            <p className={styles.railNote}>
              Every table on the site carries its own coverage line. When two numbers disagree, the coverage line is the
              authority on which period and which accounts they describe.
            </p>
            </nav>

            <div className={styles.body}>
            <h2 id="source">Where the data comes from</h2>
            <p>
              The Tokscale CLI reads the local session files that coding agents such as Claude Code, Codex CLI, OpenCode,
              Cursor, and Gemini CLI write on your machine. It counts tokens per agent, per model, and per day. Nothing
              leaves your machine until you run <code>tokscale submit</code> while signed in with GitHub.
            </p>
            <p>
              So the numbers describe <strong>people who use Tokscale and submit</strong>, not every developer. Read them
              as a large, self-selected sample.
            </p>

            <h2 id="metrics">Metrics</h2>
            <dl className={styles.metrics}>
              <div className={styles.metric} id="active-users">
                <dt>Active users</dt>
                <dd>
                  <p>
                    The number of distinct accounts that submitted any usage of a model (or agent) on at least one day in
                    the period. One account is one GitHub sign-in, across all of its devices.
                  </p>
                </dd>
              </div>
              <div className={styles.metric} id="adoption">
                <dt>Adoption</dt>
                <dd>
                  <p>
                    Active users of a model divided by all accounts with any usage in the same period. If 1,000 accounts
                    were active and 360 used a model, its adoption is 36%.
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
                    An estimate from public list prices, computed by the CLI when the usage is submitted. Prices come
                    mainly from LiteLLM&apos;s public pricing data, with OpenRouter and other public sources as fallbacks.
                    Cache tokens use their own, lower prices where the provider publishes them.
                  </p>
                  <p>
                    It is not what anyone paid. Subscriptions, free tiers, discounts, and enterprise contracts are not
                    visible to Tokscale, and usage of a model with no public price adds no cost. That is why cost is
                    always labelled &ldquo;Est. cost&rdquo;.
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
                    Periods are whole UTC days. Each table shows its exact dates in the coverage line above it. Rank
                    follows the selected sort: active users by default, or tokens.
                  </p>
                </dd>
              </div>
            </dl>

            <RowFragment />

            <h2 id="who">Who is counted</h2>
            <ul>
              <li>Every account that submitted usage in the period counts, unless it is hidden.</li>
              <li>
                <strong>Hidden accounts are excluded everywhere.</strong> Accounts that moderation hid from the
                leaderboard count in no total, no ranking, and no adoption denominator.
              </li>
              <li>Lists of people, such as the top profiles on a model page, show public profiles only.</li>
            </ul>

            <h2 id="models">Model names and Unknown</h2>
            <p>
              Agents report model names in different forms. Tokscale folds them into one key: lowercase, without a
              trailing date such as <code>-20260801</code>, without a reasoning-tier suffix, and without an{" "}
              <code>anthropic/</code> prefix. Model pages list the other names a model was reported as.
            </p>
            <p>
              When an agent does not record the model, or the name cannot be matched to a known model, the usage counts
              as <strong>Unknown</strong>. It stays in every total and is never dropped. The coverage line on each table
              shows the share of tokens that came from an identified model.
            </p>


            <h2 id="anatomy">How to read a row</h2>
            <p>
              Every number on a table row has a definition above it and a source beside it. Three crops of the interface
              show where each fact sits.
            </p>
            <div className={styles.fragmentGrid}>
              <AgentFragment />
              <ScoreFragment />
            </div>
            <div className={styles.fullWidth}>
              <CoverageFragment />
            </div>

            <h2 id="updates">Updates</h2>
            <p>
              Numbers change only when people submit. Each table shows the time of the newest submission it includes
              (&ldquo;Data through&rdquo;). Rankings are cached for up to 10 minutes, so a new submission can take that
              long to appear. We never label these pages as live.
            </p>

            <div className={styles.split}>
              <section aria-labelledby="never">
                <h2 id="never">What Tokscale never collects</h2>
                <ul>
                  <li>Prompts, responses, or any conversation content</li>
                  <li>Source code, file contents, or file names</li>
                  <li>API keys or AI provider credentials</li>
                </ul>
                <p>
                  Only usage counts, estimated costs, dates, agent and model names, session timing, and a few technical
                  details leave your machine. The <Link href="/privacy">privacy policy</Link> lists every field.
                </p>
              </section>
              <section aria-labelledby="limits">
                <h2 id="limits">Limits</h2>
                <ul>
                  <li>The sample is self-selected: people who installed Tokscale and chose to submit.</li>
                  <li>Only agents that the CLI can read are counted. Usage in other tools is invisible.</li>
                  <li>A person with two GitHub accounts counts twice.</li>
                  <li>Costs are estimates at list price, not spend.</li>
                </ul>
              </section>
            </div>

            <div className={styles.close}>
              <p className={styles.closeLead}>
                The code that computes all of this is open source at{" "}
                <a href="https://github.com/junhoyeo/tokscale" target="_blank" rel="noopener noreferrer">
                  github.com/junhoyeo/tokscale
                </a>
                .
              </p>
              <ul className={styles.closeLinks}>
                <li><Link href="/models">Models and benchmarks</Link></li>
                <li><Link href="/capital">Capital ledger</Link></li>
                <li><Link href="/leaderboard">Leaderboard</Link></li>
                <li><Link href="/privacy">Privacy policy</Link></li>
              </ul>
            </div>
          </div>
          </div>
        </article>
      </main>
      <PublicFooter />
    </div>
  );
}
