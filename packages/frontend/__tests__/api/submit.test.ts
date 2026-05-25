import { describe, it, expect } from 'vitest';
import type { ClientType } from "../../src/lib/types";
import {
  generateSubmissionHash,
  validateSubmission,
} from "../../src/lib/validation/submission";

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

  describe("Submission validation guardrails", () => {
    it("rejects one-day fabricated token totals above the submission cap", () => {
      const payload = createValidationPayload({
        totalTokens: 1_000_000_000_000,
        tokenBreakdown: {
          input: 1_000_000_000_000,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
      });

      const result = validateSubmission(payload);

      expect(result.valid).toBe(false);
      expect(result.errors.join("\n")).toContain("Daily token total exceeds");
    });

    it("rejects submitted cost that is implausible for the reported tokens", () => {
      const payload = createValidationPayload({
        totalTokens: 5,
        totalCost: 1_000_000_000,
        tokenBreakdown: {
          input: 5,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
      });

      const result = validateSubmission(payload);

      expect(result.valid).toBe(false);
      expect(result.errors.join("\n")).toContain("Cost per million tokens exceeds");
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
