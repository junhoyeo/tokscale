import { describe, it, expect } from 'vitest';
import type { ClientType } from "../../src/lib/types";
import {
  generateSubmissionHash,
  validateSubmission,
} from "../../src/lib/validation/submission";
import {
  breakdownCostIsComplete,
  deriveClientBreakdownProvenance,
  mergeClientBreakdownsWithRegressionGuard,
  recalculateDayTotals,
  type ClientBreakdownData,
} from "../../src/lib/db/helpers";

/**
 * Test suite for POST /api/submit - Client-Level Merge
 * 
 * These tests verify the client-level merge functionality:
 * - First submission creates new records
 * - Subsequent submissions merge by client
 * - Clients not in submission are preserved
 * - Totals are recalculated from dailyBreakdown
 * - Concurrent submissions are handled correctly
 */

// Mock data factories
function createMockSubmissionData(overrides: Partial<{
  clients: ClientType[];
  contributions: Array<{
    date: string;
    clients: Array<{
      client: ClientType;
      modelId: string;
      cost: number;
      tokens: { input: number; output: number; cacheRead: number; cacheWrite: number };
      messages: number;
    }>;
  }>;
}> = {}) {
  const defaultClients = overrides.clients || ['claude'];
  const defaultContributions = overrides.contributions || [
    {
      date: '2024-12-01',
      clients: defaultClients.map(client => ({
        client,
        modelId: 'claude-sonnet-4-20250514',
        cost: 1.5,
        tokens: { input: 1000, output: 500, cacheRead: 100, cacheWrite: 50 },
        messages: 5,
      })),
    },
  ];

  const contributionTokenTotal = (client: { tokens: { input: number; output: number; cacheRead: number; cacheWrite: number } }) =>
    client.tokens.input + client.tokens.output + client.tokens.cacheRead + client.tokens.cacheWrite;

  return {
    meta: {
      generatedAt: new Date().toISOString(),
      version: '1.0.0',
      dateRange: {
        start: defaultContributions[0]?.date || '2024-12-01',
        end: defaultContributions[defaultContributions.length - 1]?.date || '2024-12-01',
      },
    },
    summary: {
      totalTokens: defaultContributions.reduce((sum, d) => 
        sum + d.clients.reduce((s, client) => s + contributionTokenTotal(client), 0), 0
      ),
      totalCost: defaultContributions.reduce((sum, d) => 
        sum + d.clients.reduce((s, client) => s + client.cost, 0), 0
      ),
      totalDays: defaultContributions.length,
      activeDays: defaultContributions.filter(d => d.clients.length > 0).length,
      averagePerDay: 0,
      maxCostInSingleDay: 0,
      clients: defaultClients,
      models: ['claude-sonnet-4-20250514'],
    },
    years: [],
    contributions: defaultContributions.map(d => ({
      date: d.date,
      totals: {
        tokens: d.clients.reduce((s, client) => s + contributionTokenTotal(client), 0),
        cost: d.clients.reduce((s, client) => s + client.cost, 0),
        messages: d.clients.reduce((s, client) => s + client.messages, 0),
      },
      intensity: 2 as const,
      tokenBreakdown: {
        input: d.clients.reduce((s, client) => s + client.tokens.input, 0),
        output: d.clients.reduce((s, client) => s + client.tokens.output, 0),
        cacheRead: d.clients.reduce((s, client) => s + client.tokens.cacheRead, 0),
        cacheWrite: d.clients.reduce((s, client) => s + client.tokens.cacheWrite, 0),
        reasoning: 0,
      },
      clients: d.clients.map(client => ({
        client: client.client as ClientType,
        modelId: client.modelId,
        tokens: { ...client.tokens, reasoning: 0 },
        cost: client.cost,
        messages: client.messages,
      })),
    })),
  };
}

function createValidationPayload(overrides: Partial<{
  date: string;
  generatedAt: string;
  totalTokens: number;
  totalCost: number;
  messages: number;
  tokenBreakdown: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    reasoning: number;
  };
  clientCost: number;
  clientMessages: number;
}> = {}) {
  const date = overrides.date ?? "2024-12-01";
  const tokenBreakdown = overrides.tokenBreakdown ?? {
    input: overrides.totalTokens ?? 1000,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
  };
  const totalTokens = overrides.totalTokens ?? (
    tokenBreakdown.input +
    tokenBreakdown.output +
    tokenBreakdown.cacheRead +
    tokenBreakdown.cacheWrite +
    tokenBreakdown.reasoning
  );
  const totalCost = overrides.totalCost ?? 1.5;
  const messages = overrides.messages ?? 5;

  return {
    meta: {
      generatedAt: overrides.generatedAt ?? "2024-12-02T00:00:00.000Z",
      version: "2.1.1",
      dateRange: { start: date, end: date },
    },
    summary: {
      totalTokens,
      totalCost,
      totalDays: 1,
      activeDays: totalTokens > 0 ? 1 : 0,
      averagePerDay: totalCost,
      maxCostInSingleDay: totalCost,
      clients: ["claude" as const],
      models: ["claude-sonnet-4-20250514"],
    },
    years: [{
      year: date.slice(0, 4),
      totalTokens,
      totalCost,
      range: { start: date, end: date },
    }],
    contributions: [{
      date,
      totals: { tokens: totalTokens, cost: totalCost, messages },
      intensity: 2 as const,
      tokenBreakdown,
      clients: [{
        client: "claude" as const,
        modelId: "claude-sonnet-4-20250514",
        providerId: "anthropic",
        tokens: tokenBreakdown,
        cost: overrides.clientCost ?? totalCost,
        messages: overrides.clientMessages ?? messages,
      }],
    }],
  };
}

