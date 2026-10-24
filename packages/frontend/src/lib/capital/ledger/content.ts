import capexJson from "@/content/capital/metrics/capex.json";
import { EVENT_FILES, ORG_FILES, STATEMENT_FILES } from "./contentIndex.generated";
import {
  PRIMARY_SOURCE_TYPES,
  capexFileSchema,
  eventFileSchema,
  orgsFileSchema,
  statementsFileSchema,
  type CapexFile,
  type CapitalEvent,
  type Org,
  type Source,
  type Statement,
} from "./schema";

/**
 * The validated Capital v2 ledger.
 *
 * Validation runs once at module load. A schema error or a broken rule throws a
 * LedgerContentError, so `next build` and the test suite fail before a page can
 * render a wrong or unsourced number.
 */
export interface LedgerContent {
  orgs: Map<string, Org>;
  sources: Map<string, Source>;
  events: Map<string, CapitalEvent>;
  statements: Statement[];
  capex: CapexFile;
}

export interface RawLedgerContent {
  orgFiles: ReadonlyArray<{ file: string; data: unknown }>;
  eventFiles: ReadonlyArray<{ file: string; data: unknown }>;
  statementFiles: ReadonlyArray<{ file: string; data: unknown }>;
  capex: unknown;
}

export class LedgerContentError extends Error {
  constructor(message: string) {
    super(`[capital ledger] ${message}`);
    this.name = "LedgerContentError";
  }
}

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new LedgerContentError(message);
}

function parse<T>(schema: { safeParse: (v: unknown) => { success: true; data: T } | { success: false; error: { issues: Array<{ path: (string | number)[]; message: string }> } } }, data: unknown, file: string): T {
  const result = schema.safeParse(data);
  if (!result.success) {
    const issue = result.error.issues[0];
    throw new LedgerContentError(`${file}: ${issue.path.join(".")}: ${issue.message}`);
  }
  return result.data;
}

/** Deep equality for plain JSON values, used to allow one source to be repeated verbatim in several files. */
function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a, Object.keys(a as object).sort()) === JSON.stringify(b, Object.keys(b as object).sort());
}

/** Phrases that mark a rumor or an unfinished negotiation. Such text never becomes a published number. */
const RUMOR = /\b(in talks|in discussions|people familiar|according to (?:people|sources|a person)|is said to|plans to raise|considering)\b/i;

