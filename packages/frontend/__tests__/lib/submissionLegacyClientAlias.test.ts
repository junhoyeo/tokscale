import { describe, expect, it } from "vitest";
import { validateSubmission } from "@/lib/validation/submission";

// Guards the cross-layer invariant the alias-fold heal path depends on:
// a legacy client that still emits "kilocode" on the wire is canonicalized to
// "kilo" during validation (normalizeLegacySources, wired as a z.preprocess on
// SubmissionDataSchema), BEFORE the submit route builds incomingClientBreakdown
// from client_contrib.client. Because of this, the route's incoming breakdown
// and submitted-client set can never carry a raw "kilocode" key, so the merge
// guard always evaluates the incoming value against the folded canonical "kilo"
// entry and healing works for legacy clients too. (This is why cubic's P1 on
// #945 — "legacy kilocode resubmits cannot heal a folded kilo total" — does not
// apply: the incoming value is already keyed on "kilo" by the time the merge
// runs.) If the preprocess is ever removed or reordered, this test fails loudly
// instead of silently reintroducing the un-healable double count.
function buildKilocodeSubmission(): Record<string, unknown> {
  const tokens = {
    input: 100,
    output: 100,
    cacheRead: 100,
    cacheWrite: 0,
    reasoning: 0,
  };
  return {
    meta: {
      generatedAt: "2026-07-14T00:00:00.000Z",
      version: "4.5.3",
      dateRange: { start: "2026-05-11", end: "2026-05-11" },
    },
    summary: {
      totalTokens: 300,
      totalCost: 1.5,
      totalDays: 1,
      activeDays: 1,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: ["kilocode"],
      models: ["glm-4.6"],
    },
    years: [
      {
        year: "2026",
        totalTokens: 300,
        totalCost: 1.5,
        range: { start: "2026-05-11", end: "2026-05-11" },
      },
    ],
    contributions: [
      {
        date: "2026-05-11",
        totals: { tokens: 300, cost: 1.5, messages: 0 },
        intensity: 4,
        tokenBreakdown: tokens,
        clients: [{ client: "kilocode", modelId: "glm-4.6", tokens, cost: 1.5, messages: 0 }],
      },
    ],
  };
}

describe("legacy kilocode client alias canonicalization", () => {
  it("rewrites a wire-level 'kilocode' client to 'kilo' in both summary and contributions", () => {
    const result = validateSubmission(buildKilocodeSubmission());

    expect(result.valid).toBe(true);
    expect(result.data).toBeDefined();

    // Summary client set is canonical.
    expect(result.data!.summary.clients).toContain("kilo");
    expect(result.data!.summary.clients).not.toContain("kilocode");

    // The per-day contribution client — the exact value the submit route reads
    // to build incomingClientBreakdown — is canonical too.
    const contribClients = result.data!.contributions[0].clients.map((c) => c.client);
    expect(contribClients).toContain("kilo");
    expect(contribClients).not.toContain("kilocode");
  });
});
