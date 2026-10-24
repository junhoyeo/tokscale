import { z } from "zod";

/**
 * Capital v2 ledger: organizations, events, typed amounts and their sources.
 *
 * Content lives in src/content/capital (orgs.json, events/*.json,
 * metrics/*.json). It is validated once at module load (see content.ts), so a
 * malformed edit fails the build and the tests instead of rendering a wrong
 * number. Rules the schema cannot express are checked in content.ts.
 */

export const isoDate = z.string().regex(/^\d{4}-\d{2}-\d{2}$/, "expected YYYY-MM-DD");
/** A year, a month or a day: "2021", "2021-01", "2021-01-26". */
export const partialDate = z.string().regex(/^\d{4}(?:-\d{2}(?:-\d{2})?)?$/, "expected YYYY, YYYY-MM or YYYY-MM-DD");
export const slug = z.string().regex(/^[a-z0-9]+(?:-[a-z0-9]+)*$/, "expected a lowercase slug");
export const orgId = z.string().regex(/^org_[a-z0-9]+(?:_[a-z0-9]+)*$/, "expected an org id like org_anthropic");
export const eventId = z.string().regex(/^evt_[a-z0-9]+(?:_[a-z0-9]+)*$/, "expected an event id like evt_anthropic_series_h_2026");
export const sourceId = z.string().regex(/^src_[a-z0-9]+(?:_[a-z0-9]+)*$/, "expected a source id like src_anthropic_series_h");
export const statementId = z.string().regex(/^met_[a-z0-9]+(?:_[a-z0-9]+)*$/, "expected a metric statement id like met_anthropic_run_rate_2026_05");
const cik = z.string().regex(/^\d{10}$/, "expected a 10-digit zero-padded CIK");
const accession = z.string().regex(/^\d{10}-\d{2}-\d{6}$/, "expected an SEC accession number");
/** ISO 4217. The original currency is kept; there is no silent FX conversion. */
export const currency = z.string().regex(/^[A-Z]{3}$/, "expected an ISO 4217 currency code");
const country = z.string().regex(/^[A-Z]{2}$/, "expected an ISO 3166-1 alpha-2 country code");
const quote = z.string().trim().min(8, "a quote must be copied from the source");

/* ---------------------------------------------------------------- sources */

export const SOURCE_TYPES = [
  "company_announcement",
  "sec_filing",
  "exchange_filing",
  "dataset",
  "news",
  "regulator",
] as const;
export type SourceType = (typeof SOURCE_TYPES)[number];

/** Source types that count as primary evidence. News is secondary. */
export const PRIMARY_SOURCE_TYPES: ReadonlySet<SourceType> = new Set([
  "company_announcement",
  "sec_filing",
  "exchange_filing",
  "regulator",
]);

export const sourceSchema = z
  .object({
    id: sourceId,
    url: z.string().url(),
    /** Who published it: "Anthropic", "SEC EDGAR (Akamai Technologies, Inc.)", "Reuters". */
    publisher: z.string().min(1),
    type: z.enum(SOURCE_TYPES),
    title: z.string().min(1),
    publishedAt: isoDate.optional(),
    /** When Tokscale last read the document. */
    retrievedAt: isoDate,
    filing: z
      .object({
        form: z.string().min(1),
        accession,
        cik,
        filedAt: isoDate,
        periodOfReport: isoDate.optional(),
      })
      .optional(),
    license: z.enum(["public-domain-sec", "CC-BY-4.0", "CC0", "publisher"]).optional(),
  })
  .strict();

/* ------------------------------------------------------------------- orgs */

export const CATEGORIES = [
  "ai-lab",
  "ai-app",
  "hyperscaler",
  "cloud",
  "neocloud",
  "semiconductors",
  "data-centers",
  "power",
] as const;
export type Category = (typeof CATEGORIES)[number];

export const INVESTOR_TYPES = [
  "vc",
  "growth",
  "sovereign",
  "corporate",
  "asset-manager",
  "registered-fund",
] as const;
export type InvestorType = (typeof INVESTOR_TYPES)[number];

export const ORG_STATUSES = ["private", "public", "subsidiary", "acquired", "closed"] as const;
export type OrgStatus = (typeof ORG_STATUSES)[number];

const tickerSchema = z
  .object({
    exchange: z.enum(["NASDAQ", "NYSE", "HKEX", "TWSE", "LSE", "TSE"]),
    symbol: z.string().regex(/^[A-Z0-9.]{1,8}$/),
  })
  .strict();

