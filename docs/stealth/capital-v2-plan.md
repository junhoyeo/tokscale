# Capital v2 plan: AI-focused, Crunchbase-like, better organized

Status: plan only. No code was changed. Nothing was committed.
Branch context: `stealth/ai-v2` worktree at `/Users/junhoyeo/tokscale-worktrees/stealth-ai-v2` (local only).
Research date: 2026-10-20 (dates on SEC and Epoch AI pages confirm the environment date).
Evidence folder: `/tmp/tokscale-stealth/capital-plan/evidence/` (raw responses, notes, samples, probe log).

Owner ask (verbatim): "Capital 도 AI Focused + Crunchbase 느낌으로, 좀더 잘 정돈되고 구체적으로 만들도록 지금은 너무 13F같다 아이디어 좀 찾아줘"

---

## 0. Summary

Today Capital is a 13F reader. Its unit is a filing, its hero metric is "managers tracked", and its
company pages exist only for US-listed stocks that two managers held. AI labs (Anthropic, OpenAI,
xAI, Mistral, DeepSeek, Moonshot) have no page at all.

Capital v2 changes the unit from "filing" to **entity + event**:

- Entities: companies (AI labs, clouds, neoclouds, chips, power and data centers) and investors.
- Events: funding rounds, compute and infrastructure deals, acquisitions, IPOs, and filings.
- Metrics: capital spending (capex) by quarter, fund marks, company-stated revenue.

Each event has one ID and one page. Every participant page links to it. Every number has a money
type, a source, a date and a status. 13F becomes one source among many, in a "Public investors" area.

### The 10 most important ideas

1. **Entities and events first, filings second.** Directory of ~33 AI-economy companies and ~16
   investors. Each round or deal is one page with one ID, shared by all participants.
2. **A typed money ledger.** A closed list of money types (equity raised, debt, post-money valuation,
   contract value, compute commitment, cash capex, capex guidance, fund mark, 13F value, ...). Totals
   are allowed only inside one type and one basis. The build fails on a mixed total.
3. **"Included in" links stop double counting.** Example (verified): Anthropic's Series H ($65B at
   $965B post-money, May 28, 2026) "includes $15 billion of previously committed investments from
   hyperscalers, including $5 billion from Amazon". Amazon's $5B (Apr 20, 2026) is a separate
   announcement. The model links the two and counts the $5B once.
4. **Compute ledger (`/capital/compute`).** Contracts in dollars and in MW/GW, from the public side's
   SEC filings. Verified examples: CoreWeave and OpenAI, "up to approximately $6.5 billion through
   May 31, 2031" (8-K); Akamai and Anthropic, "approximately $11.6 billion", 7-year terms (8-K);
   TeraWulf and Anthropic, "approximately 401 MW of critical IT load", 20-year lease (8-K);
   Broadcom, Google and Anthropic, "approximately 3.5 gigawatts" from 2027 (8-K); Nebius and
   Microsoft, "about $17.4 billion through 2031", up to "about $19.4 billion" with options (6-K).
5. **Spending tracker from SEC XBRL (`/capital/spending`).** Cash paid for property and equipment,
   per company and fiscal period, with the filing link. Verified: Microsoft FY2026 $115.948B,
   Amazon FY2025 $131.819B, Alphabet FY2025 $91.447B, Meta FY2025 $69.691B, Oracle FY2026 $55.663B,
   CoreWeave FY2025 $10.309B, Nebius FY2025 $4.066B.
6. **AI lab profiles that join capital with Tokscale data.** Funding, investors, compute, then the
   lab's models on Tokscale (usage among Tokscale users) and benchmark standing. No other tracker
   can show "money in" next to "who actually uses the models".
7. **Fund marks from SEC N-PORT.** Registered funds report fair values of private AI shares.
   Verified: ARK Venture Fund, period 2026-07-31, Anthropic 89,078 shares at $49,614,780
   (about $556.98 per share, Level 3); OpenAI Group PBC Series C 72,707 shares at $49,303,475
   (about $678.11 per share). Labeled "fund's mark, not a valuation".
8. **Seed fast with Epoch AI (CC BY 4.0).** Funding rounds with equity and debt in separate columns,
   post-money, status, confidence and source URLs (48 rounds, 7 labs). Frontier data centers
   (95 sites: owner, users, MW, H100e, estimated cost). Chip sales estimates. All downloadable.
9. **Crunchbase-style directory, Tokscale-style density.** Dense tables, filters, URL-saved views,
   compare, and sector hubs (AI labs, neoclouds, hyperscalers, chips, power and data centers).
   33 deep profiles, not millions of thin ones.
10. **Filing radar feeds an editor, not the site.** SEC full-text search, SEC submissions, company
    RSS and sitemaps, and GDELT news find candidate events. An editor checks the primary source.
    Nothing auto-publishes.

Also important: **show disagreement honestly.** On 2026-10-20, five trackers showed five different
"total raised" numbers for Anthropic ($127B, $132B, $139.4B equity, $203.455B, $6.98B). The cause is
mixed money types and stale rows. Tokscale should state its definition next to every total.

---

## 1. Where Capital is today (read from code and the dev server)

Code: `packages/frontend/src/app/(main)/capital/**`, `src/app/(main)/capital/companies/[slug]`,
`src/content/capital/**`, `src/lib/capital/**`.

| Item | Today |
|---|---|
| Published funds | 2: Thiel Macro (CIK 1562087), Pershing Square (two filer CIKs). 2 in review: TCI, Atreides. |
| Companies | 27, all US-listed, keyed by SEC CIK and CUSIP issuer (`companies.json`). |
| Company pages | Only for companies a tracked manager held (`dynamicParams = false`). Title: "... Among Tracked 13F Portfolios". |
| Themes | Power and the grid, Cloud and AI platforms, Oil/gas/nuclear, Payments and market data. |
| Model to company links | `companySlugForLab()` links a lab only if a 13F company page exists (google to alphabet alias, meta, nvidia). Anthropic, OpenAI, xAI, Mistral, DeepSeek, Moonshot and others get no company page. |
| Hero | "Follow capital across the AI economy." Meta line: "2 managers tracked · Latest period Q2 2026". |
| Home preview | `getFeaturedCapital()` feeds 13F stories into the home page. |

Why it feels like a 13F site:

- The featured object is a quarterly filing. The side feed is "Recent filings" (13F-HR, 13F-NT).
- Company pages show CIK, SIC code and CUSIP, and nothing about funding, deals or models.
- No private companies. No rounds. No compute. No capex.
- `/capital/companies/nvidia` (seen on the dev server) says "Not held by any of the 2 managers Tokscale tracks in Q2 2026" and "Tokscale has not written a description of this company yet." That is the whole page.

Keep from today (these are good and should carry over):

- Build-time validation with zod plus cross-checks that throw (`content.ts`). Wrong numbers fail the build.
- Exact numbers copied from primary documents, with links. "Among managers Tokscale tracks" wording.
- The story format ("Inside the 13F"), OG images, sitemap `lastModified` from real content dates.
- The SIC sector mapping (ported from paracosm-studio) for public-company context.

---

## 2. Research: how Crunchbase-like products present AI companies and investors

### 2.1 What each product shows (observed 2026-10-20)

| Product | What we could see | Entity types | Profile sections | List and search UX | Notes |
|---|---|---|---|---|---|
| Crunchbase | Org and hub pages returned a bot check: "We must verify your session before you can proceed" (Cloudflare). Not observed directly. Docs observed at `data.crunchbase.com/llms.txt`. | From docs: Organization, Person, FundingRound, Acquisition, Investment, Fund, Ipo, Ownership, Event, Category, CategoryGroup, Location, Job, Layoff, KeyEmployeeChange, PartnershipAnnouncement, Product, ProductLaunch, PressReference, LegalProceeding, plus prediction entities. FundingRound cards: `investments, investors, lead_investors, organization, partners, press_references`. Acquisition cards: `acquiree_organization, acquirer_organization, press_references`. | Org cards in docs include founders, headquarters, parent and child organizations, investors, IPOs, jobs, acquisitions. | robots.txt disallows `/search`, `/discover`, `/textsearch` for generic crawlers; these are app pages. | API is Enterprise, "available through the Crunchbase sales team". License: licensee "may not license, sublicense, sell, offer to sell, distribute or otherwise provide any Crunchbase data to any third parties". Attribution with a hyperlink is required. Reference only. |
| PitchBook (public profile) | `pitchbook.com/profiles/company/466959-97` (Anthropic), readable. | Company, investor, deal, fund, people, service provider (from footer data menus). | General info (verticals, office), Timeline, **Valuation and Funding table** (Deal Type, Date, Amount, Raised to Date, Post-Val, Status: Announced / Upcoming / Completed, Stage), **Cap table** (series, shares authorized, par value, original issue price, liquidation preference, conversion price, % owned), Exit predictor, Comparisons, Competitors (17), Research, News, Patents, Signals (web traffic), **Investors (310)** table (name, type, holding, investor since, participating rounds), Investments and Acquisitions (39), Subsidiaries, FAQs. | Gated beyond 5 rows. | FAQ says "Anthropic has raised $127B." Shows IPO row "Announced". |
| CB Insights | `cbinsights.com/company/anthropic`, readable. | Company, investor, collections. | Tabs: Overview and Products, Financials, People, Alternatives and Competitors, Customers. Header: Founded, Stage ("Debt - II \| Alive"), Total Raised ($203.455B), Last Raised ("$42B \| 20 days ago"), Revenue (gated), Mosaic Score. Expert Collections, research briefs, patents, news feed, FAQ, compare. | Curated "Expert Collections" with counts (for example "Unicorns- Billion Dollar Startups, 1,389 items"; "Generative AI 50"). | Total includes debt. |
| Tracxn | `tracxn.com/d/companies/anthropic/__...`, readable. | Company, legal entity, investor, founder. | "Last updated: October 20, 2026", Claim profile, Suggest edits. Key metrics (founded, location, stage, total funding "$132B in 19 rounds", latest round, post-money "$380B as on Jun 01, 2026", investors, rank, revenue, employees). **Legal entities** (ANTHROPIC PBC, ANTHROPIC IRELAND, LIMITED). Funding table with **lead investors** and **other investors** per round. Founders and board. Competitors ranked by score. Investments and acquisitions. | Filterable round table. | Internal inconsistency: Series H at $965B in the table, header still says $380B. |
| Sacra | `sacra.com/c/anthropic/`, readable. | Company, category, report. | Header KPIs (Revenue $65.00B 2026, Valuation $965.00B 2026, Funding $6.98B 2024, Growth 800%). Tabs: Overview, Reports, News, **Filings**, Revenue. Narrative revenue estimates ("Sacra estimates ..."). | Category pages. | Paid ("Unlock Full Access"). Funding header is stale. |
| Epoch AI | `epoch.ai/data/ai-companies`, "Updated Oct. 8, 2026". | AI company, funding round, revenue report, staff report, usage report, compute spend; data centers; chip sales; models. | Tabs: Revenue, Funding rounds, Staff, Usage, Compute spend. Graph and Table views. Settings: annualized revenue, linear or log scale, regressions. Documentation, downloads, citation. | Small, curated, sourced. | CC BY 4.0. Best free model for us. |
| The Information | `theinformation.com/org-charts`. | Person, reporting line. | "reporting lines for 3,000 executives and directors across nearly 70 companies". | Pro subscribers only. | Reference only. |
| Dealroom | `app.dealroom.co/companies/anthropic` returned "Page not found" (slug unknown). API docs at `developers.dealroom.co`. | Companies, funds, investors, founders, funding rounds, valuations, taxonomy (from API docs). | Not observed. | API with credits. | Paid. Reference only. |