export function validateLedgerContent(raw: RawLedgerContent): LedgerContent {
  const sources = new Map<string, Source>();
  const addSource = (source: Source, file: string) => {
    const existing = sources.get(source.id);
    if (existing) {
      check(sameJson(existing, source), `${file}: source ${source.id} is defined twice with different fields`);
      return;
    }
    sources.set(source.id, source);
  };

  const orgs = new Map<string, Org>();
  const orgSlugs = new Set<string>();
  for (const { file, data } of raw.orgFiles) {
    const parsed = parse(orgsFileSchema, data, file);
    parsed.sources.forEach((s) => addSource(s, file));
    for (const org of parsed.orgs) {
      check(!orgs.has(org.id), `${file}: duplicate org id ${org.id}`);
      check(!orgSlugs.has(org.slug), `${file}: duplicate org slug ${org.slug}`);
      check(org.id === `org_${org.slug.replace(/-/g, "_")}`, `${file}: org id ${org.id} does not match slug ${org.slug}`);
      orgs.set(org.id, org);
      orgSlugs.add(org.slug);
    }
  }
  check(orgs.size > 0, "no orgs loaded");

  const events = new Map<string, CapitalEvent>();
  const eventSlugs = new Set<string>();
  for (const { file, data } of raw.eventFiles) {
    const parsed = parse(eventFileSchema, data, file);
    parsed.sources.forEach((s) => addSource(s, file));
    const event = parsed.event;
    check(file === `${event.slug}.json`, `${file}: file name must be the event slug (${event.slug}.json)`);
    check(!events.has(event.id), `${file}: duplicate event id ${event.id}`);
    check(!eventSlugs.has(event.slug), `${file}: duplicate event slug ${event.slug}`);
    events.set(event.id, event);
    eventSlugs.add(event.slug);
  }

  const statements: Statement[] = [];
  const statementIds = new Set<string>();
  for (const { file, data } of raw.statementFiles) {
    const parsed = parse(statementsFileSchema, data, file);
    parsed.sources.forEach((s) => addSource(s, file));
    for (const statement of parsed.statements) {
      check(!statementIds.has(statement.id), `${file}: duplicate statement id ${statement.id}`);
      statementIds.add(statement.id);
      statements.push(statement);
    }
  }

  const capex = parse(capexFileSchema, raw.capex, "metrics/capex.json");

  // ---- sources
  for (const source of sources.values()) {
    if (source.type === "sec_filing") {
      check(source.filing, `source ${source.id}: an SEC filing needs its form, accession, CIK and filing date`);
      check(new URL(source.url).hostname === "www.sec.gov", `source ${source.id}: SEC filings must link to www.sec.gov`);
      check(source.url.includes(source.filing.accession.replace(/-/g, "")), `source ${source.id}: URL does not contain accession ${source.filing.accession}`);
    }
    if (source.type === "dataset" && source.license === undefined) {
      throw new LedgerContentError(`source ${source.id}: a dataset needs a license`);
    }
    check(!source.publishedAt || source.publishedAt <= source.retrievedAt, `source ${source.id}: published after it was retrieved`);
  }

  const sourceOf = (id: string, where: string) => {
    const source = sources.get(id);
    check(source, `${where}: unknown source ${id}`);
    return source;
  };

  // A number that cites news is "reported" and names the outlet; a "reported" number cites news.
  const checkStatusAgainstSource = (status: string, sourceIdValue: string, where: string) => {
    const source = sourceOf(sourceIdValue, where);
    if (source.type === "news") check(status === "reported", `${where}: a number from news must have status "reported"`);
    if (status === "reported") check(source.type === "news", `${where}: status "reported" is only for numbers from news sources`);
  };

  // ---- orgs
  for (const org of orgs.values()) {
    check(!(org.kinds.includes("company") && org.categories.length === 0), `org ${org.id}: a company needs a category`);
    check(!(org.kinds.includes("investor") && org.investorTypes.length === 0), `org ${org.id}: an investor needs an investor type`);
    if (org.parent) {
      check(orgs.has(org.parent.org), `org ${org.id}: unknown parent ${org.parent.org}`);
      sourceOf(org.parent.sourceId, `org ${org.id} parent`);
      check(org.status === "subsidiary" || org.status === "acquired", `org ${org.id}: has a parent but status ${org.status}`);
    }
    if (org.founded) sourceOf(org.founded.sourceId, `org ${org.id} founded`);
    check(!(org.status === "public" && org.ids.tickers.length === 0 && !org.statusNote), `org ${org.id}: public without a ticker or a note`);
  }
  const labOwners = new Map<string, string>();
  for (const org of orgs.values()) {
    for (const lab of org.labSlugs) {
      check(!labOwners.has(lab), `lab slug ${lab} belongs to both ${labOwners.get(lab)} and ${org.id}`);
      labOwners.set(lab, org.id);
    }
  }

  // ---- events
  for (const event of events.values()) {
    const where = `event ${event.id}`;
    check(event.id === `evt_${event.slug.replace(/-/g, "_")}`, `${where}: id must be evt_ + slug with underscores (${event.slug})`);
    check(Object.values(event.dates).includes(event.date), `${where}: date ${event.date} is not one of its dates`);
    check(!RUMOR.test(event.title) && !RUMOR.test(event.summary), `${where}: rumor wording in the title or summary`);
    check(event.reviewedAt >= event.date, `${where}: reviewed before the event date`);

    const participantOrgs = new Set<string>();
    for (const p of event.participants) {
      check(orgs.has(p.org), `${where}: unknown participant ${p.org}`);
      check(!participantOrgs.has(p.org), `${where}: ${p.org} listed twice; give one entry several roles instead`);
      participantOrgs.add(p.org);
    }
    for (const id of event.sourceIds) sourceOf(id, where);
    const cited = new Set(event.sourceIds);

    const amountIds = new Set<string>();
    for (const amount of event.amounts) {
      const at = `${where} amount ${amount.id}`;
      check(!amountIds.has(amount.id), `${at}: duplicate amount id`);
      amountIds.add(amount.id);
      check(cited.has(amount.sourceId), `${at}: source ${amount.sourceId} is not in the event's sourceIds`);
      check((amount.value === null) === (amount.status === "undisclosed"), `${at}: value is null if and only if the status is "undisclosed"`);
      checkStatusAgainstSource(amount.status, amount.sourceId, at);
      if (amount.from) check(participantOrgs.has(amount.from), `${at}: from ${amount.from} is not a participant`);
      if (amount.to) check(participantOrgs.has(amount.to), `${at}: to ${amount.to} is not a participant`);
      check(!amount.term?.through || amount.term.through >= event.date, `${at}: term ends before the event`);
    }
    for (const amount of event.amounts) {
      if (amount.alternativeTo) {
        const base = event.amounts.find((a) => a.id === amount.alternativeTo && a.id !== amount.id);
        check(base, `${where} amount ${amount.id}: alternativeTo ${amount.alternativeTo} is not another amount of this event`);
        check(base.measure === amount.measure && base.currency === amount.currency, `${where} amount ${amount.id}: an alternative figure must have the measure and currency of ${base.id}`);
        check(Boolean(amount.condition), `${where} amount ${amount.id}: an alternative figure needs the condition that makes it apply`);
      }
      if (amount.partOf) {
        check(amount.partOf !== amount.id && amountIds.has(amount.partOf), `${where} amount ${amount.id}: partOf ${amount.partOf} is not another amount of this event`);
        const whole = event.amounts.find((a) => a.id === amount.partOf)!;
        check(whole.currency === amount.currency, `${where} amount ${amount.id}: part and whole use different currencies`);
        check(whole.value === null || amount.value === null || amount.value <= whole.value, `${where} amount ${amount.id}: part is larger than the whole`);
      }
    }
    const quantityIds = new Set<string>();
    for (const quantity of event.quantities) {
      check(!quantityIds.has(quantity.id), `${where} quantity ${quantity.id}: duplicate id`);
      quantityIds.add(quantity.id);
      check(cited.has(quantity.sourceId), `${where} quantity ${quantity.id}: source ${quantity.sourceId} is not in the event's sourceIds`);
    }
    for (const fact of event.facts) {
      check(cited.has(fact.sourceId), `${where} fact "${fact.label}": source ${fact.sourceId} is not in the event's sourceIds`);
    }
    for (const relation of event.relations) {
      check(relation.event !== event.id, `${where}: relation to itself`);
      check(events.has(relation.event), `${where}: relation to unknown event ${relation.event}`);
    }
  }

  // "Included in" links are two-way: the amount names the later event, and that event says it includes this one.
  for (const event of events.values()) {
    for (const amount of event.amounts) {
      if (!amount.includedIn) continue;
      const at = `event ${event.id} amount ${amount.id}`;
      const container = events.get(amount.includedIn);
      check(container, `${at}: includedIn unknown event ${amount.includedIn}`);
      check(container.id !== event.id, `${at}: includedIn its own event`);
      check(
        container.relations.some((r) => r.type === "includes" && r.event === event.id),
        `${at}: ${container.id} must list an "includes" relation to ${event.id}`
      );
    }
  }

  // ---- statements
  for (const statement of statements) {
    const at = `statement ${statement.id}`;
    check(orgs.has(statement.org), `${at}: unknown org ${statement.org}`);
    checkStatusAgainstSource(statement.status, statement.sourceId, at);
    if (statement.period) check(statement.period.start <= statement.period.end, `${at}: period starts after it ends`);
  }

  // ---- capex
  const capexOrgs = new Set<string>();
  for (const series of capex.series) {
    check(orgs.has(series.org), `capex: unknown org ${series.org}`);
    check(!capexOrgs.has(series.org), `capex: ${series.org} listed twice`);
    capexOrgs.add(series.org);
    check(orgs.get(series.org)!.ids.secCik.includes(series.cik), `capex: ${series.org} CIK ${series.cik} is not in orgs.json`);
    for (const fact of series.facts) check(fact.start < fact.end, `capex: ${series.org} fact ${fact.start}..${fact.end} is backwards`);
  }

  return { orgs, sources, events, statements, capex };
}

/** True when at least one cited source is primary evidence (not news, not a dataset). */
export function hasPrimarySource(sourceIds: string[], sources: Map<string, Source>): boolean {
  return sourceIds.some((id) => {
    const source = sources.get(id);
    return source ? PRIMARY_SOURCE_TYPES.has(source.type) : false;
  });
}

export const ledgerContent: LedgerContent = validateLedgerContent({
  orgFiles: ORG_FILES,
  eventFiles: EVENT_FILES,
  statementFiles: STATEMENT_FILES,
  capex: capexJson,
});