export const orgIdsSchema = z
  .object({
    /** Wikidata item, "Q116758847". */
    wikidata: z.string().regex(/^Q\d+$/).optional(),
    /** Every CIK the org files under (zero-padded), primary first. */
    secCik: z.array(cik).default([]),
    tickers: z.array(tickerSchema).default([]),
    /** Hugging Face organization name, "deepseek-ai". */
    hfOrg: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_.-]*$/).optional(),
    /** GitHub organization login, "anthropics". */
    githubOrg: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9-]*$/).optional(),
    /** SEC investment adviser number, "801-114985". */
    secAdviser: z.string().regex(/^\d{3}-\d{4,6}$/).optional(),
  })
  .strict();

export const orgSchema = z
  .object({
    id: orgId,
    slug,
    name: z.string().min(1),
    /** Short form for dense tables ("Andreessen Horowitz" -> "a16z" only if the firm uses it). */
    shortName: z.string().min(1).optional(),
    /** Legal names as written in filings ("Anthropic PBC", "Anthropic, PBC"). */
    legalNames: z.array(z.string().min(1)).default([]),
    /** Other names people search for ("Zhipu", "Knowledge Atlas", "Anysphere"). */
    aliases: z.array(z.string().min(1)).default([]),
    kinds: z.array(z.enum(["company", "investor"])).min(1),
    categories: z.array(z.enum(CATEGORIES)).default([]),
    investorTypes: z.array(z.enum(INVESTOR_TYPES)).default([]),
    status: z.enum(ORG_STATUSES),
    /** Plain words shown next to the status ("Public benefit corporation"). */
    statusNote: z.string().min(1).optional(),
    parent: z.object({ org: orgId, since: isoDate, sourceId, quote }).strict().optional(),
    hq: z.object({ city: z.string().min(1).optional(), country }).strict().optional(),
    founded: z.object({ date: partialDate, sourceId }).strict().optional(),
    website: z.string().url().optional(),
    /** One plain sentence in Tokscale's words. No numbers. */
    description: z.string().min(1),
    ids: orgIdsSchema.default({}),
    /** Tokscale model lab slugs that belong to this org ("google" -> Alphabet). */
    labSlugs: z.array(slug).default([]),
    /** Issuer names as written in N-PORT filings ("Anthropic, Inc."). */
    nportIssuerNames: z.array(z.string().min(1)).default([]),
    /** Vendored mark under /assets/v2/models (LobeHub, MIT). Monogram when absent. */
    logo: z.string().regex(/^\/assets\/v2\/models\/[a-z0-9]+\.svg$/).optional(),
    /**
     * Square company picture under /assets/v2/orgs: the org's own X profile picture,
     * LinkedIn logo or site icon (provenance in SOURCES.json there). Used when there
     * is no `logo`.
     */
    avatar: z.string().regex(/^\/assets\/v2\/orgs\/[a-z0-9-]+\.webp$/).optional(),
    /** Part of the owner-approved seed list (33 companies, 16 investors). */
    seed: z.boolean().default(false),
    reviewedAt: isoDate,
  })
  .strict();

/* ---------------------------------------------------------------- amounts */

export const MEASURES = [
  "equity_raised",
  "debt_raised",
  "secondary_sale",
  "ipo_proceeds",
  "valuation_post",
  "valuation_pre",
  "equity_investment",
  "investment_commitment",
  "cumulative_investment",
  "contract_value",
  "compute_commitment",
  "purchase_price",
  "capex_actual",
  "capex_guidance",
  "revenue_reported",
  "revenue_run_rate",
  "form_d_amount_sold",
  "form_d_offering_amount",
  "fund_mark",
  "estimated_capital_cost",
  "price_per_share",
] as const;
export type Measure = (typeof MEASURES)[number];

/**
 * Measures that can be added up (within one currency and one basis family).
 * Valuations, marks, cumulative statements, run-rates, guidance, estimates and
 * per-share prices are levels or opinions, not flows: summing them is an error.
 */
export const SUMMABLE_MEASURES: ReadonlySet<Measure> = new Set<Measure>([
  "equity_raised",
  "debt_raised",
  "secondary_sale",
  "ipo_proceeds",
  "equity_investment",
  "investment_commitment",
  "contract_value",
  "compute_commitment",
  "purchase_price",
  "capex_actual",
  "revenue_reported",
  "form_d_amount_sold",
  "form_d_offering_amount",
]);

export const BASES = [
  "exact",
  "approximately",
  "up_to",
  "up_to_approximately",
  "more_than",
  "at_least",
  "range",
] as const;
export type Basis = (typeof BASES)[number];

/** Per-amount status. "rumored" and "in talks" do not exist, so they cannot be published. */
export const AMOUNT_STATUSES = ["announced", "signed", "closed", "filed", "reported", "estimated", "undisclosed"] as const;
export type AmountStatus = (typeof AMOUNT_STATUSES)[number];

