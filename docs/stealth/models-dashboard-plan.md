# Models dashboard with benchmarks: plan

Stealth prototype, local only. Written 2026-10-21. Source data was checked on that date; evidence is in
`/tmp/tokscale-stealth/bench-research/` (raw downloads and coverage scripts).

## 1. What the dashboard must answer

1. Which models do builders use right now? (Tokscale usage: active users, change, adoption.)
2. Which models score best on the work I care about: agentic coding, web development, reasoning?
3. Which agent and model pairs do well, and which pairs do people actually use?
4. What does a model cost at real agent usage, not at a made-up input/output mix?
5. How fast is a new model adopted after release?

Rules carried over from the explore pages:

- No Tokscale composite score. Each benchmark stays in its own column with its own source.
- Every score shows its source, its date, and its run conditions (reasoning effort, agent or harness).
- A missing score is shown as "not evaluated", never as zero.
- Usage and benchmarks are shown side by side. We do not claim one causes the other.

## 2. Data sources (checked 2026-10-21)

| Source | What it gives | Access | License / terms | Freshness | Match with our models | Role |
|---|---|---|---|---|---|---|
| Tokscale submissions | Active users, tokens, token mix (input, output, cache read, cache write, reasoning), agent per model | Own DB | Own data | Live (10 min cache) | 100% | Core |
| [models.dev](https://models.dev/api.json) | Catalog: provider and lab, release date, knowledge cutoff, context and output limits, modalities, reasoning, tool use, open weights, prices including cache read/write | Public JSON (5.9 MB, 232 providers) | MIT (github sst/models.dev) | Pushed 2026-10-20 | 124 of 149 models; 92.8% of model-user pairs; 99.8% of tokens (30d, local seed) | Specs and prices |
| [Epoch AI benchmarking hub](https://epoch.ai/data/benchmark_data.zip) | 90 CSVs: Epoch Capabilities Index (ECI), GPQA Diamond, FrontierMath, SimpleQA Verified, SWE-bench Verified, plus collected external results (Terminal-Bench, CursorBench, DeepSWE, FrontierSWE, WebDev Arena, METR time horizon, HLE, ARC-AGI-2, APEX-Agents, GDPval, SciCode, and more). Also `model_metadata.csv` and `benchmark_metadata.csv` (scale, random baseline, ceiling, superseded-by) | Public zip (2.4 MB) | CC BY 4.0 (README in the zip); credit Epoch AI and the original benchmark | Rows through 2026-10-17 | Model metadata: 96 of 149 models, 86.6% of model-user pairs, 99.2% of tokens. ECI: 14 of our top 15 models | Capability core |
| [LMArena leaderboard dataset](https://huggingface.co/datasets/lmarena-ai/leaderboard-dataset) | 24 boards: text, webdev (+ react, html), agent (+ bash reliability, recovery steps, steerability, tool hallucination, task outcome), search, vision, and more. Score with 95% CI, vote or session count, rank, publish date | Hugging Face parquet | CC BY 4.0 | Published 2026-10-19 | Names carry effort, e.g. "Claude Opus 5.5 (Max)"; top models present | Human preference, agent arena |
| [Terminal-Bench 4.0](https://www.tbench.ai/leaderboard) | Agent x model accuracy, tokens, cost, run date (35 rows). Agents: Claude Code, Codex, Grok Build, Muse Code, mini-SWE-agent | Leaderboard page embeds JSON | Harness code Apache-2.0; site data terms not stated. Ask before public use | Latest run 2026-09-29 | 23 models incl. Opus 5.5, GPT-6.1 Sol, GPT-6 Astra, Sonnet 5.5 | Agentic coding, agent x model |
| [Artificial Analysis free API](https://artificialanalysis.ai/documentation) | Intelligence, coding and math indexes, evals, prices, output tokens per second, time to first token | API key, 1,000 requests per day, server-side cache required | Attribution required; Terms of Use and Data Platform Terms | Not checked (needs key) | Not checked | Speed (only source). Phase 2 |
| [OpenRouter models API](https://openrouter.ai/api/v1/models) | 468 models, prices, context; Artificial Analysis indexes for 304 models | Public JSON | API terms; the indexes are Artificial Analysis data | Live | Not measured | Cross-check only |
| [SWE-bench leaderboards](https://raw.githubusercontent.com/SWE-bench/swe-bench.github.io/master/data/leaderboards.json) | Verified (180), Lite, Multimodal, Multilingual boards | GitHub raw | License not stated | Newest entry 2026-02-26 | Mostly older models | History only |
| Aider polyglot, LiveBench | Coding and general boards | GitHub YAML, Hugging Face | Open | Last updates 2025-10 and 2025-04 | Stale | Skip |
| [OpenRouter Datasets API](https://openrouter.ai/docs/client-sdks/python/sdks/datasets/README.md) | Daily token totals for the top 50 models; top apps by tokens (filter `coding`, `cli-agent`); weekly cost per session by harness and model | Any OpenRouter API key; 30 requests/min, 500/day | CC BY 4.0, "Source: OpenRouter ..., as of ..." | Daily / weekly | Not measured (needs key) | Second usage sample next to Tokscale; harness x model cost |
| [OpenRouter Benchmarks API](https://openrouter.ai/docs/client-sdks/python/sdks/benchmarks/README.md) | One endpoint for Artificial Analysis indexes, Design Arena, and OpenRouter's own tau-bench, GPQA and search evals | Any OpenRouter API key | License not stated (the Datasets API is CC BY; this one is not) | Live | Not measured | Cross-check only |
| [SWE-Bench Pro V2](https://labs.scale.com/leaderboard/swe_bench_pro_public_v2) (Scale) | Refreshed public split (642 tasks, Sept 22, 2026), locked protocol, re-graded on a clean image | Leaderboard site; tasks on GitHub | Tasks MIT (github scaleapi/SWE-bench_Pro-os); leaderboard site "All rights reserved" | Sept 2026 | Newest frontier models | Cite and link; ask before republishing |
| [Vals AI](https://www.vals.ai/benchmarks/terminal-bench-4) | Independent runs with one fixed harness (Mini-SWE-agent): Terminal-Bench 4.0, SWE-bench Verified; score, cost per test, duration | Public pages | Terms not found | Updated 2026-10-19 | Opus 5.5 65.15%, Sonnet 5.5 64.14%, GPT-6 Astra 59.60% (TB 4.0) | Same-harness comparison; ask before republishing |
| [Artificial Analysis coding agents](https://artificialanalysis.ai/agents/coding-agents) | Coding Agent Index v1.5 (DeepSWE v1.1, Terminal-Bench 4.0, SWE-Atlas-QnA), harness comparison, cost, tokens and time per task, refusals, reward hacking | Public pages; API terms apply | Attribution; Terms of Use and Data Platform Terms | Current | 14 of 39 models shown by default | Reference; licensing to confirm |

## 3. Findings that shape the design

1. **Coverage is good.** Catalog and capability data exist for the models that carry about 99% of tokens. The gaps are internal or
   unlisted models (`codex-auto-review`, `gpt-reserve`) and short ids (`k3`).
2. **SWE-bench Verified is stale.** The newest official entry is from February 2026, and the newest models are not on it. It goes
   into a "history" group, not the main board.
3. **Benchmarks now exist per agent, not only per model.** Terminal-Bench 4.0 ranks agent x model pairs (for example Opus 5.5 in
   Claude Code at 64.85%). LMArena runs an agent arena. Tokscale knows which agent carries each model's tokens. Joining the two
   is something only Tokscale can show.
4. **Real agent usage is almost all cache reads.** In the local seed (built from 20 real public profiles), 95-98% of each top
   model's tokens are cache reads. At list prices that gives an effective price far below the usual 3:1 input/output blend:

   | Model | List $/1M in / out / cache read | Cache-read share | Effective $/1M at this mix | 3:1 blend |
   |---|---|---|---|---|
   | Claude Opus 5.5 | 4 / 20 / 0.2 | 98.2% | $0.32 | $8.00 |
   | GPT-6.1 Sol | 2 / 10 / 0.1 | 96.6% | $0.20 | $4.00 |
   | GPT-6 Astra | 10 / 50 / 1 | 97.1% | $1.38 | $20.00 |
   | Claude Sonnet 5.5 | 2 / 10 / 0.1 | 97.0% | $0.20 | $4.00 |

   This must be measured again on production data before we publish it. If it holds, "price at real agent usage" is a strong,
   unique column.
5. **Results depend on conditions.** The same model appears as "(Max)", "(High)", "_xhigh", or with different agents. Effort and
   agent are stored as conditions of a result, never as part of the model identity.
6. **Scales differ.** ECI is an index, arena scores are Elo-like ratings with confidence intervals, and agent benchmarks are
   pass rates. Each gets its own column and unit. ECI is labelled "Epoch Capabilities Index", not a Tokscale score.
7. **Interesting gaps appear.** For example, Gemini 4 Argon is #5 on ECI but has little Tokscale usage. The usage-vs-capability
   chart makes these gaps visible.

## 3a. Market check (searched 2026-10-21)

- **Kilo** ([kilo.ai/leaderboard](https://kilo.ai/leaderboard)) already ranks models by real usage inside Kilo Code, next to its own
  KiloBench scores, with a cost-vs-performance chart where bubble size is Kilo usage. This is the closest product to ours.
  Its usage comes from one agent. Tokscale sees usage across about 60 agents, split per agent, with the real token mix.
- **Artificial Analysis** runs a Coding Agent Index with harness comparisons, cost per task and time per task.
- **Vals AI** runs Terminal-Bench 4.0 and SWE-bench Verified with one fixed harness, with cost and duration per test.
- **Aggregators** such as BenchLM and llm-stats list hundreds of benchmarks without usage data.

So "benchmarks next to usage" is not new by itself. What only Tokscale can show:

1. Usage across many agents, and which agent carries each model.
2. Real token mix and the effective price it implies (cache reads dominate).
3. Public builder profiles linked to the models they use.

## 4. Pages

```
/models                   Models dashboard (new landing for Models)
/models/all               Full catalog (today's /models list; long tail paginated)
/models/[key]             Model page + Benchmarks, Pricing, Specs sections
/models/compare?m=a,b,c   Side-by-side comparison, shareable URL
/benchmarks               Index of tracked benchmarks by category
/benchmarks/[slug]        One benchmark: full results, conditions, sources
```

### /models dashboard

```
Models                                               [7d | 30d | All time]
What builders use, how models score, and what they cost.

[Usage] [Capability] [Agentic coding] [Web dev] [Price] [Specs]   <- column groups
┌──────────────────────────────────────────────────────────────────────────┐
│ #  Model            Active users  ECI    Terminal-Bench 4.0  WebDev   $/1M │
│ 1  [◎] Claude Opus 5.5   165 +87   168.8  64.9% Claude Code   1807   0.32 │
│ 2  [◎] GPT-6.1 Sol       141 +82   168.4  58.2% Codex         1800   0.20 │
│ ...  each cell: value, small condition tag (Max, xhigh), source on hover │
└──────────────────────────────────────────────────────────────────────────┘
Usage vs capability (scatter)       Price vs capability (frontier line)
Best in the agents people use (model x agent matrix: usage share + score)
Adoption after launch (days since release vs active users, last 8 releases)
New this week (releases, new results)        Sources and attribution
```

- Default rows: models with 5+ active users in the period, plus the top 20 by ECI (about 60 models on the seed).
- Column groups switch with chips; the URL keeps the state (`?cols=usage,coding`).
- Every score cell has a popover: benchmark, version, value with CI, effort, agent or harness, run date, source link.
- Mobile: one card per model with the selected column group, no sideways scrolling.

### /models/[key] additions

- **Benchmarks:** one row per tracked benchmark: score, rank among models we track, effort, agent or harness, date, source.
  Agent rows from Terminal-Bench sit next to this model's Tokscale agent share.
- **Pricing:** list prices per provider (models.dev); effective price at the community's token mix; effective price at this
  model's own mix.
- **Specs:** context, output limit, modalities, knowledge cutoff, open weights, release date.

### /benchmarks and /benchmarks/[slug]

- Index grouped by category, with what each benchmark measures, publisher, last update, number of models, and a "saturated" or
  "stale" note when it applies.
- Detail page: results table with conditions, a score-over-release-date chart, a Tokscale usage column, method notes, license and
  attribution.

### First benchmark set

| Category | Benchmark | Source |
|---|---|---|
| Overall | Epoch Capabilities Index | Epoch AI |
| Agentic coding | Terminal-Bench 4.0 | tbench.ai or Vals AI (terms to confirm) |
| Agentic coding | SWE-Bench Pro V2 | Scale (cite and link until terms are confirmed) |
| Agentic coding | CursorBench, DeepSWE, FrontierSWE | Epoch AI (collected) |
| Web development | WebDev Arena | LMArena |
| Agents | Agent Arena | LMArena |
| Reasoning | GPQA Diamond, HLE, FrontierMath Tiers 1-3 v2 | Epoch AI |
| Knowledge | SimpleQA Verified | Epoch AI |
| History | SWE-bench Verified | Epoch AI, SWE-bench |
| Speed (phase 2) | Output tokens/s, time to first token | Artificial Analysis |

## 5. Data model

Phase 1 stores typed JSON snapshots in `src/content/benchmarks/`, validated with zod at load (the same pattern as Capital).
Database tables can come later.

- **Benchmark:** slug, name, category, publisher, URL, license, attribution text, unit, scale, higher-is-better, random baseline,
  ceiling, saturated, superseded-by, versions.
- **Result:** benchmark, version, source, external model id, mapped model key, effort, agent or harness, score, CI low and high,
  votes or sessions, cost, tokens, run date, published date, measured-by (`lab`, `independent`, later `tokscale`), source URL,
  retrieved-at.
- **Model spec** (from models.dev): model key, provider offerings (provider, id, input, output, cache read, cache write prices,
  context, output limit), release date, knowledge cutoff, modalities, reasoning, tool use, open weights.
- **Alias map:** (source, external id) -> model key, with method (`exact`, `normalized`, `manual`) and a reviewed flag.

Identity rules: the model key is the canonical Tokscale key. Effort and agent are conditions. Dated snapshots and host prefixes
fold into the key. External ids that do not map are listed in an "unmapped" report and are never guessed.

## 6. Pipeline

`scripts/benchmarks/sync.ts` (run by hand in the stealth branch; a scheduled job later):

1. Fetch models.dev JSON, the Epoch zip, LMArena parquet boards (webdev, agent, text), and the Terminal-Bench page JSON.
2. Normalize ids, map them to model keys, attach conditions.
3. Validate, then write JSON snapshots and a report: coverage by tokens and by users, unmapped ids, and changes since the last
   snapshot.
4. If one source fails, keep its last good snapshot and show that snapshot's date on the page.

Every page that shows a source shows its attribution line.

## 7. Delivery phases

| Phase | Ships | Done when |
|---|---|---|
| 1 (stealth) | Sync script and snapshots for models.dev, Epoch, LMArena, Terminal-Bench; `/models` dashboard (board, two charts); model page Benchmarks, Pricing and Specs; `/benchmarks` index and detail; `/models/all` | A visitor can go from the board to a model, see its scores with conditions and sources, and open the benchmark |
| 2 | Compare page; model x agent matrix; adoption-after-launch chart; speed from Artificial Analysis | A visitor can compare three models and see which agent each is strongest in |
| 3 | "Measured by Tokscale": our own runs of agent x model on a fixed task set (Terminal-Bench harness), with cost and time | Results are reproducible and published with logs |

## 8. Risks

- **Wrong model matches.** Mitigation: alias map with review flags and an unmapped report.
- **Stale benchmarks.** Mitigation: dates on every cell, stale and saturated notes.
- **Licensing.** Epoch and LMArena are CC BY 4.0, models.dev is MIT. Terminal-Bench site data and the Artificial Analysis terms
  must be confirmed before public launch.
- **Over-reading the numbers.** No composite score, no causal claims, coverage notes on every table.
- **Sample bias.** Tokscale users are self-selected. The local seed is a synthetic remix; all coverage numbers above must be
  measured again on production data.

## 9. Decisions for the owner

1. Should `/models` become the dashboard, with the full catalog at `/models/all`?
2. Is the first benchmark set right?
3. Should we create an Artificial Analysis account for speed data? The API key would be stored as a secret.
4. For Terminal-Bench: ask the maintainers for permission, or only link to their leaderboard?
5. Should "price at real agent usage" be public after we measure it on production?
6. Do we want our own evaluations later (phase 3)?
7. Should we create a free OpenRouter API key for its CC BY usage datasets (a second usage sample, and cost per session by
   harness and model)? It would be stored as a secret.

## 10. Phase 1 build notes (2026-10-21)

Built on branch `stealth/ai-v2-models2`. Local only.

### Data pipeline

- Run `bun scripts/benchmarks/sync.ts` from `packages/frontend` (add `--offline` to reuse cached downloads). With a local
  `DATABASE_URL`, Tokscale usage keys join the match set and coverage by tokens and users is printed.
- Raw downloads are cached in `packages/frontend/.stealth-cache/benchmarks/` (gitignored). Committed files in
  `src/content/benchmarks/` hold normalized rows only: `epoch.json`, `lmarena.json`, `tbench.json`, `models-dev.json`,
  `aliases.json` and `report.json` (unmapped ids and match counts).
- LMArena is read through the Hugging Face datasets-server JSON API (no parquet reader). The text board is fetched for
  coverage only and is not published.
- Terminal-Bench 4.0 rows come from the JSON embedded in the tbench.ai page. Only result rows are stored; every cell links to
  the leaderboard. A reported cost of $0 is stored as "not reported".
- If a source fails, its last snapshot is kept and the report marks it.

### Matching

- Keys: Tokscale usage keys plus models.dev keys. A result maps only to a key that exists; nothing is guessed.
- Methods in `aliases.json`: exact 294, normalized 166, manual 2. All 462 rules are unreviewed. The two manual rules
  (LMArena "Muse Wonder", "Muse Wonder Full") carry a note with the evidence.
- Results matched: Epoch 872 of 1,064 rows, LMArena 181 of 201, Terminal-Bench 35 of 35. 200 external models stay
  unmapped; most are older or niche models.

### Coverage against the local seed (last 30 days, 149 models, 60 with 5+ users)

| Data | Models with 5+ users covered | Share of tokens | Share of user-model pairs |
|---|---|---|---|
| models.dev specs | 37 of 60 | 98.5% | 78.0% |
| First-party list price | 35 of 60 | 98.2% | 75.2% |
| Epoch Capabilities Index | 35 of 60 | 98.4% | 75.8% |
| Terminal-Bench 4.0 | 20 of 60 | 97.5% | 62.8% |
| WebDev Arena | 38 of 60 | 98.6% | 77.7% |
| Agent Arena | 30 of 60 | 98.4% | 70.9% |

The uncovered models with 5+ users are mostly free or routed variants (`*-free`, `space-bunny-*`), internal ids
(`codex-auto-review`) and short ids (`k3`). Measure again on production data before publishing.

### Price at real agent usage

- Community mix in the local seed (last 30 days): cache reads are 97.7% of tokens. Claude Opus 5.5 at list prices: $0.35
  per 1M tokens at the community mix and $0.32 at its own mix (checked by hand against the database).
- The list price is the lab's own API on models.dev when listed; otherwise the first listed provider, named on the page.
  Reasoning tokens are priced as output. Long-context tiers, batch prices and discounts are not applied.

### Dashboard decisions

- `/models` opens on an overview (active users, top agent, ECI, Terminal-Bench 4.0, WebDev Arena, $/1M at agent mix).
  Group chips switch to full column sets; `?cols=` and `?sort=` keep the state.
- A score cell shows the best result the source reports for the model, with its effort and agent or harness, and a popover
  with the source name, interval, date and link.
- Still open: review the alias map; confirm Terminal-Bench and SWE-Bench Pro V2 terms; phase 2 items (compare page,
  model x agent matrix, adoption chart, speed data).

### Other pages

- `/models/all`: the full catalog, grouped by lab. Each lab lists its top 5 models and folds the rest behind "Show N more".
  A search or one lab lists every match; `?lab=` server-renders the whole lab.
- `/models/[key]`: Benchmarks (best result per benchmark, rank among matched models, conditions, date, source, other
  results behind a disclosure, Terminal-Bench rows next to the model's token share through that agent), Pricing (list
  prices per provider, price at real agent usage at this model's mix and the community mix, the 3:1 blend, assumptions)
  and Specs.
- A model with benchmark results or a models.dev entry but no Tokscale usage gets a page with a "No public Tokscale usage
  yet" note. It is not indexed and not in the sitemap.
- `/benchmarks`: index by category. `/benchmarks/[slug]`: results with conditions, cost, date and a source link per row,
  active Tokscale users in the last 30 days, best result per model by default (all results on request, 50 rows before
  "Show all"), a score by release date chart with a data table, and method, matching and license notes. SWE-Bench Pro V2
  is link-only and has no page.
- Sitemap: `benchmarksSitemapEntries()` and `modelsCatalogSitemapEntry(lastModified)` in `src/lib/benchmarks/sitemap.ts`
  are ready; the integration branch wires them into `src/app/sitemap.ts`.