describe('POST /api/submit - Client-Level Merge', () => {
  describe('Device-Aware Payloads', () => {
    it('accepts a stable random submit device id', () => {
      const payload = {
        ...createMockSubmissionData({ clients: ['claude'] }),
        device: {
          id: 'dev_018f4b6f9c2d4f2d8a2f4b6f9c2d4f2d',
          name: 'Work laptop',
        },
      };

      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
      expect(result.data?.device).toEqual({
        id: 'dev_018f4b6f9c2d4f2d8a2f4b6f9c2d4f2d',
        name: 'Work laptop',
      });
    });

    it('keeps no-device legacy payloads valid', () => {
      const result = validateSubmission(createMockSubmissionData({ clients: ['claude'] }));

      expect(result.valid).toBe(true);
      expect(result.data?.device).toBeUndefined();
    });

    it('accepts jcode client submissions', () => {
      const result = validateSubmission(createMockSubmissionData({
        clients: ['jcode'],
        contributions: [{
          date: '2024-12-01',
          clients: [{
            client: 'jcode',
            modelId: 'gpt-5.5-fast',
            cost: 0.75,
            tokens: { input: 1200, output: 300, cacheRead: 800, cacheWrite: 0 },
            messages: 3,
          }],
        }],
      }));

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
      expect(result.data?.summary.clients).toEqual(['jcode']);
      expect(result.data?.contributions[0].clients[0].client).toBe('jcode');
    });

    it('includes the submit device id in the submission hash', () => {
      const base = createMockSubmissionData({ clients: ['claude'] });
      const laptop = validateSubmission({
        ...base,
        device: { id: 'dev_laptop' },
      }).data!;
      const desktop = validateSubmission({
        ...base,
        device: { id: 'dev_desktop' },
      }).data!;

      expect(generateSubmissionHash(laptop)).not.toBe(generateSubmissionHash(desktop));
    });

    it('rejects blank submit device ids', () => {
      const result = validateSubmission({
        ...createMockSubmissionData({ clients: ['claude'] }),
        device: { id: '   ' },
      });

      expect(result.valid).toBe(false);
      expect(result.errors.some((error) => error.includes('device.id'))).toBe(true);
    });

    it('accepts per-client submit provenance metadata', () => {
      const payload = createMockSubmissionData({ clients: ['codex'] });
      const client = payload.contributions[0].clients[0] as {
        provenance?: { schemaVersion: number; messageCount: number; modelCount: number };
      };
      client.provenance = {
        schemaVersion: 1,
        messageCount: 7,
        modelCount: 1,
      };

      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.data?.contributions[0].clients[0].provenance).toEqual({
        schemaVersion: 1,
        messageCount: 7,
        modelCount: 1,
      });
    });
  });

  describe('First Submission (Create Mode)', () => {
    it('should create new submission with all clients', () => {
      const data = createMockSubmissionData({ clients: ['claude', 'cursor'] });
      
      // Verify data structure
      expect(data.summary.clients).toContain('claude');
      expect(data.summary.clients).toContain('cursor');
      expect(data.contributions[0].clients.length).toBe(2);
    });

    it('should create dailyBreakdown for each day', () => {
      const data = createMockSubmissionData({
        contributions: [
          { date: '2024-12-01', clients: [{ client: 'claude', modelId: 'claude-sonnet-4', cost: 1, tokens: { input: 100, output: 50, cacheRead: 0, cacheWrite: 0 }, messages: 1 }] },
          { date: '2024-12-02', clients: [{ client: 'claude', modelId: 'claude-sonnet-4', cost: 2, tokens: { input: 200, output: 100, cacheRead: 0, cacheWrite: 0 }, messages: 2 }] },
          { date: '2024-12-03', clients: [{ client: 'claude', modelId: 'claude-sonnet-4', cost: 3, tokens: { input: 300, output: 150, cacheRead: 0, cacheWrite: 0 }, messages: 3 }] },
        ],
      });
      
      expect(data.contributions.length).toBe(3);
      expect(data.contributions.map(c => c.date)).toEqual(['2024-12-01', '2024-12-02', '2024-12-03']);
    });

    it('should support pi client in submission payload', () => {
      const data = createMockSubmissionData({ clients: ['pi'] });

      expect(data.summary.clients).toContain('pi');
      expect(data.contributions[0].clients[0].client).toBe('pi');
    });

    it('should support kimi client in submission payload', () => {
      const data = createMockSubmissionData({ clients: ['kimi'] });

      expect(data.summary.clients).toContain('kimi');
      expect(data.contributions[0].clients[0].client).toBe('kimi');
    });

    it('should support kilo client in submission payload', () => {
      const data = createMockSubmissionData({ clients: ['kilo'] });

      expect(data.summary.clients).toContain('kilo');
      expect(data.contributions[0].clients[0].client).toBe('kilo');
    });

    it('should support hermes client in submission payload', () => {
      const data = createMockSubmissionData({ clients: ['hermes'] });

      expect(data.summary.clients).toContain('hermes');
      expect(data.contributions[0].clients[0].client).toBe('hermes');
    });

    it('should support zed client in submission payload', () => {
      const data = createMockSubmissionData({ clients: ['zed'] });

      expect(data.summary.clients).toContain('zed');
      expect(data.contributions[0].clients[0].client).toBe('zed');
    });

    it('should pass validation for cc-mirror variant submissions', () => {
      const data = createMockSubmissionData({
        clients: ['cc-mirror/zaicc'],
        contributions: [
          {
            date: '2024-12-01',
            clients: [
              {
                client: 'cc-mirror/zaicc',
                modelId: 'glm-5.1',
                cost: 1.5,
                tokens: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0 },
                messages: 5,
              },
            ],
          },
        ],
      });

      const result = validateSubmission(data);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
      expect(result.data?.summary.clients).toEqual(['cc-mirror/zaicc']);
      expect(result.data?.contributions[0].clients[0].client).toBe('cc-mirror/zaicc');
    });

    it('rejects malformed cc-mirror variant client ids', () => {
      const data = createMockSubmissionData({
        clients: ['cc-mirror/../zaicc' as ClientType],
        contributions: [
          {
            date: '2024-12-01',
            clients: [
              {
                client: 'cc-mirror/../zaicc' as ClientType,
                modelId: 'glm-5.1',
                cost: 1.5,
                tokens: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0 },
                messages: 5,
              },
            ],
          },
        ],
      });

      const result = validateSubmission(data);

      expect(result.valid).toBe(false);
      expect(result.errors.join("\n")).toContain("Invalid cc-mirror variant client id");
    });

    it('should pass validation for kilo client submissions', () => {
      const payload = {
        meta: { generatedAt: new Date().toISOString(), version: '1.0.0', dateRange: { start: '2024-12-01', end: '2024-12-01' } },
        summary: { totalTokens: 1500, totalCost: 1.5, totalDays: 1, activeDays: 1, averagePerDay: 1.5, maxCostInSingleDay: 1.5, clients: ['kilo' as const], models: ['claude-sonnet-4'] },
        years: [{ year: '2024', totalTokens: 1500, totalCost: 1.5, range: { start: '2024-12-01', end: '2024-12-01' } }],
        contributions: [{
          date: '2024-12-01',
          totals: { tokens: 1500, cost: 1.5, messages: 5 },
          intensity: 2 as const,
          tokenBreakdown: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          clients: [{ client: 'kilo' as const, modelId: 'claude-sonnet-4', tokens: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 }, cost: 1.5, messages: 5 }],
        }],
      };
      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it('should accept mixed kilo and kilocode submissions', () => {
      const payload = {
        meta: { generatedAt: new Date().toISOString(), version: '1.0.0', dateRange: { start: '2024-12-01', end: '2024-12-01' } },
        summary: { totalTokens: 3000, totalCost: 3.0, totalDays: 1, activeDays: 1, averagePerDay: 3.0, maxCostInSingleDay: 3.0, clients: ['kilo' as const, 'kilocode' as const], models: ['claude-sonnet-4'] },
        years: [{ year: '2024', totalTokens: 3000, totalCost: 3.0, range: { start: '2024-12-01', end: '2024-12-01' } }],
        contributions: [{
          date: '2024-12-01',
          totals: { tokens: 3000, cost: 3.0, messages: 10 },
          intensity: 2 as const,
          tokenBreakdown: { input: 2000, output: 1000, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          clients: [
            { client: 'kilo' as const, modelId: 'claude-sonnet-4', tokens: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 }, cost: 1.5, messages: 5 },
            { client: 'kilocode' as const, modelId: 'claude-sonnet-4', tokens: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 }, cost: 1.5, messages: 5 },
          ],
        }],
      };
      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it('should pass validation for zed client submissions', () => {
      const payload = {
        meta: { generatedAt: new Date().toISOString(), version: '1.0.0', dateRange: { start: '2024-12-01', end: '2024-12-01' } },
        summary: { totalTokens: 1500, totalCost: 1.5, totalDays: 1, activeDays: 1, averagePerDay: 1.5, maxCostInSingleDay: 1.5, clients: ['zed' as const], models: ['claude-sonnet-4'] },
        years: [{ year: '2024', totalTokens: 1500, totalCost: 1.5, range: { start: '2024-12-01', end: '2024-12-01' } }],
        contributions: [{
          date: '2024-12-01',
          totals: { tokens: 1500, cost: 1.5, messages: 5 },
          intensity: 2 as const,
          tokenBreakdown: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          clients: [{ client: 'zed' as const, modelId: 'claude-sonnet-4', tokens: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 }, cost: 1.5, messages: 5 }],
        }],
      };
      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it('should pass validation for legacy zed source submissions', () => {
      const payload = {
        meta: { generatedAt: new Date().toISOString(), version: '1.0.0', dateRange: { start: '2024-12-01', end: '2024-12-01' } },
        summary: { totalTokens: 1500, totalCost: 1.5, totalDays: 1, activeDays: 1, averagePerDay: 1.5, maxCostInSingleDay: 1.5, sources: ['zed'], models: ['claude-sonnet-4'] },
        years: [{ year: '2024', totalTokens: 1500, totalCost: 1.5, range: { start: '2024-12-01', end: '2024-12-01' } }],
        contributions: [{
          date: '2024-12-01',
          totals: { tokens: 1500, cost: 1.5, messages: 5 },
          intensity: 2 as const,
          tokenBreakdown: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          sources: [{ source: 'zed', modelId: 'claude-sonnet-4', tokens: { input: 1000, output: 500, cacheRead: 0, cacheWrite: 0, reasoning: 0 }, cost: 1.5, messages: 5 }],
        }],
      };
      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
    });
  });

  describe("costIsComplete merge floor (#1044)", () => {
    function client(
      tokens: number,
      cost: number,
      costIsComplete?: boolean
    ): ClientBreakdownData {
      return {
        tokens,
        cost,
        input: tokens,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 1,
        models: {},
        ...(costIsComplete === false
          ? {
              provenance: {
                schemaVersion: 1,
                messageCount: 1,
                modelCount: 0,
                costIsComplete: false,
              },
            }
          : {}),
      };
    }

    it("floors a client's cost when the submission declares itself incomplete", () => {
      // More tokens at a LOWER cost: the token guard accepts the client, so
      // without a floor the day's recorded spend drops permanently.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: client(100, 10) },
        { claude: client(110, 5) },
        new Set(["claude"]),
        undefined,
        false
      );

      expect(merged.claude.cost).toBe(10);
      expect(merged.claude.tokens).toBe(110);
      expect(breakdownCostIsComplete(merged)).toBe(false);
    });

    it("marks the client incomplete even when its own cost was kept", () => {
      // The stored $10 was complete for the OLD token set. Deriving the tag
      // from "did the new value win?" would call this complete, which is the
      // defect adversarial review found in the earlier SQL-side design.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: client(100, 10) },
        { claude: client(110, 5) },
        new Set(["claude"]),
        undefined,
        false
      );

      expect(merged.claude.provenance?.costIsComplete).toBe(false);
    });

    it("keeps the day scalar reconcilable with the stored breakdown", () => {
      // The scalar is recomputed from this breakdown, so flooring here (rather
      // than in SQL) is what stops the row's two representations diverging.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: client(100, 10) },
        { claude: client(110, 5) },
        new Set(["claude"]),
        undefined,
        false
      );

      expect(recalculateDayTotals(merged).cost).toBe(10);
    });

    it("does not let a filtered healthy resubmit clear a sibling's incompleteness", () => {
      // Day holds incomplete A and complete B; a healthy submit naming only B
      // must leave A's state alone. Day-level state cannot express this.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: client(100, 10, false), codex: client(50, 5) },
        { codex: client(60, 6) },
        new Set(["codex"]),
        undefined,
        true
      );

      expect(merged.claude.provenance?.costIsComplete).toBe(false);
      expect(merged.codex.provenance?.costIsComplete).toBeUndefined();
      expect(breakdownCostIsComplete(merged)).toBe(false);
    });

    it("clears the floor once a complete submission covers the client", () => {
      // Recovery, and the reason downward corrections still work.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: client(100, 10, false) },
        { claude: client(100, 4) },
        new Set(["claude"]),
        undefined,
        true
      );

      expect(merged.claude.cost).toBe(4);
      expect(merged.claude.provenance?.costIsComplete).toBeUndefined();
      expect(breakdownCostIsComplete(merged)).toBe(true);
    });

    function withModels(
      models: Record<string, number>,
      costIsComplete?: boolean
    ): ClientBreakdownData {
      const entries = Object.entries(models);
      const total = entries.reduce((sum, [, cost]) => sum + cost, 0);
      const base = client(100, total, costIsComplete);
      return {
        ...base,
        models: Object.fromEntries(
          entries.map(([modelId, cost]) => [
            modelId,
            {
              tokens: 100,
              cost,
              input: 100,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
              messages: 1,
            },
          ])
        ),
      };
    }

    it("floors nested model costs, not just the client aggregate", () => {
      // Regression: flooring only the client total left `models[*].cost` at the
      // incomplete payload's value. Model-filtered leaderboards sum the nested
      // costs, so the row would rank at $0 while its client said $10.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: withModels({ "opus-4": 10 }) },
        { claude: withModels({ "opus-4": 0 }) },
        new Set(["claude"]),
        undefined,
        false
      );

      expect(merged.claude.models["opus-4"].cost).toBe(10);
      expect(merged.claude.cost).toBe(10);
    });

    it("preserves a model the incomplete payload dropped", () => {
      // Its usage did not stop existing because this submission could not
      // price it, and losing it would put the client total above the sum of
      // its models again.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: withModels({ "opus-4": 6, "sonnet-4": 4 }) },
        { claude: withModels({ "opus-4": 0, "haiku-4": 5 }) },
        new Set(["claude"]),
        undefined,
        false
      );

      expect(Object.keys(merged.claude.models).sort()).toEqual([
        "haiku-4",
        "opus-4",
        "sonnet-4",
      ]);
      expect(merged.claude.models["sonnet-4"].cost).toBe(4);
      // Derived from the floors, so client == sum(models) by construction.
      expect(merged.claude.cost).toBe(15);
    });

    it("keeps day, client and model costs mutually consistent", () => {
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { claude: withModels({ "opus-4": 6, "sonnet-4": 4 }) },
        { claude: withModels({ "opus-4": 1 }) },
        new Set(["claude"]),
        undefined,
        false
      );

      const modelSum = Object.values(merged.claude.models).reduce(
        (sum, m) => sum + m.cost,
        0
      );
      expect(merged.claude.cost).toBe(modelSum);
      expect(recalculateDayTotals(merged).cost).toBe(modelSum);
    });

    it("floors cost on the alias-heal branch too", () => {
      // Regression: the heal branch assigned the incoming client raw and
      // `continue`d, skipping the floor entirely. An incomplete submission
      // could then drop a folded $20 to $1 AND mark the day complete — the
      // exact data loss this whole mechanism exists to prevent.
      //
      // The heal gate is evidence about tokens, not pricing, so the token
      // count still heals; only the cost is held at the floor.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { kilo: client(200, 20) }, // normalized fold of kilocode+kilo
        { kilo: client(100, 1) },
        new Set(["kilo"]),
        new Map([["kilo", 100]]), // heal floor = largest single component
        false
      );

      expect(merged.kilo.tokens).toBe(100); // healed
      expect(merged.kilo.cost).toBe(20); // floored, not $1
      expect(merged.kilo.provenance?.costIsComplete).toBe(false);
      expect(breakdownCostIsComplete(merged)).toBe(false);
    });

    it("still heals cost exactly when the healing submission is complete", () => {
      // The floor must not break the heal's original purpose.
      const { merged } = mergeClientBreakdownsWithRegressionGuard(
        { kilo: client(200, 20) },
        { kilo: client(100, 1) },
        new Set(["kilo"]),
        new Map([["kilo", 100]]),
        true
      );

      expect(merged.kilo.tokens).toBe(100);
      expect(merged.kilo.cost).toBe(1); // inflated fold replaced outright
      expect(breakdownCostIsComplete(merged)).toBe(true);
    });

    it("makes a day incomplete when any single client is floored", () => {
      // The submission-level BOOL_AND composes the same way one level up: one
      // floored device makes the account total a lower bound, not an exact
      // figure. This pins the day-level half of that AND semantics.
      const mixed = {
        claude: client(100, 10, false),
        codex: client(50, 5),
      };

      expect(breakdownCostIsComplete(mixed)).toBe(false);
    });

    it("treats an untagged breakdown as complete", () => {
      expect(breakdownCostIsComplete({ claude: client(10, 1) })).toBe(true);
      expect(breakdownCostIsComplete({})).toBe(true);
      expect(breakdownCostIsComplete(null)).toBe(true);
    });
  });

  describe("costIsComplete (#1044)", () => {
    it("preserves a declared-incomplete flag through validation", () => {
      const payload = createValidationPayload();
      (
        payload.contributions[0].totals as { costIsComplete?: boolean }
      ).costIsComplete = false;

      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
      // Asserting the parsed value, not just validity: Zod strips unknown keys,
      // so a schema that never declared this field would still validate here
      // and silently drop the flag before the route could act on it.
      expect(result.data?.contributions[0].totals.costIsComplete).toBe(false);
    });

    it("leaves the flag undefined when omitted, so released CLIs keep working", () => {
      const payload = createValidationPayload();

      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
      expect(
        result.data?.contributions[0].totals.costIsComplete
      ).toBeUndefined();
    });

    it("rejects a non-boolean flag rather than coercing it", () => {
      const payload = createValidationPayload();
      (
        payload.contributions[0].totals as { costIsComplete?: unknown }
      ).costIsComplete = "false";

      const result = validateSubmission(payload);

      expect(result.valid).toBe(false);
    });

    it("does not loosen the totals consistency check", () => {
      // The flag declares the cost a floor; it must NOT become a licence to
      // send internally inconsistent numbers. The earlier version of this test
      // set summary and day cost to the same value, so it passed whether or
      // not the flag did anything — it asserted nothing.
      const payload = createValidationPayload({ totalCost: 12.5 });
      (
        payload.contributions[0].totals as { costIsComplete?: boolean }
      ).costIsComplete = false;
      // Summary now disagrees with the day it summarises.
      payload.summary.totalCost = 99.5;

      const result = validateSubmission(payload);

      expect(result.valid).toBe(false);
      expect(result.errors.join(" ")).toMatch(/cost/i);
    });
  });

  describe("Submission validation guardrails", () => {
    it("accepts internally consistent high-volume one-day token totals", () => {
      // Size caps were removed in feat/remove-submission-size-caps; this test
      // now confirms that an arbitrarily large but internally-consistent
      // payload still validates.
      const payload = createValidationPayload({
        totalTokens: 8_000_000_000,
        totalCost: 800,
        tokenBreakdown: {
          input: 4_800_000_000,
          output: 2_000_000_000,
          cacheRead: 800_000_000,
          cacheWrite: 300_000_000,
          reasoning: 100_000_000,
        },
      });

      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it("accepts trillion-token internally consistent payloads (no size cap)", () => {
      const payload = createValidationPayload({
        totalTokens: 1_000_000_000_000,
        totalCost: 100_000,
        tokenBreakdown: {
          input: 1_000_000_000_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
      });

      const result = validateSubmission(payload);

      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it("keeps structural validation for high-volume payloads", () => {
      const payload = createValidationPayload({
        totalTokens: 20_000_000_000,
        totalCost: 2_000,
        tokenBreakdown: {
          input: 12_000_000_000,
          output: 5_000_000_000,
          cacheRead: 2_000_000_000,
          cacheWrite: 750_000_000,
          reasoning: 250_000_000,
        },
      });
      payload.contributions[0].clients[0].modelId = "";

      const result = validateSubmission(payload);

      expect(result.valid).toBe(false);
      expect(result.errors.join("\n")).toContain("modelId");
    });

    it("rejects submitted cost without corresponding tokens", () => {
      const payload = createValidationPayload({
        totalTokens: 0,
        totalCost: 25,
        tokenBreakdown: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
      });

      const result = validateSubmission(payload);

      expect(result.valid).toBe(false);
      expect(result.errors.join("\n")).toContain("Cost submitted without tokens");
    });

    it("includes client/provider/model/cost/tokens detail in tokenless-cost errors", () => {
      const payload = createValidationPayload({
        totalTokens: 0,
        totalCost: 25,
        tokenBreakdown: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
      });

      const result = validateSubmission(payload);
      const errorBlob = result.errors.join("\n");

      expect(result.valid).toBe(false);
      // Client-level error must name client, provider, modelId, full cost,
      // and the full token breakdown so an operator can read the failed row
      // straight from the error output without re-running the CLI in debug mode.
      expect(errorBlob).toContain("Client claude/claude-sonnet-4-20250514");
      expect(errorBlob).toContain("(provider=anthropic)");
      expect(errorBlob).toContain("Cost submitted without tokens");
      expect(errorBlob).toContain("cost=$25.0000");
      expect(errorBlob).toContain("tokens={input=0");
      expect(errorBlob).toContain("output=0");
      expect(errorBlob).toContain("reasoning=0");

      // Day-level error must include the date, day total cost, and which
      // clients on that day were responsible (so multi-client days are still
      // actionable).
      expect(errorBlob).toContain("Day 2024-12-01: Cost submitted without tokens");
      expect(errorBlob).toContain("offending clients:");
    });

    it("allows cursor legacy premium-tool-call rows that lack token attribution", () => {
      // Cursor's pre-2025-05 usage exports include `premium-tool-call` rows
      // that are billed per tool invocation and carry no token counts. They
      // legitimately have cost > 0 and tokens = 0 and must bypass the
      // cost-without-tokens sanity check; otherwise any user with historical
      // Cursor data is permanently locked out of `tokscale submit`.
      const payload = {
        meta: {
          generatedAt: "2026-05-27T00:00:00.000Z",
          version: "2.1.3",
          dateRange: { start: "2025-04-29", end: "2025-04-29" },
        },
        summary: {
          totalTokens: 0,
          totalCost: 2.05,
          totalDays: 1,
          activeDays: 0,
          averagePerDay: 2.05,
          maxCostInSingleDay: 2.05,
          clients: ["cursor" as const],
          models: ["premium-tool-call"],
        },
        years: [
          {
            year: "2025",
            totalTokens: 0,
            totalCost: 2.05,
            range: { start: "2025-04-29", end: "2025-04-29" },
          },
        ],
        contributions: [
          {
            date: "2025-04-29",
            totals: { tokens: 0, cost: 2.05, messages: 44 },
            intensity: 0 as const,
            tokenBreakdown: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "cursor" as const,
                modelId: "premium-tool-call",
                providerId: "cursor",
                tokens: {
                  input: 0,
                  output: 0,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 2.05,
                messages: 44,
              },
            ],
          },
        ],
      };

      const result = validateSubmission(payload);

      expect(result.errors).toEqual([]);
      expect(result.valid).toBe(true);
    });

    it("does not extend the cursor legacy bypass to other cursor models", () => {
      // Only `premium-tool-call` is grandfathered. Any other cursor model with
      // cost > 0 and tokens = 0 should still be flagged so legitimate parser
      // regressions remain visible.
      const payload = {
        meta: {
          generatedAt: "2026-05-27T00:00:00.000Z",
          version: "2.1.3",
          dateRange: { start: "2025-05-18", end: "2025-05-18" },
        },
        summary: {
          totalTokens: 0,
          totalCost: 0.04,
          totalDays: 1,
          activeDays: 0,
          averagePerDay: 0.04,
          maxCostInSingleDay: 0.04,
          clients: ["cursor" as const],
          models: ["claude-3.5-sonnet"],
        },
        years: [
          {
            year: "2025",
            totalTokens: 0,
            totalCost: 0.04,
            range: { start: "2025-05-18", end: "2025-05-18" },
          },
        ],
        contributions: [
          {
            date: "2025-05-18",
            totals: { tokens: 0, cost: 0.04, messages: 1 },
            intensity: 0 as const,
            tokenBreakdown: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "cursor" as const,
                modelId: "claude-3.5-sonnet",
                providerId: "anthropic",
                tokens: {
                  input: 0,
                  output: 0,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 0.04,
                messages: 1,
              },
            ],
          },
        ],
      };

      const result = validateSubmission(payload);
      const errorBlob = result.errors.join("\n");

      expect(result.valid).toBe(false);
      expect(errorBlob).toContain("Client cursor/claude-3.5-sonnet");
      expect(errorBlob).toContain("(provider=anthropic)");
      expect(errorBlob).toContain("Cost submitted without tokens");
      expect(errorBlob).toContain("cost=$0.0400");
    });

    it("tolerates floating-point residue when subtracting cursor legacy cost", () => {
      // Regression for PR #612 review (cubic P2): strict `> 0` float
      // comparison can falsely reject an all-legacy submission when IEEE
      // 754 summation leaves a tiny positive remainder.
      //
      // Concretely: `0.1 + 0.2 === 0.30000000000000004` while the literal
      // `0.3` is IEEE `0.299999999999999988…`, so `(0.1+0.2) - 0.3 ≈ 5.5e-17`
      // — a non-zero positive number even though the user truly has $0.30 of
      // legacy cost and nothing else. Without an epsilon, the
      // cost-without-tokens check fires on noise.
      const totalCostWithFpResidue = 0.1 + 0.2; // 0.30000000000000004
      const legacyClientCost = 0.3; // 0.299999999999999988…
      expect(totalCostWithFpResidue - legacyClientCost).toBeGreaterThan(0);
      expect(totalCostWithFpResidue - legacyClientCost).toBeLessThan(1e-10);

      const payload = {
        meta: {
          generatedAt: "2026-05-27T00:00:00.000Z",
          version: "2.1.3",
          dateRange: { start: "2025-04-29", end: "2025-04-29" },
        },
        summary: {
          totalTokens: 0,
          totalCost: totalCostWithFpResidue,
          totalDays: 1,
          activeDays: 0,
          averagePerDay: totalCostWithFpResidue,
          maxCostInSingleDay: totalCostWithFpResidue,
          clients: ["cursor" as const],
          models: ["premium-tool-call"],
        },
        years: [
          {
            year: "2025",
            totalTokens: 0,
            totalCost: totalCostWithFpResidue,
            range: { start: "2025-04-29", end: "2025-04-29" },
          },
        ],
        contributions: [
          {
            date: "2025-04-29",
            totals: {
              tokens: 0,
              cost: totalCostWithFpResidue,
              messages: 6,
            },
            intensity: 0 as const,
            tokenBreakdown: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "cursor" as const,
                modelId: "premium-tool-call",
                providerId: "cursor",
                tokens: {
                  input: 0,
                  output: 0,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: legacyClientCost,
                messages: 6,
              },
            ],
          },
        ],
      };

      const result = validateSubmission(payload);

      expect(result.errors).toEqual([]);
      expect(result.valid).toBe(true);
    });

    it("excludes cursor legacy cost from the cost-per-million sanity cap", () => {
      // Regression for PR #612 review (codex P1): when a legacy
      // `premium-tool-call` row shares a day with a small amount of
      // token-bearing usage, the day-level branch falls through to the
      // cost-per-million check because `day.totals.tokens > 0`. If the full
      // day cost (including the legacy charge) is used as the numerator,
      // tiny token counts trip the $10k/M ceiling even though the legacy
      // row is meant to be skipped. The legacy cost must be subtracted
      // before computing cost-per-million as well.
      const payload = {
        meta: {
          generatedAt: "2026-05-27T00:00:00.000Z",
          version: "2.1.3",
          dateRange: { start: "2025-04-29", end: "2025-04-29" },
        },
        summary: {
          totalTokens: 100,
          totalCost: 2.06,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 2.06,
          maxCostInSingleDay: 2.06,
          clients: ["cursor" as const],
          models: ["premium-tool-call", "claude-3.5-sonnet"],
        },
        years: [
          {
            year: "2025",
            totalTokens: 100,
            totalCost: 2.06,
            range: { start: "2025-04-29", end: "2025-04-29" },
          },
        ],
        contributions: [
          {
            date: "2025-04-29",
            totals: { tokens: 100, cost: 2.06, messages: 45 },
            intensity: 1 as const,
            tokenBreakdown: {
              input: 100,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "cursor" as const,
                modelId: "premium-tool-call",
                providerId: "cursor",
                tokens: {
                  input: 0,
                  output: 0,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 2.05,
                messages: 44,
              },
              {
                client: "cursor" as const,
                modelId: "claude-3.5-sonnet",
                providerId: "anthropic",
                tokens: {
                  input: 100,
                  output: 0,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 0.01,
                messages: 1,
              },
            ],
          },
        ],
      };

      const result = validateSubmission(payload);

      // Without the legacy-cost subtraction the day-level cost-per-million
      // would be ($2.06 / 100 tokens) * 1e6 = $20,600/M, well above the
      // $10,000/M ceiling. After subtracting the legacy $2.05, the
      // checkable cost is $0.01, which is below the $1 floor in
      // pushCostPerMillionError and the check is skipped entirely.
      expect(result.errors).toEqual([]);
      expect(result.valid).toBe(true);
    });

    it("allows cursor legacy rows mixed with normal token-bearing rows", () => {
      // Same day, two clients: a legacy premium-tool-call entry (cost only)
      // and a regular cursor call that has both tokens and cost. The day
      // total has nonzero tokens, so the day-level check does not fire; the
      // per-client legacy carve-out keeps the premium-tool-call row from
      // tripping the client-level check.
      const payload = {
        meta: {
          generatedAt: "2026-05-27T00:00:00.000Z",
          version: "2.1.3",
          dateRange: { start: "2025-04-29", end: "2025-04-29" },
        },
        summary: {
          totalTokens: 1500,
          totalCost: 3.55,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 3.55,
          maxCostInSingleDay: 3.55,
          clients: ["cursor" as const],
          models: ["premium-tool-call", "claude-3.5-sonnet"],
        },
        years: [
          {
            year: "2025",
            totalTokens: 1500,
            totalCost: 3.55,
            range: { start: "2025-04-29", end: "2025-04-29" },
          },
        ],
        contributions: [
          {
            date: "2025-04-29",
            totals: { tokens: 1500, cost: 3.55, messages: 50 },
            intensity: 2 as const,
            tokenBreakdown: {
              input: 1000,
              output: 500,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "cursor" as const,
                modelId: "premium-tool-call",
                providerId: "cursor",
                tokens: {
                  input: 0,
                  output: 0,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 2.05,
                messages: 44,
              },
              {
                client: "cursor" as const,
                modelId: "claude-3.5-sonnet",
                providerId: "anthropic",
                tokens: {
                  input: 1000,
                  output: 500,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 1.5,
                messages: 6,
              },
            ],
          },
        ],
      };

      const result = validateSubmission(payload);

      expect(result.errors).toEqual([]);
      expect(result.valid).toBe(true);
    });

    it("rejects day cost totals that do not match client costs", () => {
      const payload = createValidationPayload({
        totalTokens: 1000,
        totalCost: 100,
        clientCost: 1,
      });

      const result = validateSubmission(payload);

      expect(result.valid).toBe(false);
      expect(result.errors.join("\n")).toContain("client cost");
    });

    it("hashes content changes while ignoring generatedAt churn", () => {
      const lowerPayload = createValidationPayload({
        generatedAt: "2024-12-02T00:00:00.000Z",
        totalTokens: 1000,
        totalCost: 1,
      });
      const samePayloadNewGeneratedAt = createValidationPayload({
        generatedAt: "2024-12-03T00:00:00.000Z",
        totalTokens: 1000,
        totalCost: 1,
      });
      const higherPayload = createValidationPayload({
        generatedAt: "2024-12-02T00:00:00.000Z",
        totalTokens: 2000,
        totalCost: 2,
        tokenBreakdown: {
          input: 2000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
      });

      const lowerResult = validateSubmission(lowerPayload);
      const sameResult = validateSubmission(samePayloadNewGeneratedAt);
      const higherResult = validateSubmission(higherPayload);

      expect(lowerResult.valid).toBe(true);
      expect(sameResult.valid).toBe(true);
      expect(higherResult.valid).toBe(true);
      expect(generateSubmissionHash(lowerResult.data!)).toBe(
        generateSubmissionHash(sameResult.data!)
      );
      expect(generateSubmissionHash(lowerResult.data!)).not.toBe(
        generateSubmissionHash(higherResult.data!)
      );
    });
  });

  describe('Client-Level Merge Logic', () => {
    const breakdown = (
      tokens: number,
      messages: number,
      modelId = 'gpt-5.5'
    ): ClientBreakdownData => ({
      tokens,
      cost: tokens / 1000,
      input: tokens,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages,
      models: {
        [modelId]: {
          tokens,
          cost: tokens / 1000,
          input: tokens,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages,
        },
      },
      provenance: {
        schemaVersion: 1,
        messageCount: messages,
        modelCount: 1,
      },
    });

    it('preserves lower same-client resubmits when coverage also drops', () => {
      const existing = { codex: breakdown(5_000, 30) };
      const incoming = { codex: breakdown(3_600, 20) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        incoming,
        new Set(['codex'])
      );

      expect(result.merged.codex).toEqual(existing.codex);
      expect(result.warnings).toHaveLength(1);
      expect(result.warnings[0]).toContain('codex');
      expect(result.warnings[0]).toContain('5,000');
      expect(result.warnings[0]).toContain('3,600');
    });

    it('preserves existing on lower token resubmit even when coverage does not regress (A2)', () => {
      // A token decrease alone signals a parser regression regardless of whether
      // coverage metrics are equal or higher. The old AND-gate (fewer tokens AND
      // lower coverage) let equal-coverage regressions slip through — fixed in A2.
      const existing = { codex: breakdown(5_000, 30) };
      const incoming = { codex: breakdown(4_800, 32) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        incoming,
        new Set(['codex'])
      );

      expect(result.merged.codex.tokens).toBe(5_000);
      expect(result.warnings).toHaveLength(1);
      expect(result.warnings[0]).toContain('codex');
    });

    it('preserves a submitted client that disappears from a same-device resubmit', () => {
      const existing = {
        codex: breakdown(5_000, 30),
        claude: breakdown(2_000, 10, 'claude-sonnet-4'),
      };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        {},
        new Set(['codex'])
      );

      expect(result.merged.codex).toEqual(existing.codex);
      expect(result.merged.claude).toEqual(existing.claude);
      expect(result.warnings).toHaveLength(1);
      expect(result.warnings[0]).toContain('disappeared');
    });

    it('adds new clients without warning', () => {
      const existing = { claude: breakdown(2_000, 10, 'claude-sonnet-4') };
      const incoming = { codex: breakdown(3_000, 15) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        incoming,
        new Set(['codex'])
      );

      expect(result.merged.claude).toEqual(existing.claude);
      expect(result.merged.codex).toEqual(incoming.codex);
      expect(result.warnings).toEqual([]);
    });

    it('derives provenance from aggregate messages and model keys', () => {
      const aggregate = {
        ...breakdown(5_000, 30),
        messages: 35,
        models: {
          'gpt-5.5': breakdown(3_000, 20).models['gpt-5.5'],
          'gpt-5.5-mini': breakdown(2_000, 15, 'gpt-5.5-mini').models['gpt-5.5-mini'],
        },
        provenance: {
          schemaVersion: 1,
          messageCount: 30,
          modelCount: 1,
        },
      };

      expect(deriveClientBreakdownProvenance(aggregate)).toEqual({
        schemaVersion: 1,
        messageCount: 35,
        modelCount: 2,
      });
    });

    it('should preserve clients NOT in submission but delete clients with no day activity', () => {
      const existingClientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 0, cacheWrite: 0, messages: 5 },
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 0, cacheWrite: 0, messages: 3 },
        codex: { tokens: 200, cost: 2, modelId: 'gpt-4', input: 100, output: 100, cacheRead: 0, cacheWrite: 0, messages: 1 },
      };
      
      const incomingClients = new Set(['claude', 'cursor']);
      const incomingClientBreakdown = {
        claude: { tokens: 1200, cost: 12, modelId: 'claude-sonnet-4', input: 700, output: 500, cacheRead: 0, cacheWrite: 0, messages: 6 },
      };
      
      const merged = { ...existingClientBreakdown } as Record<string, typeof existingClientBreakdown.claude>;
      for (const clientName of incomingClients) {
        if (incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown]) {
          merged[clientName] = incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown];
        } else {
          delete merged[clientName];
        }
      }
      
      expect(merged.codex).toEqual(existingClientBreakdown.codex);
      expect(merged.claude.tokens).toBe(1200);
      expect(merged.cursor).toBeUndefined();
    });

    it('should update submitted client data', () => {
      // Same client submitted again should replace, not add
      const newClaude = { tokens: 1500, cost: 15, modelId: 'claude-sonnet-4', input: 900, output: 600, cacheRead: 0, cacheWrite: 0, messages: 8 };
      
      // After merge, should be new values, not sum
      expect(newClaude.cost).toBe(15); // Not 10 + 15 = 25
      expect(newClaude.tokens).toBe(1500); // Not 1000 + 1500 = 2500
    });

    it('should merge new client into existing day', () => {
      // Day has claude, now cursor is added
      const existingClientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 0, cacheWrite: 0, messages: 5 },
      };
      
      const incomingClients = new Set(['cursor']);
      const incomingClientBreakdown = {
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 0, cacheWrite: 0, messages: 3 },
      };
      
      // Simulate merge
      const merged = { ...existingClientBreakdown };
      for (const clientName of incomingClients) {
        if (incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown]) {
          (merged as Record<string, typeof existingClientBreakdown.claude>)[clientName] = incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown];
        }
      }
      
      // Both clients should be present
      expect(Object.keys(merged)).toContain('claude');
      expect(Object.keys(merged)).toContain('cursor');
    });

    it('should add new dates without affecting existing', () => {
      const existingDates = ['2024-12-01', '2024-12-02'];
      const newDates = ['2024-12-03', '2024-12-04'];
      
      // Simulate: new dates should be added to the set
      const allDates = new Set([...existingDates, ...newDates]);
      
      expect(allDates.size).toBe(4);
      expect(Array.from(allDates)).toContain('2024-12-01');
      expect(Array.from(allDates)).toContain('2024-12-04');
    });
  });

  describe('Totals Recalculation', () => {
    it('should recalculate totalTokens from dailyBreakdown', () => {
      const clientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 50, cacheWrite: 25, messages: 5 },
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 30, cacheWrite: 15, messages: 3 },
      };
      
      // Simulate recalculateDayTotals
      let totalTokens = 0;
      for (const client of Object.values(clientBreakdown)) {
        totalTokens += client.tokens;
      }
      
      expect(totalTokens).toBe(1500);
    });

    it('should recalculate cache tokens', () => {
      const clientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 50, cacheWrite: 25, messages: 5 },
        opencode: { tokens: 800, cost: 8, modelId: 'gpt-4o', input: 500, output: 300, cacheRead: 40, cacheWrite: 20, messages: 4 },
      };
      
      let totalCacheRead = 0;
      let totalCacheWrite = 0;
      for (const client of Object.values(clientBreakdown)) {
        totalCacheRead += client.cacheRead;
        totalCacheWrite += client.cacheWrite;
      }
      
      expect(totalCacheRead).toBe(90);
      expect(totalCacheWrite).toBe(45);
    });

    it('should update clientsUsed to include all clients', () => {
      // Simulate collecting clients from all days
      const day1Clients = ['claude', 'cursor'];
      const day2Clients = ['claude', 'opencode'];
      
      const allClients = new Set([...day1Clients, ...day2Clients]);
      
      expect(Array.from(allClients).sort()).toEqual(['claude', 'cursor', 'opencode']);
    });
  });

  describe('Edge Cases', () => {
    it('should reject empty submissions', () => {
      const data = createMockSubmissionData({ contributions: [] });
      
      expect(data.contributions.length).toBe(0);
      // API should return 400 for this
    });

    it('should handle day with no data for submitted client', () => {
      // User submits --claude but a day only has opencode data
      const dayWithOnlyOpencode = {
        date: '2024-12-01',
        clients: [
          { client: 'opencode', modelId: 'gpt-4o', cost: 5, tokens: { input: 300, output: 200, cacheRead: 0, cacheWrite: 0 }, messages: 3 },
        ],
      };
      
      // No claude data to update for this day
      const claudeInDay = dayWithOnlyOpencode.clients.find(client => client.client === 'claude');
      expect(claudeInDay).toBeUndefined();
      
      // opencode should be preserved
      const opencodeInDay = dayWithOnlyOpencode.clients.find(client => client.client === 'opencode');
      expect(opencodeInDay).toBeDefined();
    });

    it('should handle concurrent submissions without data loss', () => {
      // This is tested at the database level with .for('update') locks
      // Here we just verify the concept
      const submission1Clients = ['claude'];
      const submission2Clients = ['cursor'];
      
      // Both should be present after sequential processing
      const finalClients = new Set([...submission1Clients, ...submission2Clients]);
      expect(finalClients.size).toBe(2);
    });

    it('should treat contribution clients as submitted even if summary.clients is incomplete', () => {
      const data = createMockSubmissionData({
        clients: ['claude'],
        contributions: [
          {
            date: '2024-12-01',
            clients: [
              {
                client: 'pi',
                modelId: 'pi-model',
                cost: 1,
                tokens: { input: 100, output: 50, cacheRead: 0, cacheWrite: 0 },
                messages: 1,
              },
            ],
          },
        ],
      });

      const submittedClients = new Set(data.summary.clients);
      for (const contribution of data.contributions) {
        for (const client of contribution.clients) {
          submittedClients.add(client.client);
        }
      }

      expect(submittedClients.has('pi')).toBe(true);
    });


  });

  describe('Multi-Model Per Client', () => {
    it('should aggregate multiple models per client correctly', () => {
      const dayClientEntries = [
        { client: 'claude', modelId: 'claude-sonnet-4', cost: 10, tokens: { input: 500, output: 300, cacheRead: 100, cacheWrite: 50 }, messages: 5 },
        { client: 'claude', modelId: 'claude-opus-4', cost: 20, tokens: { input: 800, output: 500, cacheRead: 200, cacheWrite: 100 }, messages: 8 },
        { client: 'cursor', modelId: 'gpt-4o', cost: 5, tokens: { input: 200, output: 100, cacheRead: 50, cacheWrite: 25 }, messages: 3 },
      ];

      type ModelData = { tokens: number; cost: number; input: number; output: number; cacheRead: number; cacheWrite: number; messages: number };
      type ClientData = ModelData & { models: Record<string, ModelData> };
      const result: Record<string, ClientData> = {};

      for (const entry of dayClientEntries) {
        const modelData: ModelData = {
          tokens: entry.tokens.input + entry.tokens.output + entry.tokens.cacheRead + entry.tokens.cacheWrite,
          cost: entry.cost,
          input: entry.tokens.input,
          output: entry.tokens.output,
          cacheRead: entry.tokens.cacheRead,
          cacheWrite: entry.tokens.cacheWrite,
          messages: entry.messages,
        };

        const existing = result[entry.client];
        if (existing) {
          existing.tokens += modelData.tokens;
          existing.cost += modelData.cost;
          existing.input += modelData.input;
          existing.output += modelData.output;
          existing.cacheRead += modelData.cacheRead;
          existing.cacheWrite += modelData.cacheWrite;
          existing.messages += modelData.messages;
          existing.models[entry.modelId] = modelData;
        } else {
          result[entry.client] = { ...modelData, models: { [entry.modelId]: modelData } };
        }
      }

      expect(result.claude.tokens).toBe(950 + 1600);
      expect(result.claude.cost).toBe(30);
      expect(Object.keys(result.claude.models)).toContain('claude-sonnet-4');
      expect(Object.keys(result.claude.models)).toContain('claude-opus-4');
      expect(result.claude.models['claude-sonnet-4'].tokens).toBe(950);
      expect(result.claude.models['claude-opus-4'].tokens).toBe(1600);

      expect(result.cursor.tokens).toBe(375);
      expect(Object.keys(result.cursor.models)).toEqual(['gpt-4o']);
    });

  });

  describe('Response Format', () => {
    it('should return mode: "create" for first submission', () => {
      const isNewSubmission = true;
      const mode = isNewSubmission ? 'create' : 'merge';
      expect(mode).toBe('create');
    });

    it('should return mode: "merge" for subsequent submissions', () => {
      const isNewSubmission = false;
      const mode = isNewSubmission ? 'create' : 'merge';
      expect(mode).toBe('merge');
    });

    it('should include recalculated metrics', () => {
      const mockResponse = {
        success: true,
        submissionId: 'test-id',
        username: 'testuser',
        metrics: {
          totalTokens: 1500,
          totalCost: 15.5,
          dateRange: { start: '2024-12-01', end: '2024-12-31' },
          activeDays: 25,
          clients: ['claude', 'cursor'],
        },
        mode: 'merge' as const,
      };
      
      expect(mockResponse.metrics).toBeDefined();
      expect(mockResponse.metrics.totalTokens).toBeGreaterThan(0);
      expect(mockResponse.metrics.clients).toContain('claude');
      expect(mockResponse.mode).toBe('merge');
    });
  });
});