Saved copies: `evidence/pitchbook-anthropic-profile.md`, `evidence/www.cbinsights.com_company_anthropic.md`,
`evidence/tracxn.com_d_companies_anthropic_*.md`, `evidence/sacra.com_c_anthropic_.md`,
`evidence/epoch.ai_data_ai-companies.md`, `evidence/crunchbase-llms.txt`.

### 2.2 Patterns to adopt

- **Header facts row** (founded, HQ, status, last round, last valuation, total raised). Every
  tracker leads with it. We add a date, a status and a source link to each fact.
- **Funding table with lead investors per round** (PitchBook, Tracxn). Columns: date, round,
  amount, post-money, leads, others, status.
- **Investors table** with "investor since" and "participating rounds" (PitchBook).
- **Legal entities** block (Tracxn). Useful for us because SEC and fund filings use legal names
  ("Anthropic PBC", "OpenAI Group PBC", "Crusoe Inc.", "Lambda, Inc.").
- **Investment as the join entity** between round and investor, with a lead flag (Crunchbase
  `investments`, `lead_investors`, `partners` cards).
- **Curated lists with counts** (CB Insights collections, Crunchbase hubs) become our sector hubs.
- **Graph and table toggle**, log scale, annualized revenue (Epoch).
- **"Last updated" date and "Suggest edits"** on every profile (Tracxn).

### 2.3 Patterns to avoid

