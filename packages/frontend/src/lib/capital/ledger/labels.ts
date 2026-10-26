import type {
  AmountStatus,
  Basis,
  Category,
  EventKind,
  EventStatus,
  EventType,
  InvestorType,
  Measure,
  OrgStatus,
  QuantityMeasure,
  Role,
} from "./schema";

/** Plain labels for the ledger's closed vocabularies. One place, so every page says the same thing. */

export const CATEGORY_LABELS: Record<Category, string> = {
  "ai-lab": "AI lab",
  "ai-app": "AI app",
  hyperscaler: "Hyperscaler",
  cloud: "Cloud",
  neocloud: "Neocloud",
  semiconductors: "Chips",
  "data-centers": "Data centers",
  power: "Power",
};

export const CATEGORY_PLURALS: Record<Category, string> = {
  "ai-lab": "AI labs",
  "ai-app": "AI apps",
  hyperscaler: "Hyperscalers",
  cloud: "Clouds",
  neocloud: "Neoclouds",
  semiconductors: "Chips",
  "data-centers": "Data centers",
  power: "Power",
};

export const INVESTOR_TYPE_LABELS: Record<InvestorType, string> = {
  vc: "Venture capital",
  growth: "Growth",
  sovereign: "Sovereign",
  corporate: "Corporate",
  "asset-manager": "Asset manager",
  "registered-fund": "Registered fund",
};

export const ORG_STATUS_LABELS: Record<OrgStatus, string> = {
  private: "Private",
  public: "Public",
  subsidiary: "Subsidiary",
  acquired: "Acquired",
  closed: "Closed",
};

export const MEASURE_LABELS: Record<Measure, string> = {
  equity_raised: "Equity raised",
  debt_raised: "Debt raised",
  secondary_sale: "Secondary sale",
  ipo_proceeds: "IPO proceeds",
  valuation_post: "Post-money valuation",
  valuation_pre: "Pre-money valuation",
  equity_investment: "Equity investment",
  investment_commitment: "Investment commitment",
  cumulative_investment: "Invested to date",
  contract_value: "Contract value",
  compute_commitment: "Compute commitment",
  purchase_price: "Purchase price",
  capex_actual: "Cash paid for property and equipment",
  capex_guidance: "Capex guidance",
  revenue_reported: "Revenue",
  revenue_run_rate: "Run-rate revenue",
  form_d_amount_sold: "Amount sold (Form D)",
  form_d_offering_amount: "Offering amount (Form D)",
  fund_mark: "Fund mark",
  estimated_capital_cost: "Estimated capital cost",
  price_per_share: "Price per share",
};

export const AMOUNT_STATUS_LABELS: Record<AmountStatus, string> = {
  announced: "Announced",
  signed: "Signed",
  closed: "Closed",
  filed: "Filed",
  reported: "Reported",
  estimated: "Estimate",
  undisclosed: "Undisclosed",
};

export const EVENT_STATUS_LABELS: Record<EventStatus, string> = {
  announced: "Announced",
  signed: "Signed",
  closed: "Closed",
  filed: "Filed",
  completed: "Completed",
  terminated: "Terminated",
};

export const EVENT_KIND_LABELS: Record<EventKind, string> = {
  financing: "Financing",
  deal: "Deal",
  acquisition: "Acquisition",
  ipo: "IPO",
  filing: "Filing",
};

export const EVENT_TYPE_LABELS: Record<EventType, string> = {
  equity_round: "Equity round",
  strategic_investment: "Strategic investment",
  debt_facility: "Debt financing",
  tender_offer: "Tender offer",
  ipo: "IPO",
  registration: "Registration statement",
  form_d: "Form D",
  compute_contract: "Compute contract",
  compute_commitment: "Compute commitment",
  datacenter_lease: "Data-center lease",
  chip_supply: "Chip supply",
  ppa: "Power purchase agreement",
  joint_venture: "Joint venture",
  acquisition: "Acquisition",
  asset_sale: "Asset sale",
  warrant: "Warrant",
};

export const ROLE_LABELS: Record<Role, string> = {
  issuer: "Issuer",
  lead_investor: "Lead",
  co_lead: "Co-lead",
  investor: "Investor",
  strategic_investor: "Strategic",
  buyer: "Buyer",
  provider: "Provider",
  lessor: "Landlord",
  lessee: "Tenant",
  acquirer: "Acquirer",
  target: "Target",
  seller: "Seller",
  lender: "Lender",
  borrower: "Borrower",
  filer: "Filer",
  partner: "Partner",
};

export const QUANTITY_UNITS: Record<QuantityMeasure, { unit: string; label: string }> = {
  it_load_mw: { unit: "MW", label: "critical IT load" },
  gross_power_mw: { unit: "MW", label: "gross power" },
  capacity_gw: { unit: "GW", label: "capacity" },
  gpus: { unit: "GPUs", label: "GPUs" },
  h100e: { unit: "H100e", label: "H100 equivalents" },
  shares: { unit: "shares", label: "shares" },
  investors: { unit: "investors", label: "investors" },
};

/** Short symbol and plain words for a basis. Symbols keep tables dense; words go in titles and screen-reader text. */
export const BASIS_TEXT: Record<Basis, { symbol: string; words: string }> = {
  exact: { symbol: "", words: "" },
  approximately: { symbol: "≈", words: "about" },
  up_to: { symbol: "≤", words: "up to" },
  up_to_approximately: { symbol: "≤≈", words: "up to about" },
  more_than: { symbol: ">", words: "more than" },
  at_least: { symbol: "≥", words: "at least" },
  range: { symbol: "", words: "between" },
};

/**
 * The short tag used in event feeds. Text labels, not colors, carry the type.
 * The first matching type wins, so list the most specific first.
 */
const FEED_TAGS: Array<[EventType, string]> = [
  ["equity_round", "Round"],
  ["ipo", "IPO"],
  ["debt_facility", "Debt"],
  ["tender_offer", "Tender"],
  ["form_d", "Filing"],
  ["registration", "Filing"],
  ["datacenter_lease", "Lease"],
  ["chip_supply", "Compute"],
  ["compute_contract", "Compute"],
  ["compute_commitment", "Compute"],
  ["ppa", "Power"],
  ["acquisition", "M&A"],
  ["asset_sale", "Deal"],
  ["joint_venture", "Deal"],
  ["strategic_investment", "Investment"],
  ["warrant", "Deal"],
];

export function feedTag(types: readonly EventType[]): string {
  for (const [type, tag] of FEED_TAGS) if (types.includes(type)) return tag;
  return "Deal";
}