const termSchema = z
  .object({
    years: z.number().positive().optional(),
    through: isoDate.optional(),
    /** Free text when the source states a start ("beginning in 2027"). */
    starts: z.string().min(1).optional(),
  })
  .strict();

export const amountSchema = z
  .object({
    /** Unique within its event or file ("a1"). */
    id: z.string().regex(/^[a-z0-9_]+$/),
    /** Short label for tables ("Series H equity"). */
    label: z.string().min(1).optional(),
    measure: z.enum(MEASURES),
    /** Whole units of the currency. null only when the status is "undisclosed". */
    value: z.number().nonnegative().nullable(),
    currency,
    basis: z.enum(BASES),
    status: z.enum(AMOUNT_STATUSES),
    asOf: isoDate,
    from: orgId.optional(),
    to: orgId.optional(),
    term: termSchema.optional(),
    /** "if Microsoft acquires additional services" */
    condition: z.string().min(1).optional(),
    /** The event that already counts this money. Totals skip the amount. */
    includedIn: eventId.optional(),
    /** Id of a larger amount in the same event that contains this one ("$5B from Amazon" inside "$65B"). Totals skip it. */
    partOf: z.string().regex(/^[a-z0-9_]+$/).optional(),
    /** Id of the amount in the same event that this one is an alternative figure for ("about $19.4 billion" with options). Totals skip it. */
    alternativeTo: z.string().regex(/^[a-z0-9_]+$/).optional(),
    sourceId,
    quote,
  })
  .strict();

export const QUANTITY_MEASURES = [
  "it_load_mw",
  "gross_power_mw",
  "capacity_gw",
  "gpus",
  "h100e",
  "shares",
  "investors",
] as const;
export type QuantityMeasure = (typeof QUANTITY_MEASURES)[number];

export const quantitySchema = z
  .object({
    id: z.string().regex(/^[a-z0-9_]+$/),
    label: z.string().min(1).optional(),
    measure: z.enum(QUANTITY_MEASURES),
    value: z.number().positive(),
    basis: z.enum(BASES),
    condition: z.string().min(1).optional(),
    term: termSchema.optional(),
    sourceId,
    quote,
  })
  .strict();

/** A sourced fact that is not a number Tokscale computes with (term, delivery date, warrant terms). */
export const factSchema = z
  .object({
    label: z.string().min(1),
    text: z.string().min(1),
    sourceId,
    quote,
  })
  .strict();

/* ----------------------------------------------------------------- events */

export const EVENT_KINDS = ["financing", "deal", "acquisition", "ipo", "filing"] as const;
export type EventKind = (typeof EVENT_KINDS)[number];

export const EVENT_TYPES = [
  "equity_round",
  "strategic_investment",
  "debt_facility",
  "tender_offer",
  "ipo",
  "registration",
  "form_d",
  "compute_contract",
  "compute_commitment",
  "datacenter_lease",
  "chip_supply",
  "ppa",
  "joint_venture",
  "acquisition",
  "asset_sale",
  "warrant",
] as const;
export type EventType = (typeof EVENT_TYPES)[number];

export const EVENT_STATUSES = ["announced", "signed", "closed", "filed", "completed", "terminated"] as const;
export type EventStatus = (typeof EVENT_STATUSES)[number];

export const ROLES = [
  "issuer",
  "lead_investor",
  "co_lead",
  "investor",
  "strategic_investor",
  "buyer",
  "provider",
  "lessor",
  "lessee",
  "acquirer",
  "target",
  "seller",
  "lender",
  "borrower",
  "filer",
  "partner",
] as const;
export type Role = (typeof ROLES)[number];

export const RELATION_TYPES = ["includes", "part_of", "amends", "supersedes", "follows", "evidence_for", "related"] as const;
export type RelationType = (typeof RELATION_TYPES)[number];

const datesSchema = z
  .object({
    announced: isoDate.optional(),
    signed: isoDate.optional(),
    closed: isoDate.optional(),
    filed: isoDate.optional(),
    completed: isoDate.optional(),
  })
  .strict();

