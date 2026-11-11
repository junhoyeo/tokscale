/**
 * Validate the Capital v2 ledger content (orgs, events, statements, capex) and print a summary.
 *
 *   bun scripts/capital/validate.ts
 *
 * The same validation runs at module load in the app, so `next build` and `bun run test`
 * fail on the same errors. This script is a fast check while editing JSON.
 */
import { ledgerContent } from "@/lib/capital/ledger/content";

const { orgs, events, sources, statements, capex } = ledgerContent;
console.log(
  `OK: ${orgs.size} orgs, ${events.size} events, ${sources.size} sources, ${statements.length} statements, ${capex.series.length} capex series.`
);