- Totals that mix debt, equity, commitments and secondary sales (CB Insights $203.455B "Total
  Raised" with stage "Debt - II").
- Stale header numbers that contradict the table (Tracxn $380B header vs $965B Series H row; Sacra
  "Funding $6.98B 2024").
- Predictions and scores (exit predictor, Mosaic score, rank). We do not speculate.
- Gated rows. Our pages show all rows we have, or say "not tracked yet".
- Thin auto-generated profiles for millions of companies.

### 2.4 Trackers disagree: Anthropic "total raised" on 2026-10-20

| Tracker | Shown | Likely cause |
|---|---|---|
| PitchBook | "Anthropic has raised $127B." | Own definition (not stated on the page). |
| Tracxn | "$132B in 19 rounds" | Own definition (19 rounds). |
| Epoch AI | `Total equity funding` = 139,404,000,000 | Equity only; may include hyperscaler rows that a later round also counts. |
| CB Insights | "Total Raised $203.455B" | Includes debt ("Debt - II"). |
| Sacra | "FUNDING $6.98B 2024" | Stale header. |

Tokscale rule: show "Equity raised in closed primary rounds Tokscale tracks: $X in N rounds.
Excludes debt, secondary sales, and commitments already counted inside a later round." Link to the
method. Never show one unlabeled "total raised".

---

## 3. Data sources we can legally use (probed 2026-10-20)

Rules used here: bounded requests, SEC fair-access User-Agent with a contact and at most 10
requests per second, no bulk scraping of paid sites, robots.txt respected. All probes are in
`evidence/probe-log.json` (149 HTTP requests; about 22 more page reads went through the Aside browser).

### 3.1 Source table

| # | Source | Access | License / terms | Fields we get | Freshness | Verified | Evidence |
|---|---|---|---|---|---|---|---|
| 1 | SEC EDGAR submissions API (`data.sec.gov/submissions/CIK##########.json`) | JSON, no key. Declared User-Agent; SEC "reserves the right to limit request rates". | "Information presented on sec.gov is considered public information and may be copied or further distributed ... without the SEC's permission. Please consider appropriate citation" (sec.gov privacy page). | Legal name, former names, SIC, state, tickers, every filing (form, date, accession, primary document). | Same day. | Yes. SpaceX, Cerebras, CoreWeave, Nebius, Nscale, Crusoe, Lambda, Fluidstack, Perplexity, Databricks, MiniMax ADR, Knowledge Atlas ADR. | `raw_data.sec.gov_submissions_*` |
| 2 | SEC XBRL APIs (`companyfacts`, `companyconcept`, `frames`) | JSON, no key. | Public. | Tagged financial facts with period, form, accession, filed date. Capex = cash paid for PP&E. | After each 10-Q / 10-K / 20-F. | Yes. 13 companies through `companyfacts` / `companyconcept`, plus Nebius through `frames`. | `sec-capex-xbrl-sample.json` |
| 3 | SEC full-text search (`efts.sec.gov/LATEST/search-index?q=...`) | JSON, no key. Entity autocomplete with `keysTyped=`. | Public. | Filings that mention a name, with form, date, filer. | Near real time. | Yes. Saw transient errors ("getaddrinfo EBUSY") on 2 of ~25 calls; retry needed. | `evidence-notes.json` |
| 4 | SEC Form D (`.../primary_doc.xml`) and Form D data sets (quarterly ZIP, through 2026q3) | XML / ZIP. | Public. | Issuer, year of incorporation, related persons (officers, directors), industry, revenue range, date of first sale, total offering amount, amount sold, number of investors. | Filed within 15 days of first sale. | Yes. Crusoe Form D filed 2026-09-17: offering $3,901,017,922; sold $3,108,543,889; 108 investors; first sale 2026-08-31; "include the amount of certain previously issued simple agreements for future equity". | `evidence-notes.json` (`crusoe_form_d`) |
| 5 | SEC 8-K / 6-K / S-1 / 10-Q text (`www.sec.gov/Archives/...`) | HTML. | Public. | Contract terms: counterparties, value, basis ("up to", "approximately"), term, MW/GW, warrants. Segment revenue. Merger facts. | Within 4 business days for 8-K. | Yes. CoreWeave/OpenAI, Akamai/Anthropic, TeraWulf/Anthropic, Broadcom/Google/Anthropic 8-Ks; SpaceX 10-Q (xAI merger, AI segment). | `evidence-notes.json` |
| 6 | SEC N-PORT (`primary_doc.xml`) and N-PORT data sets (quarterly ZIP) | XML / ZIP. | Public. | Registered funds' holdings, including private AI shares: name, shares, value, % of net assets, fair-value level. | Public with a lag (period end 2026-07-31 filed 2026-09-18). | Yes. ARK Venture Fund. Full-text search also finds "OpenAI Group" in Fidelity Select Portfolios N-PORT filings. | `ark-venture-nport-ai-positions.json` |
| 7 | SEC 13F (already in the product) | XML. | Public. | Quarter-end long US-listed positions of managers. | 45 days after quarter end. | Yes (existing pipeline). | repo |
| 8 | SEC IAPD (`api.adviserinfo.sec.gov/search/firm`) | JSON, no key. | Public. | Adviser identity (SEC number, other names, address). Form ADV has regulatory AUM (not probed). | Ongoing. | Partly. Found "ANDREESSEN HOROWITZ", SEC number 801-114985. | `raw_api.adviserinfo.sec.gov_*` |
| 9 | Epoch AI, AI companies (`epoch.ai/data/ai_companies.zip` and CSVs) | CSV / ZIP, no key. | CC BY 4.0 ("free to use, distribute, and reproduce provided the source and authors are credited"). | Funding rounds (close date, status, equity, debt, post-money, type, confidence, 3 sources, notes), revenue reports, staff, usage, compute spend. 11 companies. | Docs: "updated daily". Page: "Updated Oct. 8, 2026". | Yes. 48 rounds (OpenAI 17, Anthropic 15, Mistral 6, xAI 5, Z.ai 2, MiniMax 2, DeepSeek 1). | `epoch-funding-rounds-sample.csv` |
| 10 | Epoch AI, AI data centers (`epoch.ai/data/data_centers/data_centers.zip`) | ZIP, no key. | CC BY 4.0 (README in the ZIP). | 95 sites: owner, users (with #confident/#likely/#speculative tags), current power MW, H100 equivalents, estimated capital cost (2025 USD), investors, builders, energy companies, country, address; 567 timeline rows. | "Updated Oct. 20, 2026". | Yes. | `epoch-data-centers-sample.csv`, `epoch_data_centers.zip` |
| 11 | Epoch AI, AI chip sales and ML hardware | CSV / ZIP. | CC BY 4.0. | Quarterly unit and H100e estimates by chip, with 5th/95th percentiles and cost estimates; chip organizations. | "Updated Oct. 20, 2026". | Yes. | `raw_epoch.ai_data_ai_chip_sales_*` |
| 12 | Wikidata SPARQL (`query.wikidata.org`) | SPARQL, no key. Descriptive User-Agent. | Structured data under CC0 (Wikidata:Licensing). | QID, inception, HQ, country, website, founders, parent org, employees, Crunchbase ID (P2088) as a crosswalk. | Community edits. | Yes. Anthropic Q116758847 (inception 2021-01-26), OpenAI Q21708200, Mistral AI Q119718658, DeepSeek Q131577453 (parent High-Flyer), Moonshot AI Q130270266, CoreWeave Q121875386, Nebius Group Q47120756. | `wikidata-sample.json` |
| 13 | Hugging Face Hub API (`huggingface.co/api/...`) | JSON, no key. | Hub terms of service. | Org: models count, followers, verified flag. Models: downloads, likes, created date, license tag. | Live. | Yes. 19 orgs (for example deepseek-ai 106 models, Qwen 469, mistralai 81, zai-org 154, moonshotai 19). | `hf-org-overview.json` |
| 14 | GitHub REST API (`api.github.com/orgs/...`) | JSON; 60 requests per hour without a token. | GitHub terms. | Org name, public repos, followers, created date, verified flag. | Live. | Yes. `anthropics`: 115 public repos, verified. | `raw_api.github.com_orgs_anthropics` |
| 15 | Company newsrooms and RSS | RSS / HTML. | Publisher copyright. Use facts, short quotes, and links. | Round announcements, compute deals, revenue statements. | Same day. | Yes. RSS works: OpenAI (648 items), Mistral (91), Google blog, NVIDIA newsroom, Meta, Microsoft, Amazon. No RSS (404): Anthropic, x.ai, CoreWeave IR path tried. Anthropic and x.ai robots.txt allow all and list a sitemap. | `raw_*rss*`, `raw_*robots*` |
| 16 | GDELT DOC 2.0 API | JSON, no key. | "unlimited and unrestricted use ... must include a citation to the GDELT Project and a link". | News article URLs, titles, dates (discovery only). | ~15 minutes. Slow (14 s for one query). | Yes. | `raw_api.gdeltproject.org_*` |
| 17 | Stanford AI Index 2026 | PDF + public Google Drive data folder. | CC BY-ND 4.0 (per OWID origin metadata). NoDerivatives: quote figures, do not remix charts. | Aggregate AI investment, adoption. | Annual (published 2026-04-13). | Yes (license read from OWID metadata, not from the PDF). | `raw_api.ourworldindata.org_v1_indicators_1227957.metadata.json` |
| 18 | Our World in Data grapher (CSV + metadata) | CSV / JSON, no key. | OWID processing under CC BY 4.0; origin license still applies (AI Index CC BY-ND; source Quid). | "Private investment in AI by region", 2013-2025, constant 2021 US$. Next update 2027-04-20. | Annual. | Yes. | `raw_ourworldindata.org_grapher_*` |
| 19 | OpenAlex | JSON. | Data CC0. Response included `"cost_usd": 0.001`, so check pricing before use. | Research works count per institution (Anthropic: 1,114 works). | Weekly. | Partly. Low priority. | `raw_api.openalex.org_*` |
| 20 | HKEXnews | HTML. | Terms not checked. | Prospectus and results for Z.ai (Knowledge Atlas) and MiniMax. | Same day. | Site reachable only. Terms not verified. | — |
| 21 | Crunchbase API | Enterprise key via sales. | No redistribution; attribution required; expunge on termination. | Everything, licensed. | — | Docs only. Site blocked by bot check. | `crunchbase-llms.txt` |
| 22 | PitchBook | Paid. | Paid terms. Direct-data page returned 403 bot check. | — | — | Public profile only. | `pitchbook-anthropic-profile.md` |
| 23 | Dealroom API | Paid, credits. | Paid terms. | Companies, funds, investors, founders, rounds, valuations. | — | Docs only. | `raw_developers.dealroom.co_` |
| 24 | CB Insights, Tracxn, Sacra, The Information | Paid. | Paid terms. | — | — | Public pages only. `tracxn.com/api` returned 404. | saved pages |

Paid sources (21-24) are **reference only**: use them to find leads and check structure. Never copy
their numbers. Cite the primary source instead.

### 3.2 What each free source is good for

**SEC is far richer for AI in 2026 than the 13F section uses.** Verified on 2026-10-20:

- **xAI is now part of SpaceX.** SpaceX (Nasdaq: SPCX, CIK 1181412) filed a 10-Q for the period
  ending 2026-06-30. It says: "On February 2, 2026 ... the Company completed its acquisition of
  X.AI Holdings Corp. ('xAI'), pursuant to which xAI became a wholly-owned subsidiary of the
  Company". SpaceX reports three segments: Space, Connectivity and AI. AI segment revenue for the
  three months ended June 30, 2026: $2,561 million. Purchases of PP&E for the six months: $28,476
  million. The xAI profile can use primary segment numbers.
- **Other AI-economy IPOs are on EDGAR.** Cerebras Systems (CBRS, S-1 filed 2026-04-17). Nscale
  (NSCALE Ltd, S-1 filed 2026-09-18 after confidential DRS filings; exhibit 10.25 mentions
  "Anthropic, PBC"). SB Energy (S-1 filed 2026-09-01, mentions OpenAI). Figma (FIG).
  MiniMax and Knowledge Atlas (Z.ai) have ADR registrations (F-6EF), so their main filings are in
  Hong Kong.
- **No public Anthropic S-1 was found.** Full-text search for "Anthropic, PBC" in S-1 forms returned
  9 hits, all filed by other companies (Nscale, SpaceX, Idea Acquisition Corp., Figma). Press
  pages talk about an Anthropic IPO, but we found no public SEC filing by Anthropic itself.
- **Counterparty 8-Ks are the best compute-deal source.** The public partner must disclose material
  contracts. Full-text search for "Anthropic" in 8-Ks since 2025 returned 156 hits (Akamai,
  TeraWulf, Broadcom, and others).
- **EDGAR entity search is noisy.** `keysTyped=anthropic` returned 79 entities; the top 10 are SPVs
  ("WU Anthropic LP", "Hiive Anthropic Series I ...", "HII Anthropic-01 ..."). We need a manual
  CIK map per company. For Anthropic, the entity search returned only SPVs. The OpenAI entity
  search failed twice with a transient error, so OpenAI is unchecked.
- **XBRL capex needs per-company rules.** (1) Tags differ: Amazon and NVIDIA use
  `PaymentsToAcquireProductiveAssets`; Alibaba used `PaymentsToAcquireOtherPropertyPlantAndEquipment`
  and stops in 2020 (it changed tags); TSMC uses `ifrs-full:PurchaseOfPropertyPlantAndEquipmentClassifiedAsInvestingActivities`
  in TWD, latest FY in the API is 2024. (2) 10-Q cash flows are year-to-date, so Q2 and Q3 must be
  derived (H1 minus Q1) and flagged. The `frames` API had 2,440 points for CY2026Q1 but only 199 for
  CY2026Q2. (3) Amazon 10-Qs include trailing-twelve-month rows that look like fiscal years.
  (4) Fiscal years differ (Microsoft June, Oracle May, NVIDIA January), and `frames` maps them to
  the nearest calendar year. (5) This tag excludes finance leases, which matter for Microsoft and
  Amazon. Label it "Cash paid for property and equipment", not "capex" alone.
- **N-PORT gives public marks of private AI shares.** Issuer names are not normalized (ARK files
  "Anthropic, Inc."; the legal name is Anthropic PBC). We need an issuer-name map.

**Epoch AI is the best seed for labs and data centers.** It separates equity from debt and gives
sources and confidence for each row. Two cautions:

- Some rows double count when summed. Epoch has a separate row "Amazon Anthropic April 2026"
  ($5B equity). Anthropic says Series H "includes $15 billion of previously committed investments
  from hyperscalers, including $5 billion from Amazon". Series G "also includes a portion of the
  previously announced investments from Microsoft and NVIDIA" (quoted in Epoch's notes).
- Many revenue rows cite paywalled press (FT, Bloomberg, The Information). Status must be
  "reported", not "announced".

**One announcement can hold five money types.** Anthropic, "Anthropic and Amazon expand
collaboration for up to 5 gigawatts of new compute", Apr 20, 2026 (verified):

| Statement (quoted) | Money type | Direction |
|---|---|---|
| "committing more than $100 billion over the next ten years to AWS technologies" | compute commitment | Anthropic to Amazon |
| "secure up to 5 gigawatts (GW) of capacity" | capacity (GW), not money | — |
| "Amazon is investing $5 billion in Anthropic today" | equity investment (later included in Series H) | Amazon to Anthropic |
| "with up to an additional $20 billion in the future" | investment commitment (not raised) | Amazon to Anthropic |
| "This builds on the $8 billion Amazon has previously invested." | cumulative stake statement (not new money) | — |
| "Our run-rate revenue has now surpassed $30 billion" | run-rate revenue (metric) | — |

A single "total" here would be wrong. This is why the data model needs typed amounts.

### 3.3 Reusable code in paracosm-studio (read-only check)

`/Users/junhoyeo/paracosm-studio/server/studio_server/`:

- `filings.py`: `EdgarClient`. Declared User-Agent, 0.12 s minimum interval (under 10 requests per
  second) shared across the process, retries on 429 and 5xx, `record_dir` snapshots with a
  `manifest.json` (URL, params, `fetched_at`), `replay_dir` for offline tests, EDGAR entity search,
  submissions plus older pages, filing index. Port this pattern (or call it from a Python build
  tool) for 8-K, Form D, N-PORT and XBRL fetches.
- `thirteenf_data.py`: 13F filing resolution, amendments, aggregation, ticker data from
  `company_tickers_exchange.json`, OpenFIGI pacing. Already reflected in `build_13f_content.py`.
- `thirteenf_sectors.py`: SIC to sector (already ported to `src/lib/capital/sectors.ts`).
- `sources.py`: SSRF-safe fetch, article extraction (trafilatura), immutable source snapshots.
  Good for saving a copy of each newsroom page we cite.
- `discovery_news.py`: GDELT DOC 2.0 client with 6 s pacing. Its docstring says "Google News RSS
  is not used: its terms allow personal feed readers only." Keep that rule.

### 3.4 Not verified

- Stanford AI Index license from the PDF itself (we used OWID's origin metadata).
- HKEXnews terms of use.
- Form ADV AUM fields through IAPD (only the firm search was probed).
- Crunchbase page layout (blocked). We used Crunchbase's own docs instead.
- OpenAlex pricing.
- Any number from paid trackers. We did not use them as data.

---

## 4. Product plan for Capital v2

### 4.1 Positioning and rules

One line: **"Where money moves in the AI economy."** Capital answers four questions:

1. Who funds the AI labs, how much, and at what valuation? (rounds, investors)
2. Where does that money go? (compute contracts, data-center leases, chips, power)
3. How much are the clouds spending to build capacity? (capex by quarter)
4. How do public markets see private AI companies? (fund marks, IPOs, 13F holdings)

Follow the money down the stack:

```
Investors (VC, growth, sovereign, corporate, public funds)
   │  equity rounds, strategic investments
   ▼
AI labs (Anthropic, OpenAI, xAI, Mistral, DeepSeek, ...)
   │  compute contracts, data-center leases
   ▼
Clouds and neoclouds (Microsoft, Amazon, Google, Oracle, CoreWeave, Nebius, ...)
   │  capex: chips, buildings, power equipment
   ▼
Chips (NVIDIA, AMD, Broadcom, TSMC)  and  data centers (TeraWulf, Crusoe, ...)
   │  power purchase agreements, leases
   ▼
Power (Vistra, Constellation, Talen, X-energy)

Public markets watch every layer: IPOs, 13F holdings, fund marks.
```

Rules (already agreed, now enforced in the model):

- Never mix money types in one total. Totals exist only within one money type and one basis.
- One event = one ID = one page. All participants link to it. No double counting.
- Every number has a source (URL, publisher, date, quote), an as-of date and a status.
- No invented numbers. Unknown is "Undisclosed" (not public) or "Not tracked yet" (we lack it). Never zero.
- Partial coverage says "among companies Tokscale tracks".
- Derived numbers (step-ups, derived quarters, per-share marks) are labeled "computed by Tokscale" and show the formula.

### 4.2 Information architecture and routes

Everything lives under `/capital` (owner decision, 2026-10-21): company and investor profiles, events,
dashboards and methodology. Model pages link a lab to its company page under `/capital/companies`, so a company
is still one shared page.

| Route | Purpose | Data | Index |
|---|---|---|---|
| `/capital` | Overview: latest events, four money-type cards, stack view, capex chart, sector hubs, public-investor teaser | all | Yes |
| `/capital/companies` | Directory with filters and saved views | orgs | Yes (base URL only) |
| `/capital/companies/[slug]` | Company profile. Template varies by category (lab, neocloud, hyperscaler, chips, power/DC) | org, events, metrics | Yes if 1+ sourced fact |
| `/capital/companies/compare?c=a,b,c` | Side-by-side (phase 2) | orgs | No |
| `/capital/investors` | Investor directory (VC, growth, sovereign, corporate, mutual fund, 13F manager) | orgs | Yes (base URL only) |
| `/capital/investors/[slug]` | Investor profile: AI rounds, leads, 13F holdings if a filer, fund marks if a registered fund | orgs, events | Yes if 1+ sourced fact |
| `/capital/investors/[slug]/13f/[quarter]` | 13F quarter page (moved from `/capital/funds/[fund]/holdings/[quarter]`) | 13F | Yes for story quarters |
| `/capital/rounds` | All financing events: equity rounds, debt facilities, IPOs, tender offers | events | Yes |
| `/capital/rounds/[slug]` | One financing event | event | Yes if a primary source |
| `/capital/compute` | Compute and infrastructure deals list, plus frontier data centers | events, Epoch | Yes |
| `/capital/deals/[slug]` | One non-financing event: compute contract, lease, chip supply, PPA, JV, acquisition | event | Yes if a primary source |
| `/capital/spending` | Capex tracker: cash paid for PP&E by quarter, fiscal labels, links to filings | XBRL | Yes |
| `/capital/data-centers` | Frontier data centers (Epoch AI, CC BY) (phase 2) | Epoch | Yes |
| `/capital/marks` | Fund marks of private AI companies (SEC N-PORT) (phase 2, owner decision) | N-PORT | Decide |
| `/capital/sectors/[slug]` | Hubs: `ai-labs`, `neoclouds`, `hyperscalers`, `semiconductors`, `power`, `data-centers` | orgs, events | Yes |
| `/capital/methodology` | Money types, statuses, coverage, corrections | — | Yes |

Old 13F routes: `/capital/portfolios` becomes `/capital/investors?type=13f`; `/capital/funds/[fund]`
becomes `/capital/investors/[fund]`. The branch is stealth and unreleased, so a move costs nothing. Add 308
redirects anyway in case links were shared.

Crunchbase concepts mapped to Tokscale v2:

| Crunchbase concept | Tokscale v2 | Difference |
|---|---|---|
| Organization profile | `/capital/companies/[slug]` | AI-economy companies only; category templates; Tokscale usage and benchmarks |
| Funding round page | `/capital/rounds/[slug]` | Typed amounts, "included in" links, quotes from the primary source |
| Acquisition page | `/capital/deals/[slug]` (type acquisition) | Same page type as compute deals |
| Investment (round × investor) | Participant row with role (lead, co-lead, investor, strategic) | Per-investor amount only when a source states it |
| Investor profile | `/capital/investors/[slug]` | Adds 13F holdings and N-PORT marks when they exist |
| Person profile | Phase 3 | Public roles only |
| Hubs | `/capital/sectors/[slug]` | Curated, small, sourced |
| Lists, saved searches, Discover | Directory filters and views in the URL | No login needed |
| Rank, heat, predictions | Not built | "Most active (90 days)" and "Usage momentum" are counts, not scores |
| Press references, news | Sources list and timeline | Primary sources first |
| (not in Crunchbase) | Compute ledger, capex tracker, fund marks, data centers | AI-specific |

Capital sub-navigation (one thin row under the main bar):
`Overview · Companies · Investors · Rounds · Compute · Spending · Public investors · Method`

### 4.3 Wireframe: `/capital` overview

Values marked ✓ were verified today. `$XX` are placeholders computed at build time.

```
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Tokscale   Leaderboards  Models  [Capital]  Tracker                                    Sign in │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ Overview  Companies  Investors  Rounds  Compute  Spending  Public investors  Method            │
│                                                                                                │
│ CAPITAL                                                                                        │
│ Where money moves in the AI economy.                              ← serif display, 56–64px     │
│ Funding rounds, compute contracts and capital spending across AI labs, clouds, chips and       │
│ power. Every number has a source, a date and a status.                                         │
│ 33 companies · 16 investors · NNN events · Reviewed Oct 20, 2026                               │
├───────────────────────┬────────────────────────┬────────────────────────┬──────────────────────┤
│ EQUITY · 2026 YTD     │ COMPUTE · 2026 YTD     │ CASH CAPEX · CY26 Q1   │ LARGEST VALUATION    │
│ $XXX.XB               │ $XXX.XB                │ $XXX.XB                │ $965B ✓              │
│ N closed rounds,      │ N contracts, stated    │ N companies, cash paid │ Anthropic, post-     │
│ AI labs tracked       │ maximum values         │ for PP&E (10-Q)        │ money, May 28, 2026  │
│ Rounds →              │ Compute →              │ Spending →             │ Rounds →             │
├───────────────────────┴────────────────────────┴────────────────────────┴──────────────────────┤
│ These four numbers are different money types. They are never added together.                   │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ LATEST EVENTS                                                              All events →        │
│ Sep 18 '26  COMPUTE  Anthropic ↔ Akamai · cloud capacity, 7-yr terms   ≈$11.6B ✓   Signed  8-K │
│ Sep 18 '26  FILING   Nscale files S-1 for a US listing                   —          Filed   S-1│
│ Sep 17 '26  FILING   Crusoe Form D · amount sold                         $3.11B ✓   Filed   D  │
│ Jul 06 '26  COMPUTE  Anthropic ↔ TeraWulf · 20-yr lease                  ≈401 MW ✓  Signed  8-K│
│ May 28 '26  ROUND    Anthropic Series H                         $65B ✓ at $965B ✓  Closed  ↗   │
│ Apr 20 '26  COMPUTE  Anthropic ↔ Amazon · up to 5 GW, >$100B over 10 yrs ✓      Announced ↗    │
├──────────────────────────────────────────────┬─────────────────────────────────────────────────┤
│ FOLLOW THE MONEY DOWN THE STACK              │ CASH PAID FOR PP&E, BY QUARTER (USD B)          │
│ Investors ──equity──▶ AI labs        (N)     │ MSFT ▁▂▃▅▆█   AMZN ▁▂▄▅▇█   GOOGL ▁▂▃▅▇█        │
│ AI labs ──contracts──▶ Clouds        (N)     │ META ▁▂▃▄▆█   ORCL ▁▁▂▄▆█   CRWV  ▁▂▃▄▆█        │
│ Clouds ──capex──▶ Chips, DCs         (N)     │ ◦ = derived quarter (YTD minus prior YTD)       │
│ DCs ──leases, PPAs──▶ Power          (N)     │ Fiscal labels kept: MSFT FY26 Q3 = Jan–Mar 2026 │
├──────────────────────────────────────────────┴─────────────────────────────────────────────────┤
│ SECTORS   AI labs (12)   Neoclouds (7)   Hyperscalers (5)   Chips (5)   Power and DCs (5)      │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ PUBLIC INVESTORS · Inside the 13F                                     All public investors →   │
│ Thiel Macro Q2 2026: eight stocks, seven energy   ·   Pershing Square Q2 2026: Netflix is back │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ How we count: money types · statuses · coverage · corrections                    Method →      │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Notes:

- The card "EQUITY · 2026 YTD" sums only closed primary equity rounds, and skips amounts linked as
  "included in" a later round. The card says so in its tooltip.
- "COMPUTE · 2026 YTD" sums stated maximum contract values with the same basis. If bases differ
  (exact vs "up to"), show count plus the largest deal instead of a sum.
- The event feed uses type labels in text (ROUND, COMPUTE, FILING, SPENDING, DEAL), not colors.
  One accent (#0073FF) for links and the active tab only.

### 4.4 Wireframe: `/capital/companies` directory

```
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Companies                                                         33 tracked · Download CSV    │
│ The companies behind AI: labs, clouds, chips, data centers and power.                          │
│ [Search name, alias, ticker…]  [Category ▾] [Country ▾] [Status ▾] [Stage ▾] [Last round ▾]    │
│ [Equity raised ▾] [Has compute deals] [Has models on Tokscale] [Backed by… ▾]       Reset      │
│ Views: (Overview)  Funding  Compute  Public market  Models          Saved view: AI labs ▾      │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ Company          Category  HQ  Status        Last financing       Post-money  Equity* Deals    │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ◼ Anthropic      AI lab    US  Private PBC   Series H · May 2026  $965B ✓     $XXXB   N        │
│ ◼ OpenAI         AI lab    US  Private PBC   …                    …           …       N        │
│ ◼ xAI            AI lab    US  SpaceX unit   Merged · Feb 2026 ✓  —           —       N        │
│ ◼ Mistral AI     AI lab    FR  Private       …                    …           …       N        │
│ ◼ DeepSeek       AI lab    CN  Private       Reported · 2026      Reported    —       —        │
│ ◼ Moonshot AI    AI lab    CN  Private       Not tracked yet      —           —       —        │
│ ◼ CoreWeave      Neocloud  US  CRWV          …                    —           —       N        │
│ ◼ Crusoe         Neocloud  US  Private       Form D · Sep 2026 ✓  Undisclosed —       N        │
│ ◼ NVIDIA         Chips     US  NVDA          —                    —           —       N        │
│ …                                                                                              │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ * Equity raised in closed primary rounds Tokscale tracks. Excludes debt, secondary sales and   │
│   commitments counted inside a later round. "—" = not tracked yet. "Undisclosed" = not public. │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

View columns:

| View | Columns |
|---|---|
| Overview | Company, category, HQ, status, last financing (type, date), last post-money (date), equity raised*, deals count, last activity |
| Funding | Rounds count, last round, last post-money, step-up vs previous (computed), lead investors of last round, Form D amount sold (if any) |
| Compute | Deals as buyer, deals as provider, largest stated contract value, contracted capacity (MW, same measure only), data centers (Epoch) |
| Public market | Ticker, latest quarter cash capex, TTM cash capex (computed), 13F holders among tracked managers |
| Models | Models in the Tokscale catalog, top model, share of tokens among Tokscale users (30 days), open-weight models on Hugging Face |

Directory UX rules:

- **Density.** 40 px rows, thin rules (`--v2-rule`), sticky header, `font-variant-numeric: tabular-nums`, right-aligned numbers, numbers in `--v2-numeric`. Monogram or vendored logo per company.
- **URL state.** Every filter, sort and view lives in the query string, so a view can be shared. Saved views are presets in code ("AI labs by valuation", "Raised in 2026", "Neoclouds by contracts", "Chinese labs", "Public AI infrastructure"). Per-user saved views can come later with GitHub sign-in.
- **Canonical.** Filtered URLs canonicalize to `/capital/companies`. Sector hubs are the indexable filtered pages.
- **Search.** Client-side fuzzy match over names, legal names and aliases ("Z.ai", "Zhipu", "Knowledge Atlas"; "Anysphere", "Cursor").
- **Empty values.** "—" with a tooltip: "Not tracked yet" or "Undisclosed by the company".
- **Mobile.** Keep Company, Category and one numeric column; the rest move into a stacked row (the existing `stack` table style).
- **Compare.** Select up to 4 rows, then "Compare" opens `/capital/companies/compare` (phase 2).
- **Trending without scores.** "Most active (90 days)" = count of events. "Usage momentum" = change in share of Tokscale tokens (labs only). No proprietary rank.

### 4.5 Wireframe: AI lab profile, `/capital/companies/anthropic`

```
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Capital / Companies / Anthropic                                                                │
│ ◼  AI LAB · PRIVATE (PBC) · SAN FRANCISCO, US                                                  │
│ Anthropic                                                          ← serif display             │
│ AI safety and research company that builds the Claude models.                                  │
│ Legal name: Anthropic PBC (as named in SEC filings)          Founded Jan 26, 2021 (Wikidata)   │
│ anthropic.com · Wikidata Q116758847 · github.com/anthropics                                    │
├───────────────────┬──────────────────────┬───────────────────────┬──────────────────┬──────────┤
│ LAST ROUND        │ POST-MONEY VALUATION │ EQUITY RAISED*        │ RUN-RATE REVENUE │ COMPUTE  │
│ Series H ✓        │ $965B ✓              │ $XXX.XB in N rounds   │ >$47B ✓          │ N deals  │
│ May 28, 2026      │ May 28, 2026         │ closed primary rounds │ May 2026         │ $ and GW │
│ Closed · Anthropic│ Announced · Anthropic│ computed · method     │ company stated   │ below    │
├───────────────────┴──────────────────────┴───────────────────────┴──────────────────┴──────────┤
│ Overview · Funding · Investors · Compute · Models & usage · Benchmarks · Metrics · Timeline ·  │
│ Public markets · Sources                                                                       │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ FUNDING                                                                     Graph | Table      │
│  post-money (log)   ●──────●──────────●───────────●──────────────●  $965B                      │
│  equity per round   ▌      ▌          █           ██             ████   ░ debt (hollow)        │
│                     2023   2024       2025                       2026                          │
│  Date        Round      Equity   Debt   Post-money  Leads                         Status  Src  │
│  May 28 '26  Series H   $65B ✓   —      $965B ✓     Altimeter, Dragoneer,          Closed  ↗   │
│                                                     Greenoaks, Sequoia ✓                       │
│              └ includes $15B committed earlier by hyperscalers, incl. Amazon $5B → counted once│
│  Feb 12 '26  Series G   $30B     —      $380B       GIC, Coatue                    Closed  ↗   │
│  …                                                                                             │
│  Why other trackers show other totals: some add debt, commitments or secondary sales. Method → │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ INVESTORS (named in primary sources)                                                           │
│  Led a round   Altimeter · Dragoneer · Greenoaks · Sequoia · GIC · Coatue · ICONIQ · Lightspeed│
│  Co-led        Capital Group · Coatue · D1 · GIC · ICONIQ · XN (Series H)                      │
│  Strategic     Amazon · Google · Microsoft · NVIDIA                → see compute ties          │
│  Per-investor amounts are shown only when a source states them (Amazon $5B, Apr 2026).         │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ COMPUTE AND INFRASTRUCTURE                                                                     │
│  Date        Counterparty      Type               Value (basis)      Capacity     Term    Src  │
│  Sep 18 '26  Akamai            Cloud capacity     ≈$11.6B ✓          —            7 yrs   8-K  │
│  Jul 06 '26  TeraWulf          Data-center lease  Not in 8-K body    ≈401 MW IT ✓ 20 yrs  8-K  │
│  Apr 20 '26  Amazon (AWS)      Compute commitment >$100B over 10 y ✓ ≤5 GW ✓      10 yrs  ↗    │
│  Apr 06 '26  Broadcom, Google  TPU capacity       No $ in 8-K        ≈3.5 GW ✓    from 2027 8-K│
│  Data centers used (Epoch AI, CC BY): Colossus 1 and 2 (SpaceX) · New Carlisle (Amazon) · …    │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ MODELS AND USAGE ON TOKSCALE                                       Among Tokscale users only   │
│  Claude Opus 5.5   203 active users · 5.3T tokens · 68% adoption   → /models/claude-opus-5-5   │
│  Claude Opus 5     133 active users · 1.9T tokens                  → /models/claude-opus-5     │
│  Lab share of tokens, 30 days: XX%                                                             │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ BENCHMARKS → top results for current models (from the benchmarks dashboard)                    │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ METRICS (company stated)                                                                       │
│  Run-rate revenue  >$30B ✓  Apr 20, 2026  "surpassed $30 billion"                         ↗    │
│  Run-rate revenue  >$47B ✓  May 2026      "crossed $47 billion earlier this month"        ↗    │
│  (press-reported values hidden, or labeled "Reported by <outlet>", per owner decision)         │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ PUBLIC MARKETS                                  Fund marks are not valuations.                 │
│  ARK Venture Fund · Jul 31, 2026 · 89,078 sh · $49.61M · ≈$556.98/sh (computed) · Level 3 ✓ ↗  │
│  IPO: no public S-1 found on EDGAR (checked Oct 20, 2026)                                      │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ TIMELINE  every event, newest first, each links to its event page                              │
│ RELATED   Same category: OpenAI · xAI · Mistral AI · DeepSeek                                  │
│           Counterparties: Amazon · Alphabet · Akamai · TeraWulf · Broadcom · SpaceX            │
│ SOURCES   N sources · last reviewed Oct 20, 2026 · Corrections: none · Report an error         │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

The usage rows above are from the local dev seed of `/models` (Claude Opus 5.5: "203 active users",
"68% adoption", "5.3T tokens"). Production numbers come from the real rollups.

### 4.6 Wireframe: funding round page, `/capital/rounds/anthropic-series-h-2026`

```
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Capital / Rounds / Anthropic Series H                                                          │
│ ROUND · EQUITY · CLOSED                                        Event ID evt_anthropic_series_h │
│ Anthropic Series H                                                                             │
│ Announced May 28, 2026 by Anthropic.                                                           │
├──────────────────────┬────────────────────────┬──────────────────────────┬─────────────────────┤
│ AMOUNT RAISED        │ POST-MONEY VALUATION   │ STEP-UP VS SERIES G      │ INCLUDED EARLIER    │
│ $65B equity ✓        │ $965B ✓                │ 2.54x (computed)         │ $15B ✓              │
│ company statement    │ company statement      │ $965B ÷ $380B            │ hyperscaler         │
│                      │                        │                          │ commitments, incl.  │
│                      │                        │                          │ Amazon $5B →        │
├──────────────────────┴────────────────────────┴──────────────────────────┴─────────────────────┤
│ PARTICIPANTS                                                                                   │
│  Role       Investor                Type        Other tracked rounds                           │
│  Lead       Altimeter Capital       Growth      …                                              │
│  Lead       Dragoneer               Growth      Series G (co-lead)                             │
│  Lead       Greenoaks               Growth      …                                              │
│  Lead       Sequoia Capital         VC          …                                              │
│  Co-lead    Capital Group · Coatue · D1 Capital Partners · GIC · ICONIQ · XN                   │
│  Strategic  Amazon ($5B, previously committed, Apr 20, 2026 event)                             │
│  Amounts per investor are not public unless a source states them.                              │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ CONTEXT                                                                                        │
│  Previous round   Series G · Feb 12, 2026 · $30B at $380B post-money                           │
│  Linked events    Amazon investment and compute deal, Apr 20, 2026 (part included here)        │
│  Company metric   Run-rate revenue "crossed $47 billion earlier this month" (same post)        │
│  On Tokscale      Anthropic share of tokens among Tokscale users in May 2026: XX%              │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ SOURCES                                                                                        │
│  1. Anthropic, "Anthropic raises $65B in Series H funding at $965B post-money valuation",      │
│     May 28, 2026. "…led by Altimeter Capital, Dragoneer, Greenoaks, and Sequoia Capital,       │
│     valuing the company at $965 billion post-money."                                     ↗     │
│  2. Epoch AI, "Data on AI companies", funding round row (CC BY 4.0), retrieved Oct 20, 2026 ↗  │
│  Corrections: none · Report an error                                                           │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 4.7 Wireframe: compute deals view, `/capital/compute`

```
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Capital / Compute                                                                              │
│ Compute and infrastructure deals                                                               │
│ Contracts for cloud capacity, data-center leases, chips and power among companies Tokscale     │
│ tracks. Contract values are stated maximums over multi-year terms. They are not annual spend.  │
│ [Buyer ▾] [Provider type ▾] [Deal type ▾] [Status ▾] [Year ▾] [Has $ value] [Has MW/GW]        │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ Date        Buyer      Provider          Type            Value (basis)        Capacity   Term  │
│ Sep 18 '26  Anthropic  Akamai            Cloud capacity  ≈$11.6B aggregate ✓  —          7 yrs │
│ Jul 06 '26  Anthropic  TeraWulf          DC lease        Not in 8-K body      ≈401 MW IT 20 yrs│
│ Apr 20 '26  Anthropic  Amazon (AWS)      Compute commit. >$100B ✓             ≤5 GW ✓    10 yrs│
│ Apr 06 '26  Anthropic  Broadcom, Google  TPU capacity    No $ in 8-K          ≈3.5 GW ✓  2027– │
│ Sep 23 '25  OpenAI     CoreWeave         Cloud capacity  ≤$6.5B ✓             —     to May 2031│
│ Sep 08 '25  Microsoft  Nebius            GPU capacity    ≈$17.4B ✓ (≈$19.4B   —     5 yrs, 2031│
│                                                          with options ✓)                       │
│ …                                                                    Source column: 8-K ↗ etc. │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ BY BUYER (count, largest stated value)  ·  BY PROVIDER  ·  CAPACITY (sum only within one       │
│ measure: critical IT MW vs gross MW vs GW of chips are kept apart)                             │
├────────────────────────────────────────────────────────────────────────────────────────────────┤
│ FRONTIER DATA CENTERS · Epoch AI (CC BY 4.0) · 95 sites · updated Oct 20, 2026                 │
│ Name                           Owner      Users (Epoch tags)              Power MW  Est. cost  │
│ Anthropic-Amazon New Carlisle  Amazon     Anthropic #confident            1,037     $39.3B est.│
│ Colossus 2                     SpaceXAI   Anthropic, Cursor, SpaceXAI     946       $35.8B est.│
│ Microsoft Fairwater Atlanta    Microsoft  OpenAI #likely, Microsoft       636       $24.1B est.│
│ Meta Prometheus                Meta       Meta #confident                 471       $17.8B est.│
│ OpenAI Stargate Abilene        Oracle     OpenAI #confident               421       $15.9B est.│
│ Est. cost = Epoch's estimate in 2025 US$, status "estimated". Not a reported spend.            │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Data-center rows above are copied from Epoch's `data_centers.csv` (retrieved 2026-10-20).

### 4.8 Profile anatomy by company type

All profiles share: header facts, sub-nav, timeline, related, sources and corrections. The middle
sections change by category.

| Type | Header facts | Core sections | Main free sources |
|---|---|---|---|
| AI lab (Anthropic, OpenAI, xAI, Mistral, DeepSeek, Moonshot, Z.ai, MiniMax) | Status, founded, HQ, last round, last post-money, equity raised*, run-rate revenue (company stated), compute deals | Funding, investors, compute and infrastructure, models and usage on Tokscale, benchmarks, metrics, public markets (fund marks, IPO), timeline | Newsroom, Epoch, counterparty 8-Ks, N-PORT, Wikidata, Hugging Face, Tokscale |
| Neocloud (CoreWeave, Nebius, Lambda, Crusoe, Nscale, Fluidstack) | Ticker or private, latest quarter cash capex, largest disclosed customer contract, debt facilities, data centers | Customers and contracts (as provider), debt and equity financing, capex by quarter, data centers (Epoch), public investors (13F) | 8-K/6-K/S-1, XBRL, Form D, N-PORT, Epoch |
| Hyperscaler (Microsoft, Amazon, Alphabet, Meta, Oracle) | Ticker, latest quarter cash capex, fiscal-year cash capex, capex guidance (labeled), AI lab stakes | Capex by quarter, compute deals as provider and as buyer, stakes in AI labs, data centers (Epoch), models (if a lab), public investors (13F) | XBRL, 8-K exhibits 99.1, Epoch, newsrooms |
| Chips (NVIDIA, AMD, Broadcom, TSMC, Cerebras) | Ticker, latest revenue (reported), capex, strategic investments | Supply agreements, strategic investments in labs, chip sales estimates (Epoch, "estimated"), customers named in filings, public investors (13F) | XBRL, 8-K, S-1 (Cerebras), Epoch chip sales |
| Power and data centers (TeraWulf, Vistra, Constellation, Talen, X-energy) | Ticker, contracted capacity (MW), counterparties | Leases and PPAs with AI buyers, capex, public investors (13F: Thiel Macro held VST, AEP and others) | 8-K, XBRL, 13F |

### 4.9 Investor profiles and where 13F fits

- `/capital/investors` lists all investors with type filters: VC, growth, sovereign, corporate,
  mutual fund (N-PORT), public fund manager (13F).
- An investor profile shows: AI rounds tracked (as lead / co-lead / participant), first and latest
  participation, portfolio companies (logos), strategic ties (compute deals for corporates), and
  public data where it exists: 13F holdings (if a 13F filer), fund marks (if an N-PORT filer),
  regulatory identity (IAPD).
- **13F is demoted, not deleted.** Thiel Macro and Pershing Square become investor profiles of type
  "public fund manager". Their quarter pages and "Inside the 13F" stories stay, under
  `/capital/investors/[slug]/13f/[quarter]`. Company profiles keep a small "Public investors" section that
  reuses today's positions table.
- Scope notes carry over ("It is not Founders Fund's portfolio and not Peter Thiel's total
  investments"). Founders Fund gets its own investor profile, linked as a separate organization.

### 4.10 Data model (typed JSON first, database later)

Keep today's pattern: JSON in `src/content/capital/`, zod schemas, cross-checks at module load,
tests that fail the build. Move to Postgres (drizzle) only when an editor UI exists.

Files:

```
src/content/capital/
  orgs.json              companies and investors (one table; an org can be both)
  people.json            phase 3
  sources.json           every cited document
  events/<slug>.json     one file per event (round, deal, acquisition, IPO, filing)
  metrics/capex.json     generated from SEC XBRL (build tool)
  metrics/marks.json     generated from SEC N-PORT (build tool, phase 2)
  datasets/epoch/*.csv   pinned copies of Epoch files with retrieval date (CC BY)
  filings/*.json         existing 13F content (unchanged)
  tools/                 build_13f_content.py, build_capex.py, import_epoch.py, build_marks.py
```

Core types (sketch):

```ts
type OrgKind = "company" | "investor";
type Category = "ai-lab" | "neocloud" | "hyperscaler" | "semiconductors" | "data-centers" | "power" | "ai-apps";
type InvestorType = "vc" | "growth" | "sovereign" | "corporate" | "mutual-fund" | "public-fund-manager";

interface Org {
  id: string;                 // "org_anthropic" (never reused)
  slug: string;               // "anthropic"
  name: string; legalNames: string[]; aliases: string[];
  kinds: OrgKind[]; categories: Category[]; investorTypes?: InvestorType[];
  status: "private" | "public" | "subsidiary" | "acquired" | "closed";
  parent?: { org: string; since: string; sourceId: string };   // xAI -> SpaceX, 2026-02-02
  hq?: { city: string; country: string };
  founded?: { date: string; sourceId: string };
  ids: { wikidata?: string; secCik?: string[]; tickers?: string[]; cusip6?: string; hfOrg?: string; githubOrg?: string; crunchbase?: string /* link only, from Wikidata P2088 */ };
  labSlugs?: string[];        // Tokscale model labs ("google" -> org_alphabet)
  nportIssuerNames?: string[]; // "Anthropic, Inc." as filed by ARK
}

type Measure =
  | "equity_raised" | "debt_raised" | "secondary_sale" | "ipo_proceeds"
  | "valuation_post" | "valuation_pre"
  | "equity_investment" | "investment_commitment" | "cumulative_investment"
  | "contract_value" | "compute_commitment" | "purchase_price"
  | "capex_actual" | "capex_guidance"
  | "revenue_reported" | "revenue_run_rate"
  | "form_d_amount_sold" | "fund_mark" | "reported_13f_value" | "estimated_capital_cost";

type Basis = "exact" | "approximately" | "up_to" | "more_than" | "at_least" | "range";
type Status = "announced" | "signed" | "closed" | "filed" | "reported" | "estimated" | "undisclosed";   // per amount
type EventStatus = "announced" | "signed" | "closed" | "filed" | "completed" | "terminated";                // per event

interface Amount {
  id: string; measure: Measure; value: number | null;  // null only with status "undisclosed"
  currency: string;           // ISO 4217; original currency kept, no silent FX
  basis: Basis; status: Status; asOf: string;
  from?: string; to?: string; // org ids, for money that moves between parties
  term?: { years?: number; through?: string };
  condition?: string;         // "if Microsoft acquires additional services" (Nebius: ~$17.4B base, ~$19.4B with options)
  includedIn?: string;        // event id that already counts this amount
  sourceId: string; quote: string;
}

interface Quantity { measure: "it_load_mw" | "gross_power_mw" | "capacity_gw" | "gpus" | "h100e"; value: number; basis: Basis; sourceId: string; quote: string }

interface Event {
  id: string; slug: string;
  kind: "financing" | "deal" | "acquisition" | "ipo" | "filing";
  types: string[];            // "equity_round", "compute_contract", "datacenter_lease", "chip_supply", "ppa", "strategic_investment", "form_d", "13f_holdings_report", "nport_report"
  title: string; roundLabel?: string;
  dates: { announced?: string; signed?: string; closed?: string; filed?: string };
  status: EventStatus;
  participants: { org: string; roles: ("issuer" | "lead_investor" | "co_lead" | "investor" | "buyer" | "provider" | "lessor" | "lessee" | "acquirer" | "target" | "lender" | "borrower" | "filer")[] }[];
  amounts: Amount[]; quantities: Quantity[];
  relations: { type: "includes" | "part_of" | "amends" | "supersedes" | "follows" | "evidence_for"; event: string }[];
  sourceIds: string[]; reviewedAt: string; corrections: { date: string; text: string }[];
}

interface Source {
  id: string; url: string; publisher: string;
  type: "company_announcement" | "sec_filing" | "exchange_filing" | "dataset" | "news" | "regulator";
  title: string; publishedAt?: string; retrievedAt: string;
  filing?: { form: string; accession: string; cik: string };
  license?: "public-domain-sec" | "CC-BY-4.0" | "CC0" | "publisher";
  snapshotPath?: string;      // saved copy, like paracosm-studio sources.py
}
```

Validation rules (build fails if any rule breaks):

1. Every `Amount` and `Quantity` has a `sourceId` that exists and a non-empty `quote`.
2. `value === null` only when `status === "undisclosed"`. Zero is never used for unknown.
3. `total(amounts, measure)` accepts one measure, one currency and one basis family. It skips any
   amount with `includedIn`. It throws on mixed input. Non-summable measures (valuations, marks,
   cumulative statements, run-rate) throw always.
4. Event participants reference existing orgs. Each org appears once per event and may hold several roles (for example buyer and issuer).
5. Status "rumored" / "in talks" does not exist, so it cannot be published.
6. Every org page needs at least one sourced fact to be indexable.
7. Event IDs and slugs are unique and stable. A rename keeps the ID and adds a redirect.

Example event (abridged), the Apr 20, 2026 Amazon announcement:

```json
{
  "id": "evt_anthropic_amazon_2026_04_20",
  "slug": "anthropic-amazon-compute-and-investment-2026-04",
  "kind": "deal",
  "types": ["compute_commitment", "strategic_investment"],
  "title": "Anthropic commits more than $100B to AWS over ten years; Amazon invests $5B",
  "dates": { "announced": "2026-04-20" },
  "status": "announced",
  "participants": [
    { "org": "org_anthropic", "roles": ["buyer", "issuer"] },
    { "org": "org_amazon", "roles": ["provider", "investor"] }
  ],
  "amounts": [
    { "id": "a1", "measure": "compute_commitment", "value": 100000000000, "currency": "USD", "basis": "more_than",
      "status": "announced", "asOf": "2026-04-20", "from": "org_anthropic", "to": "org_amazon", "term": { "years": 10 },
      "sourceId": "src_anthropic_amazon_2026_04_20",
      "quote": "We are committing more than $100 billion over the next ten years to AWS technologies" },
    { "id": "a2", "measure": "equity_investment", "value": 5000000000, "currency": "USD", "basis": "exact",
      "status": "announced", "asOf": "2026-04-20", "from": "org_amazon", "to": "org_anthropic",
      "includedIn": "evt_anthropic_series_h_2026",
      "sourceId": "src_anthropic_amazon_2026_04_20", "quote": "Amazon is investing $5 billion in Anthropic today" },
    { "id": "a3", "measure": "investment_commitment", "value": 20000000000, "currency": "USD", "basis": "up_to",
      "status": "announced", "asOf": "2026-04-20", "from": "org_amazon", "to": "org_anthropic",
      "sourceId": "src_anthropic_amazon_2026_04_20", "quote": "with up to an additional $20 billion in the future" }
  ],
  "quantities": [
    { "measure": "capacity_gw", "value": 5, "basis": "up_to", "sourceId": "src_anthropic_amazon_2026_04_20",
      "quote": "secure up to 5 gigawatts (GW) of capacity" }
  ],
  "relations": [],
  "sourceIds": ["src_anthropic_amazon_2026_04_20"],
  "reviewedAt": "2026-10-20",
  "corrections": []
}
```

The event stays a separate deal. The amount-level `includedIn` on `a2` is what stops the $5B from
being counted twice when Series H is summed.

How 13F (and other filings) map into the model:

| Filing | Event | Facts | Shown on |
|---|---|---|---|
| 13F-HR / 13F-NT | `kind: "filing"`, `types: ["13f_holdings_report"]`, participant = manager (`filer`) | Positions per issuer: shares, value (`reported_13f_value`), weight (computed) | Investor profile (quarters), company profile ("Public investors"), stories |
| Form D | `types: ["form_d"]`, participant = issuer | `form_d_amount_sold`, total offering amount, investor count, first sale date | Company profile (Funding, as "SEC Form D"); `evidence_for` link to the round when known |
| N-PORT | `types: ["nport_report"]`, participant = fund | `fund_mark` per issuer, shares, fair-value level | Company "Public markets", investor profile, `/capital/marks` |
| 8-K / 6-K | `kind: "deal"` or `"financing"`, both parties as participants | Contract value, capacity, term, warrants | Both company profiles, `/capital/compute` |
| 10-Q / 10-K XBRL | No event; a metric series | `capex_actual` per period, with accession and form | Company "Spending", `/capital/spending` |

### 4.11 Seed list for v1

Status and evidence below were checked today unless marked "to check". ✓ = probed today.

**Companies (33)**

| # | Company | Category | Status | Tokscale lab | Free / primary sources | Fields we can fill | Gaps (honest) |
|---|---|---|---|---|---|---|---|
| 1 | Anthropic | AI lab | Private, Anthropic PBC | anthropic | Newsroom ✓ (no RSS; sitemap ✓), Epoch ✓ (15 rounds), counterparty 8-Ks ✓ (Akamai, TeraWulf, Broadcom), Nscale S-1 exhibit ✓, N-PORT ✓, Wikidata ✓, GitHub ✓ | Rounds, leads, post-money, run-rate revenue (company), compute deals ($, MW, GW, term), fund marks, basics | Audited financials (no public S-1 found), per-investor amounts, cap table |
| 2 | OpenAI | AI lab | Private, OpenAI Group PBC (N-PORT name) | openai | RSS ✓ (648 items), Epoch ✓ (17 rounds), CoreWeave 8-K ✓, Cerebras S-1 exhibit ✓ (hit), SB Energy S-1 ✓ (hit), N-PORT ✓, Wikidata ✓ | Same as above | Same as above |
| 3 | xAI | AI lab | Subsidiary of SpaceX since 2026-02-02 ✓ | xai | SpaceX 10-Q ✓, Epoch ✓ (5 rounds before the merger), Epoch data centers ✓ (Colossus 1 and 2), x.ai sitemap ✓ | Merger event, AI segment revenue, data-center capacity, pre-merger rounds | Standalone capex (segment capex needs the filing table) |
| 4 | SpaceX | Parent of xAI | Public, Nasdaq SPCX ✓ | — | 10-Q ✓, 8-K ✓, S-1 ✓ (IPO June 2026) | AI segment revenue, cash capex, IPO, debt | Segment capex |
| 5 | Alphabet (Google, Google DeepMind) | Hyperscaler, AI lab | Public GOOGL | google | XBRL ✓ (FY2025 cash capex $91.447B; Q1 2026 $35.674B), Broadcom 8-K ✓, Epoch data centers ✓, Hugging Face ✓ | Capex, TPU deals, stakes in labs, models | DeepMind-only spend; primary source for the April 2026 Google investment (to check) |
| 6 | Meta | Hyperscaler, AI lab | Public META | meta | XBRL ✓ (FY2025 $69.691B; Q1 2026 $18.997B), Epoch data centers ✓, Hugging Face ✓, RSS ✓ | Capex, data centers, open models | Lab-only spend |
| 7 | Mistral AI | AI lab | Private, France | mistral | RSS ✓ (91 items), Epoch ✓ (6 rounds), Wikidata ✓, Hugging Face ✓ (81 models) | Rounds, leads, valuation, models | EUR amounts (keep currency); revenue mostly press |
| 8 | DeepSeek | AI lab | Private, China (parent High-Flyer, Wikidata ✓) | deepseek | Hugging Face ✓ (106 models, downloads), Epoch ✓ (1 round, confidence "Likely", WSJ) | Open-weight releases, downloads | Funding is press-reported only |
| 9 | Moonshot AI | AI lab | Private, China | moonshot-ai | Hugging Face ✓ (19 models), Wikidata ✓, Epoch revenue ✓ (3 rows, press) | Models, basics | No Epoch funding rows; few primary sources |
| 10 | Z.ai (Zhipu, Knowledge Atlas Technology) | AI lab | Listed in Hong Kong (Epoch cites 2513.HK); ADR registered with SEC ✓ | zai | HKEX (terms to check), Epoch ✓ (IPO row), Hugging Face ✓ (154 models) | IPO, results (HK filings), models | HK filing parsing; CNY/HKD |
| 11 | MiniMax | AI lab | Listed in Hong Kong (Epoch cites 0100.HK); ADR registered ✓ | minimax | HKEX (terms to check), Epoch ✓ (IPO row), Hugging Face ✓ | IPO, results, models | Same |
| 12 | Alibaba (Qwen) | AI lab, cloud | Public NYSE BABA (20-F) | alibaba | XBRL ✓ (capex tag changed after 2020, needs mapping), Hugging Face ✓ (469 Qwen models) | Capex after mapping, models | Tag mapping; CNY |
| 13 | Cursor (Anysphere) | AI app and lab | Private | cursor | Company blog (to check); EDGAR shows SPVs only ✓ | Rounds from the blog | Everything still to check |
| 14 | CoreWeave | Neocloud | Public CRWV | — | XBRL ✓ (FY2025 $10.309B; Q1 2026 $7.695B), 8-Ks ✓ (OpenAI up to ≈$6.5B; debt facilities), Epoch data centers ✓ | Capex, contracts, debt | Per-customer backlog |
| 15 | Nebius | Neocloud | Public NBIS (20-F/6-K, US GAAP XBRL ✓) | — | XBRL ✓ (FY2025 $4.066B), 6-K ✓ (Microsoft: "about $17.4 billion through 2031", up to "about $19.4 billion" with added services, five-year term) | Capex, contracts | — |
| 16 | Lambda | Neocloud | Private (Lambda, Inc.) | — | Form D ✓ (2022, 2024), N-PORT ✓ (about $38.00/sh) | Form D amounts, fund marks | Round sizes, contracts |
| 17 | Crusoe | Neocloud, DC developer | Private (Crusoe Inc.) | — | Form D ✓ (sold $3,108,543,889, 108 investors), N-PORT ✓ (about $226.20/sh) | Form D, marks | Round label, valuation |
| 18 | Nscale | Neocloud | Private, S-1 filed 2026-09-18 ✓ | — | S-1 ✓ (Anthropic exhibit) | Financials and contracts from the S-1 | IPO outcome |
| 19 | Fluidstack | Neocloud | Private, UK (Form D 2026 ✓) | — | Form D ✓ | Form D | Large lab deals are press-only so far |
| 20 | Akamai | Cloud (edge) | Public AKAM | — | 8-K ✓ (Anthropic ≈$11.6B; warrant up to 387,051 preferred shares) | Contract, warrant | — |
| 21 | Oracle | Hyperscaler | Public ORCL | — | XBRL ✓ (FY2026 ending May 2026: $55.663B), Epoch ✓ (owner of Stargate Abilene) | Capex, data centers | RPO tag (to check) |
| 22 | Microsoft | Hyperscaler | Public MSFT | — | XBRL ✓ (FY2026 ending June 2026: $115.948B; Jan–Mar 2026 $30.876B) | Capex, OpenAI ties | Finance leases outside this tag |
| 23 | Amazon | Hyperscaler | Public AMZN | — | XBRL ✓ (FY2025 $131.819B via `PaymentsToAcquireProductiveAssets`; Q1 2026 $44.203B), Anthropic newsroom ✓ | Capex, Anthropic stake events | TTM rows in 10-Qs (filter) |
| 24 | NVIDIA | Chips | Public NVDA | nvidia | XBRL ✓, Epoch chip sales ✓, Hugging Face ✓ (1,004 models) | Chip sales estimates (Epoch), strategic investments | Per-customer sales |
| 25 | AMD | Chips | Public AMD | — | XBRL ✓ | Capex, deals (to check) | — |
| 26 | Broadcom | Chips | Public AVGO | — | 8-K ✓ (Google TPU agreement; Anthropic ≈3.5 GW) | Deals | — |
| 27 | TSMC | Foundry | Public TSM (20-F, IFRS, TWD) | — | XBRL ✓ (latest FY in API: 2024), 6-K monthly revenue (to check) | Capex (TWD) | 2025 XBRL gap; IFRS |
| 28 | Cerebras | Chips | Public CBRS (S-1 2026-04-17 ✓) | — | S-1 ✓ (OpenAI exhibit), 10-Q ✓ | Contracts, financials | — |
| 29 | TeraWulf | Data centers, power | Public WULF | — | 8-K ✓ (Anthropic ≈401 MW critical IT load, 20 years, delivery late 2027 to early 2028; the same 8-K also reports a separate ≈$530M sale of an interest to Fluidstack) | Lease, capacity, a second event | Lease value not in the 8-K body (check exhibit 99.1) |
| 30 | Vistra | Power | Public VST (in 13F set) | — | XBRL, 8-K (to check), 13F ✓ | 13F holders, PPAs (to check) | — |
| 31 | Constellation Energy | Power | Public CEG | — | XBRL, 8-K (to check) | PPAs (to check) | — |
| 32 | Talen Energy | Power | Public TLN | — | 8-K (to check) | PPA with Amazon (to check) | — |
| 33 | X-energy | Nuclear | Public XE (in 13F set) | — | SEC filings, 13F ✓ | 13F holders | — |

**Investors (16) plus existing 13F managers**

| # | Investor | Type | Free / primary sources | Fields we can fill | Gaps |
|---|---|---|---|---|---|
| 1 | Amazon | Corporate | Anthropic newsroom ✓ | Stake events, commitments, compute ties | Stake value |
| 2 | Alphabet (Google) | Corporate | Epoch row (press source); Broadcom 8-K ✓ | Stake events, TPU ties | Primary source for the 2026 investment (to check) |
| 3 | Microsoft | Corporate | Epoch notes quoting Anthropic ✓ (Series G "includes a portion of the previously announced investments from Microsoft and NVIDIA"); OpenAI posts (to check); Nebius 6-K ✓ (as a buyer) | Stake events, compute deals | — |
| 4 | NVIDIA | Corporate | Epoch notes quoting Anthropic ✓ (Series G); OpenAI posts (to check) | Stake events | — |
| 5 | SoftBank Group | Corporate | OpenAI posts ✓ (co-led a 2026 round); SB Energy S-1 ✓ (hit) | Rounds, energy ties | Japanese filings not used |
| 6 | Sequoia Capital | VC | Anthropic Series H ✓ (lead) | Participations, leads | Fund sizes (Form ADV, phase 3) |
| 7 | Andreessen Horowitz | VC | IAPD ✓ (SEC 801-114985); OpenAI post ✓ (co-led) | Participations, identity | Same |
| 8 | Lightspeed Venture Partners | VC | Epoch quote ✓ (Series E lead) | Participations | — |
| 9 | ICONIQ | Growth | Epoch quote ✓ (Series F lead); Series H co-lead ✓ | Participations | — |
| 10 | Coatue | Growth, 13F filer (to check) | Series G lead and Series H co-lead ✓ | Participations, 13F holdings | — |
| 11 | Thrive Capital | Growth | OpenAI posts (to check) | Participations | — |
| 12 | Founders Fund | VC | Series G co-lead ✓ (Epoch quote) | Participations | Kept separate from Thiel Macro |
| 13 | GIC | Sovereign | Series G lead ✓; Series H co-lead ✓ | Participations | — |
| 14 | MGX | Sovereign (Abu Dhabi) | Series G co-lead ✓ | Participations | — |
| 15 | Fidelity | Mutual fund manager | N-PORT ✓ (full-text hits in Fidelity Select Portfolios) | Fund marks | Series mapping |
| 16 | ARK Venture Fund | Interval fund | N-PORT ✓ | Level 3 marks for Anthropic, OpenAI, Lambda, Crusoe, Databricks, Figure AI, Perplexity; SpaceX as a listed (Level 1) holding | — |
| — | Thiel Macro, Pershing Square | Public fund manager (13F) | Existing pipeline ✓ | 13F quarters, stories | TCI, Atreides stay "in review" |

**Field coverage summary**

| Field | Best free source | Coverage for the seed list |
|---|---|---|
| Founded, HQ, founders, website, IDs | Wikidata (CC0) | Good for US/EU labs; partial for Chinese labs |
| Legal name, CIK, SIC, state | SEC submissions | US filers and Form D filers |
| Funding rounds (amount, post-money, leads) | Company newsroom, then Epoch (CC BY) | Good for the top US/EU labs; weak for Chinese labs and neoclouds |
| Per-investor amounts | Rarely disclosed | Mostly "Undisclosed" |
| Form D amount sold, investor count | SEC Form D | US issuers that file (Crusoe, Lambda, Fluidstack, Databricks, Perplexity). Not Anthropic (only SPVs seen); OpenAI unchecked |
| Compute contracts | Public counterparty's 8-K / 6-K / S-1; newsrooms | Good when one side files with the SEC |
| Cash capex | SEC XBRL | All public companies, after per-company tag mapping |
| Revenue | Company statements (run-rate), filings (public companies), Epoch (often press) | Uneven |
| Fund marks | SEC N-PORT | Labs held by registered funds |
| Data centers | Epoch (CC BY) | 95 frontier sites |
| Chip volumes | Epoch chip sales (CC BY, estimates) | Major chips |
| Models and usage | Hugging Face API, Tokscale | All Tokscale labs |

### 4.12 SEO and structured data

Target queries and pages:

| Query pattern | Page | Title pattern |
|---|---|---|
| "Anthropic funding", "Anthropic valuation", "who invested in Anthropic" | `/capital/companies/anthropic` | "Anthropic: Funding Rounds, Valuation, Investors and Compute Deals \| Tokscale" |
| "Anthropic Series H" | `/capital/rounds/anthropic-series-h-2026` | "Anthropic Series H: $65B at $965B Post-Money (May 2026) \| Tokscale" (numbers only when sourced and dated) |
| "CoreWeave contracts", "CoreWeave customers" | `/capital/companies/coreweave#compute` | "CoreWeave: Customer Contracts, Capex and Financing \| Tokscale" |
| "hyperscaler capex 2026", "Microsoft capex" | `/capital/spending` | "AI Capex Tracker: Cash Spending by Microsoft, Amazon, Alphabet, Meta and Oracle \| Tokscale" |
| "AI data centers list" | `/capital/data-centers` | "Frontier AI Data Centers: Owners, Users and Power \| Tokscale" |
| "Sequoia AI investments" | `/capital/investors/sequoia-capital` | "Sequoia Capital: AI Investments Tokscale Tracks \| Tokscale" |
| "AI labs funding" | `/capital/sectors/ai-labs` | "AI Labs: Funding, Valuations and Compute \| Tokscale" |

Structured data (JSON-LD):

- Company pages: `Organization` (or `Corporation` with `tickerSymbol` for public companies), `legalName`, `url`, `foundingDate`, `founder` (`Person`), `address`, `sameAs` (Wikidata URL, official site), `identifier` (`PropertyValue` for SEC CIK), `parentOrganization` (xAI to SpaceX). Plus `BreadcrumbList`.
- Investor pages: `Organization` with `sameAs` and `identifier` (SEC IAPD number when present).
- Event pages: `WebPage` with `about` (the issuer) and `mentions` (participants), plus `BreadcrumbList`. There is no good schema.org type for a funding round; do not fake one.
- Dataset pages (`/capital/spending`, `/capital/data-centers`): `Dataset` with `creator`, `license`, `isBasedOn` (SEC; Epoch AI with CC BY), `temporalCoverage`, `distribution` (CSV).
- Directory pages: `ItemList`.
- Editorial stories: `NewsArticle` (already used for 13F stories).

Do not index:

- Filter, sort and view permutations (canonical to the base list). Search results. Compare pages.
- Events and profiles with no primary source, and anything "in review".
- Org pages with no sourced fact (thin-content gate, like the model-page rule of five accounts).
- Raw per-fund, per-month N-PORT pages (fold them into company and investor pages).
- OG image routes and CSV downloads.

Sitemap: reuse `capitalSitemapEntries()` rules. `lastModified` = latest `reviewedAt` or correction
date, never the request time.

---

## 5. Phased plan for the stealth branch

Sizes: S = up to 1 day, M = 2-3 days, L = about 1 week (one engineer plus one editor).

### Phase 0: data foundation (no visible UI change)

| ID | Task | Size | Output |
|---|---|---|---|
| P0.1 | Schema and ledger: `Org`, `Event`, `Amount`, `Quantity`, `Source`, metric series (zod). `total()` guard that throws on mixed measures, currencies or bases and skips `includedIn` amounts. | M | `src/lib/capital/v2/*`, tests |
| P0.2 | `orgs.json` for 33 companies and 16 investors. IDs: Wikidata QID, SEC CIK, tickers, Hugging Face org, GitHub org, Tokscale `labSlugs`, N-PORT issuer names. | M | content |
| P0.3 | Hand-written events from verified primary sources (start with ~20): Anthropic Series E/F/G/H, Amazon (Apr 2026), Akamai, TeraWulf, Broadcom/Google; OpenAI rounds (newsroom), CoreWeave/OpenAI; xAI merger into SpaceX; Mistral (Sep 2026); Crusoe Form D; Lambda Form D; Nscale S-1; Nebius/Microsoft 6-K. Each with quotes. | L | `events/*.json`, `sources.json` |
| P0.4 | `build_capex.py`: SEC XBRL for every public company in the seed list (about 20; start with Microsoft, Amazon, Alphabet, Meta, Oracle, CoreWeave, Nebius, SpaceX). Per-company concept map. Fiscal labels. Derived quarters flagged. Accession links. Record/replay snapshots (paracosm-studio `EdgarClient` pattern). | M | `metrics/capex.json` |
| P0.5 | `import_epoch.py`: pin Epoch ZIPs with retrieval date; write candidate events to an inbox, not to the site. Map Epoch fields: equity → `equity_raised`, debt → `debt_raised`, post-money → `valuation_post`, Confidence and Source carried. | S | `datasets/epoch/`, `inbox/` |
| P0.6 | Replace `companySlugForLab()` aliases with `orgs.json` `labSlugs`. Every Tokscale lab with an org gets a profile. | S | `src/lib/companyLinks.ts` |
| P0.7 | Tests: ledger guard, every amount sourced, ID uniqueness, no "rumored" status, sitemap gating. | S | tests |

### Phase 1: stealth v1 UI

| ID | Task | Size |
|---|---|---|
| P1.1 | `/capital` overview rewrite (section 4.3). Title: "Capital: Funding, Compute Deals and AI Spending \| Tokscale". | M |
| P1.2 | `/capital/companies` directory with filters, views, URL state, CSV download (section 4.4). | L |
| P1.3 | `/capital/companies/[slug]` v2 template with category variants (sections 4.5 and 4.8). Reuse the 13F positions table as "Public investors". | L |
| P1.4 | `/capital/rounds` and `/capital/rounds/[slug]` (section 4.6). | M |
| P1.5 | `/capital/compute` and `/capital/deals/[slug]`, with the Epoch data-center block and CC BY attribution (section 4.7). | M |
| P1.6 | `/capital/spending` with small multiples and fiscal labels. | M |
| P1.7 | `/capital/investors` and `/capital/investors/[slug]`. Move 13F quarter pages to `/capital/investors/[slug]/13f/[quarter]`. Add 308 redirects. | M |
| P1.8 | Methodology page (money types, statuses, coverage, corrections). Home capital preview fed by events. Sitemap entries. JSON-LD. OG images for round and deal pages (reuse the quarter `og` route pattern). | M |

Definition of done for Phase 1:

- Every number on every page traces to `sources.json` with a quote.
- `bun run test` (vitest), `bun run typecheck`, `bun run lint` and `bun run build` pass in `packages/frontend` on the branch tip.
- Visual QA on the dev server at `http://localhost:3300`: desktop and phone screenshots of each new route.

### Phase 2

- Fund marks from N-PORT (`/capital/marks` and profile blocks), if the owner says yes.
- `/capital/data-centers`: the full Epoch list (95 sites), with owner and user links.
- Sector hubs and the compare page.
- Filing radar (internal only): daily SEC full-text search for tracked names, SEC submissions for
  tracked CIKs, company RSS and sitemaps (Anthropic and x.ai have no RSS), GDELT. Output: `inbox/*.json`
  for an editor. Nothing auto-publishes.
- Tokscale usage share and benchmark links on lab pages.

### Phase 3

- People (founders and executives) from Wikidata and company pages. Public roles only.
- Acquisitions as a first-class list.
- Non-US primary sources: HKEXnews (after a terms check), UK Companies House, French registries.
- Corrections UI and a public changelog. CSV and JSON downloads with license notes. Alerts.
- Move to database tables if editing volume grows.

---

## 6. Risks and how to handle them

| Risk | Example seen today | Mitigation |
|---|---|---|
| Mixed money types | CB Insights "Total Raised $203.455B" includes debt; one Anthropic post holds five money types | Closed `Measure` list; `total()` throws on mixed input; labels on every total |
| Double counting | Series H includes $15B of earlier hyperscaler commitments (incl. Amazon $5B), and Epoch also has the Amazon $5B as its own row | `includedIn` and `part_of` relations; one event ID; tests |
| Press vs primary | Many Epoch revenue rows cite paywalled press; DeepSeek round is "Likely" via WSJ | Primary first; status "reported" with the outlet named; reported values never enter totals |
| "Up to" and multi-year values | CoreWeave/OpenAI "up to approximately $6.5 billion through May 31, 2031" | `basis` and `term` fields; no annualizing; no sums across bases |
| Fiscal periods and derived quarters | Microsoft FY ends June, Oracle May; Q2 frames for CY2026 have 199 points vs 2,440 for Q1 | Keep fiscal labels; derive Q2/Q3 from YTD and flag "derived" |
| XBRL tag drift | Alibaba tag stops in 2020; TSMC latest FY in the API is 2024; Amazon uses a different tag | Per-company concept map; a test that the latest filing has the mapped tag |
| Name and entity collisions | "Anthropic, Inc." in ARK's N-PORT vs Anthropic PBC; 79 EDGAR entities for "anthropic", mostly SPVs; Vistra (VST) vs Vista Energy (VIST) | Manual alias and CIK maps in `orgs.json`; reviewer sign-off |
| Fund marks misread as valuations | ARK's ≈$556.98 per Anthropic share | Label "fund's fair-value mark (Level 3), not a valuation"; no company-level valuation from marks |
| Staleness | Tracxn header still $380B after the $965B round | Visible "Reviewed on" date; daily radar; stale badge after 30 days with open inbox items |
| Licensing | Epoch CC BY; AI Index CC BY-ND; paid tracker terms; news copyright | Attribution on every page and CSV; no remixed AI Index charts; no paid data; short quotes and links only |
| Editorial and legal | "Circular deal" framing can read as an accusation | Neutral text: "Amazon invested in Anthropic. Anthropic committed to buy AWS compute." Not investment advice |
| Privacy | Form D lists addresses of officers and directors | Never publish addresses; people pages show public roles only |
| Operations | SEC full-text search returned "getaddrinfo EBUSY" twice; GDELT took 14 s; Crunchbase and PitchBook bot checks | Retries with backoff; ≤10 SEC requests per second with a declared User-Agent; no scraping behind bot checks |
| Fast-changing status | xAI became part of SpaceX; SpaceX and Cerebras are public (SEC); Z.ai and MiniMax listed in Hong Kong (per Epoch's sources); Nscale filed an S-1 | `status` and `parent` fields with sources; status-change events on the timeline |
| Sample bias | Tokscale usage is not market share | Always "among Tokscale users" |

---

## 7. Decisions the owner must make

1. **Routes.** Decided 2026-10-21: everything under `/capital` (`/capital/companies`, `/capital/investors`, `/capital/rounds`, ...). Company pages already moved; `/companies/*` redirects with 308.
2. **Source policy.** Primary sources only? Or also press-reported numbers with status "reported" and the outlet named (recommended: allowed, but never in totals or header facts)?
3. **Epoch AI.** Import as a seed with CC BY attribution and re-verify each row against its primary source before publishing (recommended)? Or show Epoch rows as they are, labeled "via Epoch AI"?
4. **Fund marks (N-PORT).** Publish now, later, or never? They are new and useful, and they can be misread.
5. **Capex definition.** Cash paid for PP&E only (recommended)? Or also "including finance leases" as a separate measure where the company reports it?
6. **v1 coverage.** Approve or edit the 33 companies and 16 investors. Keep Chinese labs with sparse data? Add Databricks, Perplexity, Figure AI?
7. **Tokscale data on capital pages.** Show lab usage share next to funding (recommended, with the "among Tokscale users" label)?
8. **Storage.** JSON in the repo now (recommended) or database tables now?
9. **Editorial ownership.** Who reviews the radar inbox, how often, and who signs off numbers?
10. **Logos.** Company logos (license check needed) or monograms?
11. **13F.** Keep "Inside the 13F" as an editorial series under Public investors (recommended) or retire it?
12. **Names.** Keep "Capital"? "Compute" or "Infrastructure"? "Spending" or "Capex"?
13. **Other trackers.** Name them on product pages when totals differ? (Recommended: no vendor names on product pages; explain the definitions in the method page.)

---

## 8. Evidence index

All files are in `/tmp/tokscale-stealth/capital-plan/evidence/`.

| File | What it shows |
|---|---|
| `evidence-notes.json` | Quotes and URLs for every verified fact used in this plan (SpaceX 10-Q, Anthropic Series H, Amazon, CoreWeave, Akamai, TeraWulf, Broadcom 8-Ks, Nebius 6-K, Crusoe Form D, Nscale S-1 search, ARK N-PORT, tracker totals, Epoch double-count example) |
| `probe-log.json` | 149 requests: URL, time, HTTP status, content type, size, duration |
| `sec-capex-xbrl-sample.json` | Cash capex by company and period from SEC XBRL, with accession numbers |
| `ark-venture-nport-ai-positions.json` | ARK Venture Fund AI positions (period 2026-07-31) with computed per-share marks |
| `epoch-funding-rounds-sample.csv` | 20 newest Epoch funding-round rows |
| `epoch-data-centers-sample.csv`, `epoch_data_centers.zip` | Epoch frontier data centers (README with CC BY license inside the ZIP) |
| `wikidata-sample.json` | Wikidata facts for 7 companies |
| `hf-org-overview.json` | Hugging Face org stats for 19 labs |
| `crunchbase-llms.txt` | Crunchbase docs index (entity types and cards) |
| `pitchbook-anthropic-profile.md`, `www.cbinsights.com_company_anthropic.md`, `tracxn.com_d_companies_anthropic_*.md`, `sacra.com_c_anthropic_.md`, `epoch.ai_data_ai-companies*.md` | Page structure of each tracker (text copies) |
| `raw_*` | Raw responses (SEC submissions, XBRL, full-text search, RSS, robots.txt, OWID, GDELT, GitHub, Hugging Face, Crunchbase docs, filings) |
| `local-capital.png` | Current `/capital` on the dev server |