export const eventSchema = z
  .object({
    id: eventId,
    slug,
    kind: z.enum(EVENT_KINDS),
    types: z.array(z.enum(EVENT_TYPES)).min(1),
    title: z.string().min(1),
    /** One or two plain sentences in Tokscale's words. Numbers here must also be amounts. */
    summary: z.string().min(1),
    /** "Series H" */
    roundLabel: z.string().min(1).optional(),
    /** The date the event is listed under. Must equal one of `dates`. */
    date: isoDate,
    dates: datesSchema,
    status: z.enum(EVENT_STATUSES),
    participants: z
      .array(
        z
          .object({ org: orgId, roles: z.array(z.enum(ROLES)).min(1), note: z.string().min(1).optional() })
          .strict()
      )
      .min(1),
    amounts: z.array(amountSchema).default([]),
    quantities: z.array(quantitySchema).default([]),
    facts: z.array(factSchema).default([]),
    relations: z
      .array(z.object({ type: z.enum(RELATION_TYPES), event: eventId, note: z.string().min(1).optional() }).strict())
      .default([]),
    sourceIds: z.array(sourceId).min(1),
    reviewedAt: isoDate,
    corrections: z.array(z.object({ date: isoDate, text: z.string().min(1) }).strict()).default([]),
  })
  .strict();

/** One file per event: the event plus the sources it cites. Shared sources may repeat if identical. */
export const eventFileSchema = z.object({ sources: z.array(sourceSchema).default([]), event: eventSchema }).strict();

export const orgsFileSchema = z
  .object({ sources: z.array(sourceSchema).default([]), orgs: z.array(orgSchema) })
  .strict();

/* ---------------------------------------------------------------- metrics */

/** A number a company states about itself (run-rate revenue) or a filing reports. */
export const statementSchema = z
  .object({
    id: statementId,
    org: orgId,
    label: z.string().min(1),
    measure: z.enum(MEASURES),
    value: z.number().nonnegative(),
    currency,
    basis: z.enum(BASES),
    status: z.enum(AMOUNT_STATUSES),
    asOf: isoDate,
    /** The period the number covers, when it is a period total ("three months ended June 30, 2026"). */
    period: z.object({ start: isoDate, end: isoDate, label: z.string().min(1) }).strict().optional(),
    sourceId,
    quote,
  })
  .strict();

export const statementsFileSchema = z
  .object({ sources: z.array(sourceSchema).default([]), statements: z.array(statementSchema) })
  .strict();

export type Source = z.infer<typeof sourceSchema>;
export type Org = z.infer<typeof orgSchema>;
export type OrgIds = z.infer<typeof orgIdsSchema>;
export type Amount = z.infer<typeof amountSchema>;
export type Quantity = z.infer<typeof quantitySchema>;
export type Fact = z.infer<typeof factSchema>;
export type CapitalEvent = z.infer<typeof eventSchema>;
export type Participant = CapitalEvent["participants"][number];
export type Relation = CapitalEvent["relations"][number];
export type EventFile = z.infer<typeof eventFileSchema>;
export type Statement = z.infer<typeof statementSchema>;

/* ------------------------------------------------------------------ capex */

/**
 * One XBRL fact for cash paid for property and equipment, as filed. 10-Q cash
 * flows are year to date, so most facts span 6 or 9 months; quarters are
 * derived from them in capex.ts and flagged.
 */
export const capexFactSchema = z
  .object({
    /** XBRL concept, "us-gaap:PaymentsToAcquirePropertyPlantAndEquipment". */
    concept: z.string().regex(/^[a-z-]+:[A-Za-z]+$/),
    start: isoDate,
    end: isoDate,
    value: z.number().nonnegative(),
    form: z.string().min(1),
    accession,
    filed: isoDate,
    /** Fiscal year and period of the filing the fact came from (XBRL fy/fp), not of the fact. */
    fy: z.number().int().optional(),
    fp: z.string().optional(),
  })
  .strict();

export const capexSeriesSchema = z
  .object({
    org: orgId,
    cik,
    currency,
    /** Fiscal year end as MM-DD ("06-30"); from the SEC submissions record. */
    fiscalYearEnd: z.string().regex(/^\d{2}-\d{2}$/),
    /** Line item as the company labels it ("Additions to property and equipment"). */
    lineItem: z.string().min(1).optional(),
    facts: z.array(capexFactSchema).min(1),
    /** Per-company caveats ("Excludes finance leases", "Tag changed in 2021"). */
    notes: z.array(z.string().min(1)).default([]),
  })
  .strict();

export const capexFileSchema = z
  .object({
    /** When the SEC responses used for this file were fetched (ISO timestamp, UTC). */
    retrievedAt: z.string().datetime(),
    source: z.literal("SEC XBRL companyfacts"),
    definition: z.string().min(1),
    series: z.array(capexSeriesSchema),
  })
  .strict();

export type CapexFact = z.infer<typeof capexFactSchema>;
export type CapexSeries = z.infer<typeof capexSeriesSchema>;
export type CapexFile = z.infer<typeof capexFileSchema>;
